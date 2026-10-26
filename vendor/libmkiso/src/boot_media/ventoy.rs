//! Pinned official Ventoy provisioning on disposable Linux loop devices.
//!
//! The upstream installer receives a loop device, never an arbitrary image path.
//! No physical-device provisioning, downloads, trust enrollment or OS modification
//! are performed. Firmware and installer results remain separate evidence.
#[cfg(target_os = "linux")]
use crate::boot_media::manifest::MediaTarget;
use crate::boot_media::{
    Error, Result,
    manifest::read_multiboot,
    optical::OperationContext,
    plan::{MultibootPlan, fingerprint},
};
use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "linux", test))]
use std::{fs, io::Write};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Audited official release.
pub const PINNED_VERSION: &str = "1.1.17";
/// Required user-supplied official Linux archive filename.
pub const PINNED_ARCHIVE: &str = "ventoy-1.1.17-linux.tar.gz";
/// SHA256 published by the official GitHub release asset API.
pub const PINNED_SHA256: &str = "7fb4ed08cef6a6b4d39dd19260d8c80291a78dfdf9af7d461571e23cbbc43805";
const MAX_IMAGE: u64 = 8 << 40;
const BOOT_BYTES: u64 = 32 << 20;

/// Requested upstream partition scheme.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PartitionTable {
    Mbr,
    Gpt,
}
/// Explicit disk-image build settings.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    pub image_bytes: u64,
    pub partition_table: PartitionTable,
    pub replace: bool,
}
/// Exact runtime provenance. Bundled components retain their individual licenses.
#[derive(Debug, Clone, Serialize)]
pub struct AssetReport {
    pub version: String,
    pub archive: PathBuf,
    pub sha256: String,
    pub license: String,
    pub source: String,
}
/// Byte ranges parsed from the on-disk partition table.
#[derive(Debug, Clone, Serialize)]
pub struct Partition {
    pub offset: u64,
    pub bytes: u64,
}
/// Independently checked partition and filesystem structures.
#[derive(Debug, Clone, Serialize)]
pub struct DiskLayout {
    pub partition_table: PartitionTable,
    pub image_bytes: u64,
    pub data: Partition,
    pub boot: Partition,
}
/// Bounded durable provenance for the successful official installer invocation.
#[derive(Debug, Clone, Serialize)]
pub struct InstallerReport {
    pub program: String,
    pub arguments: Vec<String>,
    pub stdout_prefix: String,
    pub stdout_sha256: String,
    pub stdout_bytes: usize,
}
/// Structural/content verification; it does not assert firmware bootability.
#[derive(Debug, Clone, Serialize)]
pub struct VentoyReport {
    pub image: PathBuf,
    pub sha256: String,
    pub backend: AssetReport,
    pub layout: DiskLayout,
    pub iso_sha256: Vec<(String, String)>,
    pub secure_boot: String,
    pub firmware_startup: String,
    pub installer_startup: String,
    pub installer: Option<InstallerReport>,
}

