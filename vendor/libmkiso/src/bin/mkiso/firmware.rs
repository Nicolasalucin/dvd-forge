//! Explicit firmware observation without promoting screenshots to installer gates.
#[cfg(unix)]
use libmkiso::boot_media::progress::{Phase, PhaseProgress, ProgressUnit};
use libmkiso::boot_media::{Error, Result, optical::OperationContext};
use serde_json::Value;
#[cfg(any(unix, test))]
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Settings {
    pub firmware: String,
    pub evidence_dir: PathBuf,
    pub entry: String,
    pub backend: String,
    pub bios: Option<PathBuf>,
    pub ovmf_code: Option<PathBuf>,
    pub ovmf_vars: Option<PathBuf>,
    pub swtpm: Option<PathBuf>,
    pub keys: Vec<String>,
    pub timeout: u64,
    pub select_after: u64,
    pub memory: u32,
    pub accel: String,
}

pub fn run(image: &Path, settings: &Settings, ctx: &mut OperationContext<'_>) -> Result<Value> {
    ctx.cancellation
        .checkpoint()
        .map_err(|_| Error::Cancelled)?;
    validate(image, settings)?;
    #[cfg(not(unix))]
    return Err(Error::Unsupported(
        "firmware observation currently requires a Unix host".into(),
    ));
    #[cfg(unix)]
    {
        fs::create_dir(&settings.evidence_dir)?;
        let evidence = settings.evidence_dir.canonicalize()?;
        let script = evidence.join("test-firmware.py");
        fs::write(
            &script,
            include_str!("../../../scripts/mkiso/test-firmware.py"),
        )?;
        let log = fs::File::create(evidence.join("harness.log"))?;
        let mut command = Command::new("python3");
        command
            .arg(&script)
            .arg(image.canonicalize()?)
            .arg("--output")
            .arg(&evidence)
            .arg("--entry")
            .arg(&settings.entry)
            .arg("--backend")
            .arg(&settings.backend)
            .arg("--firmware")
            .arg(&settings.firmware)
            .arg("--timeout")
            .arg(settings.timeout.to_string())
            .arg("--select-after")
            .arg(settings.select_after.to_string())
            .arg("--memory")
            .arg(settings.memory.to_string())
            .arg("--accel")
            .arg(&settings.accel)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        for (flag, path) in [
            ("--bios", &settings.bios),
            ("--ovmf-code", &settings.ovmf_code),
            ("--ovmf-vars", &settings.ovmf_vars),
            ("--swtpm", &settings.swtpm),
        ] {
            if let Some(path) = path {
                command.arg(flag).arg(path.canonicalize()?);
            }
        }
        for key in &settings.keys {
            command.arg("--key").arg(key);
        }
        let mut phase = PhaseProgress::start(
            ctx.observer,
            ctx.operation_id,
            Phase::BootTest,
            Some(1),
            ProgressUnit::Operations,
            Some(settings.entry.clone()),
        );
        let result = (|| {
            let mut child = command.spawn()?;
            let deadline = Instant::now()
                + Duration::from_secs(
                    settings.timeout * settings.firmware.split(',').count() as u64 + 300,
                );
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {}
                    Err(error) => {
                        stop(&mut child);
                        return Err(Error::Io(error));
                    }
                }
                if fs::metadata(evidence.join("harness.log"))
                    .is_ok_and(|metadata| metadata.len() > 8 * 1024 * 1024)
                {
                    stop(&mut child);
                    return Err(Error::Resource(
                        "firmware observer stderr exceeded 8 MiB".into(),
                    ));
                }
                if ctx.cancellation.is_cancelled() {
                    stop(&mut child);
                    return Err(Error::Cancelled);
                }
                if Instant::now() >= deadline {
                    stop(&mut child);
                    return Err(Error::Resource(
                        "firmware observation exceeded its time budget; evidence retained".into(),
                    ));
                }
                thread::sleep(Duration::from_millis(100));
            };
            if !status.success() {
                return Err(Error::Verification(format!(
                    "firmware observer exited {status}; inspect {}",
                    evidence.join("harness.log").display()
                )));
            }
            let file = fs::File::open(evidence.join("report.json"))?;
            let mut bytes = Vec::new();
            file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 4 * 1024 * 1024 {
                return Err(Error::Resource("firmware report exceeds 4 MiB".into()));
            }
            serde_json::from_slice(&bytes)
                .map_err(|error| Error::Verification(format!("invalid firmware report: {error}")))
        })();
        match &result {
            Ok(_) => {
                phase
                    .advance(1)
                    .map_err(|e| Error::Verification(e.to_string()))?;
                phase
                    .finish()
                    .map_err(|e| Error::Verification(e.to_string()))?;
            }
            Err(Error::Cancelled) => {
                let _ = phase.cancel();
            }
            Err(_) => {
                let _ = phase.fail();
            }
        }
        result
    }
}