/// Check the complete official archive against the pinned upstream digest.
pub fn audit_assets(assets: &Path) -> Result<AssetReport> {
    let archive = assets.join(PINNED_ARCHIVE);
    let input = fingerprint(&archive, 64 << 20)?;
    if input.sha256 != PINNED_SHA256 {
        return Err(Error::Verification(format!(
            "Ventoy archive SHA256 differs from audited {PINNED_VERSION} release"
        )));
    }
    Ok(AssetReport { version: PINNED_VERSION.into(), archive: input.path, sha256: input.sha256, license: "GPL-3.0-or-later; bundled components retain upstream licenses; source notices: https://github.com/ventoy/Ventoy/tree/v1.1.17/License".into(), source: format!("https://github.com/ventoy/Ventoy/releases/tag/v{PINNED_VERSION}") })
}
/// Conservative capacity reservation for EFI, alignment, exFAT metadata and files.
pub fn minimum_image_bytes(payload_bytes: u64, entry_count: usize) -> Result<u64> {
    let count =
        u64::try_from(entry_count).map_err(|_| Error::Resource("entry count overflow".into()))?;
    // exFAT allocation clusters are at most 128 KiB in this upstream route.
    let needed = payload_bytes
        .checked_add(payload_bytes / 100)
        .and_then(|n| n.checked_add(128 << 20))
        .and_then(|n| {
            count
                .checked_mul(128 << 10)
                .and_then(|slack| n.checked_add(slack))
        })
        .ok_or_else(|| Error::Resource("disk capacity overflow".into()))?;
    let aligned = needed
        .checked_add((1 << 20) - 1)
        .map(|n| n & !((1 << 20) - 1))
        .ok_or_else(|| Error::Resource("disk alignment overflow".into()))?;
    Ok(aligned.max(256 << 20))
}
#[cfg(target_os = "linux")]
fn checkpoint(ctx: &OperationContext<'_>) -> Result<()> {
    if ctx.cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn recheck(plan: &MultibootPlan, ctx: &mut OperationContext<'_>) -> Result<()> {
    if plan.target != MediaTarget::Usb || plan.backend != "ventoy" || plan.entries.is_empty() {
        return Err(Error::Unsupported(
            "requires a nonempty Ventoy USB plan".into(),
        ));
    }
    let fresh =
        crate::boot_media::plan::plan_multiboot_with_context(&plan.manifest, plan.target, ctx)?;
    // Replanning also rejects forged destination paths in serialized plans.
    if serde_json::to_value(&fresh).map_err(|e| Error::InvalidInput(e.to_string()))?
        != serde_json::to_value(plan).map_err(|e| Error::InvalidInput(e.to_string()))?
    {
        return Err(Error::Verification(
            "multiboot plan changed; replan original manifest".into(),
        ));
    }
    Ok(())
}
fn le32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn le64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320_u32 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
fn read_at(file: &mut File, offset: u64, size: usize) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn partition(first: u64, sectors: u64, image_bytes: u64) -> Result<Partition> {
    let offset = first
        .checked_mul(512)
        .ok_or_else(|| Error::Verification("partition offset overflow".into()))?;
    let bytes = sectors
        .checked_mul(512)
        .ok_or_else(|| Error::Verification("partition length overflow".into()))?;
    if bytes == 0
        || offset
            .checked_add(bytes)
            .is_none_or(|end| end > image_bytes)
    {
        return Err(Error::Verification("partition exceeds disk image".into()));
    }
    Ok(Partition { offset, bytes })
}
fn gpt_header(file: &mut File, lba: u64, sectors: u64) -> Result<Vec<u8>> {
    let mut bytes = read_at(
        file,
        lba.checked_mul(512)
            .ok_or_else(|| Error::Verification("GPT header overflow".into()))?,
        512,
    )?;
    let size = le32(&bytes[12..16]) as usize;
    if &bytes[..8] != b"EFI PART"
        || !(92..=512).contains(&size)
        || le64(&bytes[24..32]) != lba
        || le64(&bytes[32..40]) >= sectors
    {
        return Err(Error::Verification("invalid GPT header".into()));
    }
    let expected = le32(&bytes[16..20]);
    bytes[16..20].fill(0);
    if crc32(&bytes[..size]) != expected {
        return Err(Error::Verification("GPT header CRC mismatch".into()));
    }
    Ok(bytes)
}
/// Validate MBR/GPT bounds and CRCs, required exFAT and FAT partition signatures.
pub fn inspect_layout(path: &Path) -> Result<DiskLayout> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let image_bytes = meta.len();
    if !meta.is_file()
        || !(256 << 20..=MAX_IMAGE).contains(&image_bytes)
        || !image_bytes.is_multiple_of(512)
    {
        return Err(Error::InvalidInput(
            "requires a bounded, sector-aligned regular disk image".into(),
        ));
    }
    let mbr = read_at(&mut file, 0, 512)?;
    if mbr[510..512] != [0x55, 0xaa] {
        return Err(Error::Verification("missing MBR signature".into()));
    }
    let (partition_table, data, boot) = if mbr[450] == 0xee {
        let sectors = image_bytes / 512;
        let head = gpt_header(&mut file, 1, sectors)?;
        let backup = gpt_header(&mut file, sectors - 1, sectors)?;
        if le64(&head[32..40]) != sectors - 1
            || le64(&backup[32..40]) != 1
            || head[56..72] != backup[56..72]
        {
            return Err(Error::Verification("GPT headers disagree".into()));
        }
        let count = le32(&head[80..84]) as usize;
        let size = le32(&head[84..88]) as usize;
        if count != 128 || size != 128 {
            return Err(Error::Verification("unexpected Ventoy GPT entries".into()));
        }
        let table_offset = le64(&head[72..80])
            .checked_mul(512)
            .ok_or_else(|| Error::Verification("GPT table overflow".into()))?;
        let entries = read_at(&mut file, table_offset, count * size)?;
        let back_entries = read_at(
            &mut file,
            le64(&backup[72..80])
                .checked_mul(512)
                .ok_or_else(|| Error::Verification("GPT backup overflow".into()))?,
            count * size,
        )?;
        if crc32(&entries) != le32(&head[88..92])
            || crc32(&back_entries) != le32(&backup[88..92])
            || entries != back_entries
        {
            return Err(Error::Verification("GPT table CRC/content mismatch".into()));
        }
        if entries[256..].iter().any(|b| *b != 0) {
            return Err(Error::Verification(
                "unexpected additional partitions".into(),
            ));
        }
        let parse = |entry: &[u8]| -> Result<Partition> {
            if entry[..16].iter().all(|b| *b == 0) {
                return Err(Error::Verification("missing GPT partition".into()));
            }
            let first = le64(&entry[32..40]);
            let last = le64(&entry[40..48]);
            let length = last
                .checked_sub(first)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| Error::Verification("invalid GPT partition range".into()))?;
            if first < le64(&head[40..48]) || last > le64(&head[48..56]) {
                return Err(Error::Verification(
                    "GPT partition outside usable range".into(),
                ));
            }
            partition(first, length, image_bytes)
        };
        (
            PartitionTable::Gpt,
            parse(&entries[..128])?,
            parse(&entries[128..256])?,
        )
    } else {
        if mbr[450] != 0x07 || mbr[466] != 0xef || mbr[478..510].iter().any(|b| *b != 0) {
            return Err(Error::Verification(
                "unexpected Ventoy MBR partition types/count".into(),
            ));
        }
        (
            PartitionTable::Mbr,
            partition(
                u64::from(le32(&mbr[454..458])),
                u64::from(le32(&mbr[458..462])),
                image_bytes,
            )?,
            partition(
                u64::from(le32(&mbr[470..474])),
                u64::from(le32(&mbr[474..478])),
                image_bytes,
            )?,
        )
    };
    if data.offset != 1 << 20 || data.offset + data.bytes > boot.offset || boot.bytes != BOOT_BYTES
    {
        return Err(Error::Verification(
            "invalid Ventoy partition order/alignment/EFI size".into(),
        ));
    }
    let exfat = read_at(&mut file, data.offset, 512)?;
    let fat = read_at(&mut file, boot.offset, 512)?;
    if &exfat[3..11] != b"EXFAT   "
        || exfat[510..512] != [0x55, 0xaa]
        || fat[510..512] != [0x55, 0xaa]
        || (&fat[54..62] != b"FAT16   " && &fat[82..90] != b"FAT32   ")
    {
        return Err(Error::Verification(
            "missing exFAT/FAT filesystem signatures".into(),
        ));
    }
    Ok(DiskLayout {
        partition_table,
        image_bytes,
        data,
        boot,
    })
}

/// Generate documented plugins; explicitly disable upstream Windows policy bypasses.
pub fn menu_config(plan: &MultibootPlan) -> Result<serde_json::Value> {
    let manifest = read_multiboot(&plan.manifest)?;
    let aliases: Vec<_> = plan
        .entries
        .iter()
        .map(|e| serde_json::json!({"image":e.destination,"alias":e.title}))
        .collect();
    let mut control = vec![
        serde_json::json!({"VTOY_DEFAULT_SEARCH_ROOT":"/isos"}),
        serde_json::json!({"VTOY_WIN11_BYPASS_CHECK":"0"}),
        serde_json::json!({"VTOY_WIN11_BYPASS_NRO":"0"}),
    ];
    // Ventoy treats an explicit zero as immediate boot; omission waits for input.
    if manifest.menu.timeout_seconds > 0 {
        control.push(
            serde_json::json!({"VTOY_MENU_TIMEOUT":manifest.menu.timeout_seconds.to_string()}),
        );
    }
    if let Some(id) = manifest.menu.default {
        let entry = plan
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::InvalidInput("default entry missing".into()))?;
        control.push(serde_json::json!({"VTOY_DEFAULT_IMAGE":entry.destination}));
    }
    Ok(serde_json::json!({"control":control,"menu_alias":aliases}))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::boot_media::{
        plan::{destination_path, fingerprint_with_context, same_file},
        progress::{Phase, ProgressUnit},
    };
    use std::{
        os::unix::process::CommandExt,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    const TOOL_TIMEOUT: Duration = Duration::from_secs(300);
    const LOG_LIMIT: u64 = 8 << 20;
    /// All commands receive separate argument arrays. Entire subprocess groups are
    /// killed on cancellation/time/output limits so dd/mount cannot outlive them.
    fn run(
        program: &str,
        args: &[&std::ffi::OsStr],
        cwd: Option<&Path>,
        stdin: Option<&[u8]>,
        ctx: Option<&OperationContext<'_>>,
    ) -> Result<String> {
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(stdout.try_clone()?)
            .stderr(stderr.try_clone()?);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        // SAFETY: setpgid is async-signal-safe and only affects the forked child.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|e| Error::Unsupported(format!("cannot run {program}: {e}")))?;
        if let Some(data) = stdin {
            let result = child
                .stdin
                .take()
                .ok_or_else(|| Error::Io(std::io::Error::other("missing subprocess stdin")))?
                .write_all(data);
            if let Err(error) = result {
                stop(&mut child);
                return Err(error.into());
            }
        }
        let started = Instant::now();
        let status = loop {
            let abort = if ctx.is_some_and(|c| c.cancellation.is_cancelled()) {
                Some(Error::Cancelled)
            } else if started.elapsed() > TOOL_TIMEOUT {
                Some(Error::Resource(format!(
                    "{program} exceeded 300 second deadline"
                )))
            } else if stdout
                .metadata()?
                .len()
                .saturating_add(stderr.metadata()?.len())
                > LOG_LIMIT
            {
                Some(Error::Resource(format!("{program} output exceeds 8 MiB")))
            } else {
                None
            };
            if let Some(error) = abort {
                stop(&mut child);
                return Err(error);
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let mut output = String::new();
        let mut reader = stdout;
        reader.seek(SeekFrom::Start(0))?;
        reader.take(LOG_LIMIT + 1).read_to_string(&mut output)?;
        if !status.success() {
            let mut diagnostic = String::new();
            let mut reader = stderr;
            reader.seek(SeekFrom::Start(0))?;
            reader.take(4096).read_to_string(&mut diagnostic)?;
            return Err(Error::Io(std::io::Error::other(format!(
                "{program} exited {status}: {} {diagnostic}",
                output.chars().take(4096).collect::<String>()
            ))));
        }
        Ok(output)
    }
    fn stop(child: &mut std::process::Child) {
        // SAFETY: child owns this PID/process group; no pointers are passed.
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    pub(super) fn available() -> Result<()> {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return Err(Error::Unsupported("Ventoy Linux loop provisioning requires root/CAP_SYS_ADMIN; run in a disposable Linux guest with loop devices and exFAT support".into()));
        }
        if !Path::new("/dev/loop-control").exists() {
            return Err(Error::Unsupported(
                "Linux loop-control is unavailable".into(),
            ));
        }
        use std::os::unix::fs::PermissionsExt;
        let search = std::env::var_os("PATH").unwrap_or_default();
        let missing: Vec<_> = [
            "parted",
            "mkfs.vfat",
            "losetup",
            "mount",
            "umount",
            "tar",
            "sh",
            "dd",
            "sync",
        ]
        .into_iter()
        .filter(|name| {
            !std::env::split_paths(&search).any(|directory| {
                fs::metadata(directory.join(name)).is_ok_and(|metadata| {
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                })
            })
        })
        .collect();
        if !missing.is_empty() {
            return Err(Error::Unsupported(format!(
                "Ventoy provisioning tools unavailable: {}; install util-linux, parted, dosfstools and coreutils in the disposable Linux environment",
                missing.join(", ")
            )));
        }
        Ok(())
    }
    struct LoopDevice {
        path: PathBuf,
        mount: Option<PathBuf>,
    }
    impl LoopDevice {
        fn attach(image: &Path, readonly: bool, ctx: &OperationContext<'_>) -> Result<Self> {
            let mut args: Vec<&std::ffi::OsStr> =
                vec!["--find".as_ref(), "--show".as_ref(), "--partscan".as_ref()];
            if readonly {
                args.push("--read-only".as_ref());
            }
            args.push("--".as_ref());
            args.push(image.as_os_str());
            let path = run("losetup", &args, None, None, None)?.trim().to_owned();
            let suffix = path.strip_prefix("/dev/loop");
            if !suffix.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) {
                return Err(Error::Verification(
                    "losetup returned unexpected device".into(),
                ));
            }
            let device = Self {
                path: PathBuf::from(path),
                mount: None,
            };
            let backing = run(
                "losetup",
                &[
                    "--list".as_ref(),
                    "--noheadings".as_ref(),
                    "--output".as_ref(),
                    "BACK-FILE".as_ref(),
                    device.path.as_os_str(),
                ],
                None,
                None,
                Some(ctx),
            )?;
            if fs::canonicalize(backing.trim())? != fs::canonicalize(image)? {
                return Err(Error::Verification(
                    "loop device backing-file identity mismatch".into(),
                ));
            }
            Ok(device)
        }
        fn mount(&mut self, readonly: bool, ctx: &OperationContext<'_>) -> Result<PathBuf> {
            let partition = PathBuf::from(format!("{}p1", self.path.display()));
            let started = Instant::now();
            while !partition.exists() {
                checkpoint(ctx)?;
                if started.elapsed() > Duration::from_secs(10) {
                    return Err(Error::Unsupported(
                        "loop data partition did not appear".into(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            let path = tempfile::tempdir()?.keep();
            self.mount = Some(path.clone());
            let opts = if readonly {
                "ro,nodev,nosuid,noexec"
            } else {
                "rw,nodev,nosuid,noexec"
            };
            run(
                "mount",
                &[
                    "-t".as_ref(),
                    "exfat".as_ref(),
                    "-o".as_ref(),
                    opts.as_ref(),
                    "--".as_ref(),
                    partition.as_os_str(),
                    path.as_os_str(),
                ],
                None,
                None,
                Some(ctx),
            )?;
            Ok(path)
        }
        fn close(&mut self) -> Result<()> {
            if let Some(mount) = &self.mount {
                run(
                    "umount",
                    &["--".as_ref(), mount.as_os_str()],
                    None,
                    None,
                    None,
                )?;
                fs::remove_dir(mount)?;
                self.mount = None;
            }
            if !self.path.as_os_str().is_empty() {
                run(
                    "losetup",
                    &["--detach".as_ref(), self.path.as_os_str()],
                    None,
                    None,
                    None,
                )?;
                self.path = PathBuf::new();
            }
            Ok(())
        }
    }
    impl Drop for LoopDevice {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }
    fn verify_mounted(
        plan: &MultibootPlan,
        mount: &Path,
        ctx: &mut OperationContext<'_>,
    ) -> Result<Vec<(String, String)>> {
        let file = File::open(mount.join("ventoy/ventoy.json"))?;
        let mut config = Vec::new();
        file.take((1 << 20) + 1).read_to_end(&mut config)?;
        if config.len() > 1 << 20 {
            return Err(Error::Resource("menu configuration exceeds 1 MiB".into()));
        }
        let config: serde_json::Value = serde_json::from_slice(&config)
            .map_err(|e| Error::Verification(format!("invalid menu JSON: {e}")))?;
        if config != menu_config(plan)? {
            return Err(Error::Verification(
                "Ventoy menu configuration differs from manifest".into(),
            ));
        }
        let mut hashes = Vec::new();
        for entry in &plan.entries {
            checkpoint(ctx)?;
            let input = fingerprint_with_context(
                &mount.join(entry.destination.trim_start_matches('/')),
                MAX_IMAGE,
                ctx,
            )?;
            if input.size != entry.input.size || input.sha256 != entry.input.sha256 {
                return Err(Error::Verification(format!(
                    "entry {} contents differ from original ISO",
                    entry.id
                )));
            }
            hashes.push((entry.id.clone(), input.sha256));
        }
        let mut count = 0;
        for entry in fs::read_dir(mount.join("isos"))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Err(Error::Verification("non-regular ISO payload".into()));
            }
            count += 1;
            if count > plan.entries.len() {
                return Err(Error::Verification("unexpected ISO payload count".into()));
            }
        }
        if count != plan.entries.len() {
            return Err(Error::Verification("unexpected ISO payload count".into()));
        }
        Ok(hashes)
    }
    fn stage_bundle(
        assets: &AssetReport,
        ctx: &OperationContext<'_>,
    ) -> Result<(tempfile::TempDir, PathBuf)> {
        let staging = tempfile::tempdir()?;
        let staged_archive = staging.path().join(PINNED_ARCHIVE);
        let source = File::open(&assets.archive)?;
        let mut staged = File::create(&staged_archive)?;
        let copied = std::io::copy(&mut source.take((64 << 20) + 1), &mut staged)?;
        if copied > 64 << 20 {
            return Err(Error::Resource("Ventoy archive exceeds 64 MiB".into()));
        }
        staged.sync_all()?;
        if fingerprint(&staged_archive, 64 << 20)?.sha256 != PINNED_SHA256 {
            return Err(Error::Verification(
                "Ventoy archive changed before extraction".into(),
            ));
        }
        run(
            "tar",
            &[
                "--extract".as_ref(),
                "--gzip".as_ref(),
                "--file".as_ref(),
                staged_archive.as_os_str(),
                "--directory".as_ref(),
                staging.path().as_os_str(),
                "--no-same-owner".as_ref(),
            ],
            None,
            None,
            Some(ctx),
        )?;
        let bundle = staging.path().join(format!("ventoy-{PINNED_VERSION}"));
        if fs::read_to_string(bundle.join("ventoy/version"))?.trim() != PINNED_VERSION {
            return Err(Error::Verification("Ventoy bundle version mismatch".into()));
        }
        Ok((staging, bundle))
    }
    fn verify_runtime(
        bundle: &Path,
        image: &Path,
        layout: &DiskLayout,
        device: &LoopDevice,
        ctx: &OperationContext<'_>,
    ) -> Result<String> {
        let source = fs::read(bundle.join("boot/boot.img"))?;
        if source.len() != 512 {
            return Err(Error::Verification(
                "invalid pinned BIOS boot sector".into(),
            ));
        }
        let mut image_file = File::open(image)?;
        let actual = read_at(&mut image_file, 0, 512)?;
        for index in 0..440 {
            // Upstream randomizes the disk UUID and patches the GPT core start.
            if (384..400).contains(&index) {
                continue;
            }
            let expected = if index == 92 && layout.partition_table == PartitionTable::Gpt {
                0x22
            } else {
                source[index]
            };
            if actual[index] != expected {
                return Err(Error::Verification(
                    "BIOS runtime bytes differ from pinned release".into(),
                ));
            }
        }
        let info = run(
            "sh",
            &[
                "Ventoy2Disk.sh".as_ref(),
                "-l".as_ref(),
                device.path.as_os_str(),
            ],
            Some(bundle),
            None,
            Some(ctx),
        )?;
        if !info.contains(&format!("Ventoy Version in Disk: {PINNED_VERSION}")) {
            return Err(Error::Verification(
                "on-disk EFI runtime version differs from pinned release".into(),
            ));
        }
        let secure = info
            .lines()
            .find(|line| line.contains("Secure Boot Support"))
            .ok_or_else(|| {
                Error::Verification("upstream runtime did not report Secure Boot support".into())
            })?;
        Ok(if secure.ends_with("NO") {
            "disabled"
        } else if secure.ends_with("YES") {
            "runtime support present; firmware trust unmeasured"
        } else {
            return Err(Error::Verification(
                "invalid upstream Secure Boot support result".into(),
            ));
        }
        .into())
    }
    pub(super) fn build(
        plan: &MultibootPlan,
        output: &Path,
        options: &BuildOptions,
        ctx: &mut OperationContext<'_>,
    ) -> Result<VentoyReport> {
        checkpoint(ctx)?;
        available()?;
        recheck(plan, ctx)?;
        let assets = audit_assets(&plan.assets)?;
        let required = minimum_image_bytes(plan.payload_bytes, plan.entries.len())?;
        if options.image_bytes < required
            || options.image_bytes > MAX_IMAGE
            || !options.image_bytes.is_multiple_of(1 << 20)
        {
            return Err(Error::InvalidInput(format!(
                "image size must be MiB-aligned and within {required}..{MAX_IMAGE} bytes"
            )));
        }
        if options.partition_table == PartitionTable::Mbr && options.image_bytes > 2 << 40 {
            return Err(Error::Unsupported(
                "MBR images over 2 TiB require GPT".into(),
            ));
        }
        let output = destination_path(output)?;
        for input in plan
            .entries
            .iter()
            .map(|e| &e.input.path)
            .chain(plan.asset_inputs.iter().map(|a| &a.path))
            .chain(std::iter::once(&plan.manifest))
        {
            if same_file(input, &output)? {
                return Err(Error::InvalidInput(
                    "output aliases a manifest, asset or source ISO".into(),
                ));
            }
        }
        if output.exists()
            && (!options.replace || !fs::symlink_metadata(&output)?.file_type().is_file())
        {
            return Err(Error::InvalidInput(
                "existing output requires --replace and a regular file".into(),
            ));
        }
        let parent = output
            .parent()
            .ok_or_else(|| Error::InvalidInput("output parent unavailable".into()))?;
        let temporary = tempfile::Builder::new()
            .prefix(".mkiso-ventoy-")
            .suffix(".img")
            .tempfile_in(parent)?;
        let (temporary_file, temporary_path) = temporary.keep().map_err(|e| Error::Io(e.error))?;
        let result = (|| -> Result<VentoyReport> {
            temporary_file.set_len(options.image_bytes)?;
            let (_staging, bundle) = stage_bundle(&assets, ctx)?;
            let mut device = LoopDevice::attach(&temporary_path, false, ctx)?;
            let mut args: Vec<&std::ffi::OsStr> =
                vec!["Ventoy2Disk.sh".as_ref(), "-i".as_ref(), "-S".as_ref()];
            if options.partition_table == PartitionTable::Gpt {
                args.push("-g".as_ref());
            }
            args.push(device.path.as_os_str());
            let installer_output = run("sh", &args, Some(&bundle), Some(b"y\ny\n"), Some(ctx))?;
            use sha2::Digest;
            let installer = InstallerReport {
                program: "sh".into(),
                arguments: args
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
                stdout_prefix: installer_output.chars().take(16 << 10).collect(),
                stdout_sha256: hex::encode(sha2::Sha256::digest(installer_output.as_bytes())),
                stdout_bytes: installer_output.len(),
            };
            let layout = inspect_layout(&temporary_path)?;
            if layout.partition_table != options.partition_table {
                return Err(Error::Verification(
                    "installer produced wrong partition scheme".into(),
                ));
            }
            let secure_boot = verify_runtime(&bundle, &temporary_path, &layout, &device, ctx)?;
            if secure_boot != "disabled" {
                return Err(Error::Verification(
                    "installer did not disable Secure Boot runtime support as requested".into(),
                ));
            }
            let mount = device.mount(false, ctx)?;
            fs::create_dir(mount.join("isos"))?;
            fs::create_dir(mount.join("ventoy"))?;
            let config = serde_json::to_vec_pretty(&menu_config(plan)?)
                .map_err(|e| Error::InvalidInput(e.to_string()))?;
            let mut config_file = File::create(mount.join("ventoy/ventoy.json"))?;
            config_file.write_all(&config)?;
            config_file.sync_all()?;
            // An open file on the data filesystem prevents its later unmount.
            drop(config_file);
            let mut progress = crate::boot_media::progress::PhaseProgress::start(
                ctx.observer,
                ctx.operation_id,
                Phase::EmitImage,
                Some(plan.payload_bytes),
                ProgressUnit::Bytes,
                None,
            );
            let copied = (|| -> Result<()> {
                for entry in &plan.entries {
                    let mut source = File::open(&entry.input.path)?;
                    let mut dest =
                        File::create(mount.join(entry.destination.trim_start_matches('/')))?;
                    let mut buffer = vec![0; 1 << 20];
                    let mut copied = 0_u64;
                    loop {
                        if ctx.cancellation.is_cancelled() {
                            return Err(Error::Cancelled);
                        }
                        let n = source.read(&mut buffer)?;
                        if n == 0 {
                            break;
                        }
                        copied = copied
                            .checked_add(n as u64)
                            .ok_or_else(|| Error::Resource("ISO copy overflow".into()))?;
                        if copied > entry.input.size {
                            return Err(Error::Verification("source ISO grew".into()));
                        }
                        dest.write_all(&buffer[..n])?;
                        progress
                            .advance(n as u64)
                            .map_err(|e| Error::Resource(e.to_string()))?;
                    }
                    if copied != entry.input.size {
                        return Err(Error::Verification("source ISO shrank".into()));
                    }
                    dest.sync_all()?;
                }
                Ok(())
            })();
            match copied {
                Ok(()) => {
                    progress
                        .finish()
                        .map_err(|e| Error::Resource(e.to_string()))?;
                }
                Err(error) => {
                    if matches!(error, Error::Cancelled) {
                        let _ = progress.cancel();
                    } else {
                        let _ = progress.fail();
                    }
                    return Err(error);
                }
            }
            drop(progress);
            let iso_sha256 = verify_mounted(plan, &mount, ctx)?;
            device.close()?;
            temporary_file.sync_all()?;
            recheck(plan, ctx)?;
            checkpoint(ctx)?;
            let hash = fingerprint_with_context(&temporary_path, MAX_IMAGE, ctx)?.sha256;
            if options.replace {
                fs::rename(&temporary_path, &output)?;
            } else {
                fs::hard_link(&temporary_path, &output)?;
                fs::remove_file(&temporary_path)?;
            }
            #[cfg(unix)]
            File::open(parent)?.sync_all()?;
            Ok(VentoyReport {
                image: output.clone(),
                sha256: hash,
                backend: assets,
                layout,
                iso_sha256,
                secure_boot,
                firmware_startup: "untested".into(),
                installer_startup: "untested".into(),
                installer: Some(installer),
            })
        })();
        result.map_err(|source| Error::Recovery {
            path: if temporary_path.exists() {
                temporary_path
            } else {
                output
            },
            source: Box::new(source),
        })
    }
    pub(super) fn verify(
        plan: &MultibootPlan,
        image: &Path,
        ctx: &mut OperationContext<'_>,
    ) -> Result<VentoyReport> {
        checkpoint(ctx)?;
        available()?;
        recheck(plan, ctx)?;
        let backend = audit_assets(&plan.assets)?;
        let layout = inspect_layout(image)?;
        let original = fingerprint_with_context(image, MAX_IMAGE, ctx)?;
        let (_staging, bundle) = stage_bundle(&backend, ctx)?;
        let mut device = LoopDevice::attach(image, true, ctx)?;
        let secure_boot = verify_runtime(&bundle, image, &layout, &device, ctx)?;
        let mount = device.mount(true, ctx)?;
        let iso_sha256 = verify_mounted(plan, &mount, ctx)?;
        device.close()?;
        if fingerprint_with_context(image, MAX_IMAGE, ctx)? != original {
            return Err(Error::Verification(
                "disk image changed during verification".into(),
            ));
        }
        Ok(VentoyReport {
            image: fs::canonicalize(image)?,
            sha256: original.sha256,
            backend,
            layout,
            iso_sha256,
            secure_boot,
            firmware_startup: "untested".into(),
            installer_startup: "untested".into(),
            installer: None,
        })
    }
    #[cfg(test)]
    mod subprocess_tests {
        use super::*;
        use crate::boot_media::progress::{CancellationToken, NoProgress};
        #[test]
        fn cancelling_running_tool_kills_it_and_returns_before_late_side_effect() {
            let directory = tempfile::tempdir().unwrap();
            let script = directory.path().join("slow-tool.sh");
            let started_file = directory.path().join("started");
            let late_file = directory.path().join("late-write");
            fs::write(
                &script,
                "printf started > \"$1\"\nsleep 30\nprintf late > \"$2\"\n",
            )
            .unwrap();
            let cancellation = CancellationToken::default();
            let cancel = cancellation.clone();
            let started_marker = started_file.clone();
            let canceller = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !started_marker.exists() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                cancel.cancel();
            });
            let mut observer = NoProgress;
            let ctx = OperationContext {
                observer: &mut observer,
                cancellation: &cancellation,
                operation_id: 1,
            };
            let start = Instant::now();
            let result = run(
                "sh",
                &[
                    script.as_os_str(),
                    started_file.as_os_str(),
                    late_file.as_os_str(),
                ],
                None,
                None,
                Some(&ctx),
            );
            canceller.join().unwrap();
            assert!(
                matches!(result, Err(Error::Cancelled)),
                "unexpected result: {result:?}"
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            assert!(started_file.exists());
            assert!(!late_file.exists());
        }
        #[test]
        fn subprocess_arguments_preserve_spaces_quotes_and_shell_metacharacters() {
            let directory = tempfile::tempdir().unwrap();
            let script = directory.path().join("argument probe.sh");
            fs::write(&script, "printf '%s' \"$1\"\n").unwrap();
            let argument = "spaces ' quotes $(printf injected) ; &";
            assert_eq!(
                run(
                    "sh",
                    &[script.as_os_str(), argument.as_ref()],
                    None,
                    None,
                    None
                )
                .unwrap(),
                argument
            );
        }
    }
}
/// Check provisioning privileges, loop support and required executable tools
/// before hashing ISOs or creating an output artifact.
pub fn provisioning_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::available()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::Unsupported(
            "Ventoy regular-image provisioning requires Linux loop devices".into(),
        ))
    }
}
/// Build a regular disk image through an owned Linux loop device and the pinned installer.
pub fn build(
    plan: &MultibootPlan,
    output: &Path,
    options: &BuildOptions,
    ctx: &mut OperationContext<'_>,
) -> Result<VentoyReport> {
    #[cfg(target_os = "linux")]
    {
        linux::build(plan, output, options, ctx)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (plan, output, options, ctx);
        Err(Error::Unsupported(
            "Ventoy regular-image provisioning requires Linux loop devices".into(),
        ))
    }
}
/// Verify raw disk structures and exact ISO hashes using a read-only loop mount.
pub fn verify(
    plan: &MultibootPlan,
    image: &Path,
    ctx: &mut OperationContext<'_>,
) -> Result<VentoyReport> {
    #[cfg(target_os = "linux")]
    {
        linux::verify(plan, image, ctx)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (plan, image, ctx);
        Err(Error::Unsupported(
            "Ventoy content verification requires Linux read-only loop devices".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mbr_fixture() -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(256 << 20).unwrap();
        let mut mbr = [0_u8; 512];
        mbr[510..512].copy_from_slice(&[0x55, 0xaa]);
        mbr[450] = 0x07;
        mbr[466] = 0xef;
        let data_first = 2048_u32;
        let boot_first = ((256 - 32) << 11) as u32;
        mbr[454..458].copy_from_slice(&data_first.to_le_bytes());
        mbr[458..462].copy_from_slice(&(boot_first - data_first).to_le_bytes());
        mbr[470..474].copy_from_slice(&boot_first.to_le_bytes());
        mbr[474..478].copy_from_slice(&(32_u32 << 11).to_le_bytes());
        file.write_all(&mbr).unwrap();
        let mut exfat = [0_u8; 512];
        exfat[3..11].copy_from_slice(b"EXFAT   ");
        exfat[510..512].copy_from_slice(&[0x55, 0xaa]);
        file.seek(SeekFrom::Start(1 << 20)).unwrap();
        file.write_all(&exfat).unwrap();
        let mut fat = [0_u8; 512];
        fat[54..62].copy_from_slice(b"FAT16   ");
        fat[510..512].copy_from_slice(&[0x55, 0xaa]);
        file.seek(SeekFrom::Start(u64::from(boot_first) * 512))
            .unwrap();
        file.write_all(&fat).unwrap();
        file
    }
    #[test]
    fn recovery_error_preserves_machine_category_and_identifies_artifact() {
        let error = Error::Recovery {
            path: PathBuf::from("/tmp/.mkiso-ventoy-partial.img"),
            source: Box::new(Error::Cancelled),
        };
        assert_eq!(
            (error.code(), error.exit_code(), error.to_string()),
            (
                "cancelled",
                130,
                "operation cancelled; recovery artifact: /tmp/.mkiso-ventoy-partial.img".into()
            )
        );
    }
    #[test]
    fn checks_valid_mbr_disk_partition_bounds_and_filesystem_signatures() {
        let file = mbr_fixture();
        let layout = inspect_layout(file.path()).unwrap();
        assert_eq!(
            (
                layout.partition_table,
                layout.data.offset,
                layout.boot.bytes
            ),
            (PartitionTable::Mbr, 1 << 20, 32 << 20)
        );
    }
    #[test]
    fn rejects_partition_overlapping_boot_runtime() {
        let mut file = mbr_fixture();
        file.seek(SeekFrom::Start(458)).unwrap();
        file.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(matches!(
            inspect_layout(file.path()),
            Err(Error::Verification(_))
        ));
    }
    #[test]
    fn rejects_missing_exfat_signature() {
        let mut file = mbr_fixture();
        file.seek(SeekFrom::Start((1 << 20) + 3)).unwrap();
        file.write_all(b"CORRUPT!").unwrap();
        assert!(matches!(
            inspect_layout(file.path()),
            Err(Error::Verification(_))
        ));
    }
    #[test]
    fn rejects_unpinned_runtime_before_executing_any_installer() {
        let assets = tempfile::tempdir().unwrap();
        fs::write(assets.path().join(PINNED_ARCHIVE), b"#!/bin/sh\nexit 0\n").unwrap();
        assert!(matches!(
            audit_assets(assets.path()),
            Err(Error::Verification(_))
        ));
    }
    #[test]
    fn capacity_includes_boot_and_per_iso_cluster_rounding() {
        assert!(
            minimum_image_bytes(4 << 30, 4096).unwrap()
                > (4 << 30) + (32 << 20) + ((4096 * 128) << 10)
        );
    }
    #[test]
    fn capacity_rejects_integer_overflow() {
        assert!(matches!(
            minimum_image_bytes(u64::MAX, 1),
            Err(Error::Resource(_))
        ));
    }
    #[test]
    fn crc32_matches_standard_check_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }
    #[test]
    fn generated_menu_preserves_entry_aliases_and_disables_windows_bypasses() {
        let directory = tempfile::tempdir().unwrap();
        let manifest = directory.path().join("multiboot.toml");
        fs::write(
            &manifest,
            r#"version = 1
[menu]
title = "Installers"
default = "windows"
timeout_seconds = 12
[boot]
backend = "ventoy"
firmware = ["bios", "uefi"]
assets = "assets"
[[entries]]
id = "windows"
title = 'Windows "Original"'
image = "windows.iso"
"#,
        )
        .unwrap();
        let iso = directory.path().join("windows.iso");
        fs::write(&iso, b"synthetic fingerprint for menu-only test").unwrap();
        let plan = MultibootPlan {
            schema_version: 1,
            manifest,
            target: crate::boot_media::manifest::MediaTarget::Usb,
            backend: "ventoy".into(),
            assets: directory.path().join("assets"),
            asset_inputs: Vec::new(),
            entries: vec![crate::boot_media::plan::PlannedEntry {
                id: "windows".into(),
                title: "Windows \"Original\"".into(),
                input: fingerprint(&iso, 1024).unwrap(),
                destination: "/isos/windows.iso".into(),
                status: "untested".into(),
            }],
            payload_bytes: 0,
            image_bytes: None,
            unsupported: Vec::new(),
        };
        let config = menu_config(&plan).unwrap();
        assert_eq!(
            config,
            serde_json::json!({
                "control": [
                    {"VTOY_DEFAULT_SEARCH_ROOT": "/isos"},
                    {"VTOY_WIN11_BYPASS_CHECK": "0"},
                    {"VTOY_WIN11_BYPASS_NRO": "0"},
                    {"VTOY_MENU_TIMEOUT": "12"},
                    {"VTOY_DEFAULT_IMAGE": "/isos/windows.iso"}
                ],
                "menu_alias": [{"image": "/isos/windows.iso", "alias": "Windows \"Original\""}]
            })
        );
        let contents = fs::read_to_string(&plan.manifest).unwrap();
        fs::write(
            &plan.manifest,
            contents.replace("timeout_seconds = 12", "timeout_seconds = 0"),
        )
        .unwrap();
        let config = menu_config(&plan).unwrap();
        assert!(
            config["control"]
                .as_array()
                .unwrap()
                .iter()
                .all(|item| item.get("VTOY_MENU_TIMEOUT").is_none()),
            "zero timeout must wait for user input instead of booting immediately"
        );
    }
    #[test]
    fn checks_both_gpt_headers_and_partition_table_copies() {
        let mut file = mbr_fixture();
        file.seek(SeekFrom::Start(450)).unwrap();
        file.write_all(&[0xee]).unwrap();
        let sectors = (256_u64 << 20) / 512;
        let boot_first = (224_u64 << 20) / 512;
        let mut table = vec![0_u8; 128 * 128];
        for (offset, first, last) in [
            (0, 2048_u64, boot_first - 1),
            (128, boot_first, sectors - 34),
        ] {
            table[offset] = 1;
            table[offset + 32..offset + 40].copy_from_slice(&first.to_le_bytes());
            table[offset + 40..offset + 48].copy_from_slice(&last.to_le_bytes());
        }
        // Preserve exactly 32 MiB boot partition while reserving backup GPT sectors.
        table[128 + 32..128 + 40].copy_from_slice(&(boot_first - 33).to_le_bytes());
        table[40..48].copy_from_slice(&(boot_first - 34).to_le_bytes());
        let mut header = [0_u8; 512];
        header[..8].copy_from_slice(b"EFI PART");
        header[12..16].copy_from_slice(&92_u32.to_le_bytes());
        header[40..48].copy_from_slice(&34_u64.to_le_bytes());
        header[48..56].copy_from_slice(&(sectors - 34).to_le_bytes());
        header[80..84].copy_from_slice(&128_u32.to_le_bytes());
        header[84..88].copy_from_slice(&128_u32.to_le_bytes());
        header[88..92].copy_from_slice(&crc32(&table).to_le_bytes());
        for (lba, alternate, table_lba) in [(1, sectors - 1, 2), (sectors - 1, 1, sectors - 33)] {
            header[24..32].copy_from_slice(&lba.to_le_bytes());
            header[32..40].copy_from_slice(&alternate.to_le_bytes());
            header[72..80].copy_from_slice(&table_lba.to_le_bytes());
            header[16..20].fill(0);
            let crc = crc32(&header[..92]);
            header[16..20].copy_from_slice(&crc.to_le_bytes());
            file.seek(SeekFrom::Start(lba * 512)).unwrap();
            file.write_all(&header).unwrap();
            file.seek(SeekFrom::Start(table_lba * 512)).unwrap();
            file.write_all(&table).unwrap();
        }
        let mut fat = [0_u8; 512];
        fat[54..62].copy_from_slice(b"FAT16   ");
        fat[510..512].copy_from_slice(&[0x55, 0xaa]);
        file.seek(SeekFrom::Start((boot_first - 33) * 512)).unwrap();
        file.write_all(&fat).unwrap();
        assert_eq!(
            inspect_layout(file.path()).unwrap().partition_table,
            PartitionTable::Gpt
        );
    }
    #[test]
    fn rejects_bad_gpt_crc_before_reading_arbitrary_table_offsets() {
        let mut file = mbr_fixture();
        file.seek(SeekFrom::Start(450)).unwrap();
        file.write_all(&[0xee]).unwrap();
        let mut header = [0_u8; 512];
        header[..8].copy_from_slice(b"EFI PART");
        header[12..16].copy_from_slice(&92_u32.to_le_bytes());
        header[24..32].copy_from_slice(&1_u64.to_le_bytes());
        header[32..40].copy_from_slice(&((256_u64 << 11) - 1).to_le_bytes());
        file.seek(SeekFrom::Start(512)).unwrap();
        file.write_all(&header).unwrap();
        assert!(
            matches!(inspect_layout(file.path()),Err(Error::Verification(message)) if message.contains("CRC"))
        );
    }
}