fn validate(image: &Path, settings: &Settings) -> Result<()> {
    if !matches!(settings.firmware.as_str(), "bios" | "uefi" | "bios,uefi")
        || !matches!(settings.accel.as_str(), "tcg" | "kvm")
        || !(1..=3600).contains(&settings.timeout)
        || settings.select_after >= settings.timeout
        || !(128..=65536).contains(&settings.memory)
        || settings.keys.len() > 100
        || settings.entry.is_empty()
        || settings.backend.is_empty()
    {
        return Err(Error::Usage("invalid firmware observer settings".into()));
    }
    if settings.evidence_dir.symlink_metadata().is_ok() {
        return Err(Error::InvalidInput(
            "firmware evidence must use a new directory".into(),
        ));
    }
    let parent = settings
        .evidence_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = parent.canonicalize()?.join(
        settings
            .evidence_dir
            .file_name()
            .ok_or_else(|| Error::Usage("evidence directory needs a file name".into()))?,
    );
    for path in std::iter::once(image).chain(
        [
            settings.bios.as_deref(),
            settings.ovmf_code.as_deref(),
            settings.ovmf_vars.as_deref(),
            settings.swtpm.as_deref(),
        ]
        .into_iter()
        .flatten(),
    ) {
        let source = path.canonicalize()?;
        if !source.is_file() || source.starts_with(&destination) || source == destination {
            return Err(Error::InvalidInput(
                "evidence directory conflicts with an input or input is not a regular file".into(),
            ));
        }
        if source.to_string_lossy().contains(',') {
            return Err(Error::InvalidInput(
                "QEMU input paths cannot contain commas".into(),
            ));
        }
    }
    if destination.to_string_lossy().contains(',') {
        return Err(Error::InvalidInput(
            "QEMU evidence paths cannot contain commas".into(),
        ));
    }
    if settings.firmware.contains("bios") && settings.bios.is_none() {
        return Err(Error::Usage("BIOS observation requires --bios".into()));
    }
    if settings.firmware.contains("uefi")
        && (settings.ovmf_code.is_none() || settings.ovmf_vars.is_none())
    {
        return Err(Error::Usage(
            "UEFI observation requires --ovmf-code and --ovmf-vars".into(),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn stop(child: &mut Child) {
    // SIGTERM lets Python stop its private QEMU process group and save evidence.
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(child.id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings(directory: &Path) -> Settings {
        Settings {
            firmware: "bios".into(),
            evidence_dir: directory.join("evidence"),
            entry: "debian".into(),
            backend: "ventoy-1.1.17".into(),
            bios: Some(directory.join("bios.fd")),
            ovmf_code: None,
            ovmf_vars: None,
            swtpm: None,
            keys: vec![],
            timeout: 10,
            select_after: 1,
            memory: 2048,
            accel: "tcg".into(),
        }
    }
    #[test]
    fn refuses_existing_evidence_directory_before_running_qemu() {
        let dir = tempfile::tempdir().unwrap();
        let settings = settings(dir.path());
        fs::create_dir(&settings.evidence_dir).unwrap();
        assert!(matches!(
            validate(&dir.path().join("image"), &settings),
            Err(Error::InvalidInput(_))
        ));
    }
    #[test]
    fn refuses_unbounded_observation_before_reading_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings(dir.path());
        settings.timeout = u64::MAX;
        assert!(matches!(
            validate(&dir.path().join("image"), &settings),
            Err(Error::Usage(_))
        ));
    }
    #[test]
    fn validates_new_evidence_location_and_explicit_firmware_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let settings = settings(dir.path());
        let image = dir.path().join("image");
        fs::write(&image, b"media").unwrap();
        fs::write(settings.bios.as_ref().unwrap(), b"firmware").unwrap();
        assert!(validate(&image, &settings).is_ok());
    }
}
