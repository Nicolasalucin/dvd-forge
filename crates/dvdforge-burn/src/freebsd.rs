//! FreeBSD optical-drive backend built on `camcontrol`, `dvd+rw-mediainfo`,
//! `growisofs` and `xorriso`.
//!
//! Parsers and argument builders are pure so they run in CI on every host; the
//! process calls are only selected by `Platform::FreeBsd`.

use crate::linux::{
    build_growisofs_args_for_media, build_xorriso_args, parse_media_info,
    parse_progress_line_with_speed, validate_burn_guards,
};
use crate::{
    BurnBackend, BurnError, BurnProgress, BurnReport, BurnRequest, DeviceId, DiscBurner, MediaInfo,
    OpticalDevice, ProgressSink, VerifyReport, DVD_SECTOR_BYTES,
};
use async_trait::async_trait;
use dvdforge_toolchain::resolve_binary;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, Command};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct FreeBsdBurner {
    pub camcontrol: PathBuf,
    pub dvd_rw_mediainfo: PathBuf,
    pub growisofs: PathBuf,
    pub xorriso: PathBuf,
}

impl Default for FreeBsdBurner {
    fn default() -> Self {
        Self {
            camcontrol: resolve_binary("camcontrol"),
            dvd_rw_mediainfo: resolve_binary("dvd+rw-mediainfo"),
            growisofs: resolve_binary("growisofs"),
            xorriso: resolve_binary("xorriso"),
        }
    }
}

impl FreeBsdBurner {
    pub fn new() -> Self {
        Self::default()
    }

    async fn ensure_detected_device(&self, requested: &DeviceId) -> Result<(), BurnError> {
        let devices = self.list_devices().await?;
        let requested = crate::canonical_device_path(requested.as_str());
        let detected = devices.iter().map(|device| device.id.as_str().to_string());
        if crate::device_matches_allowlist(&requested, detected, crate::canonical_device_path) {
            Ok(())
        } else {
            Err(BurnError::InvalidRequest(format!(
                "la unidad {requested} no es un dispositivo óptico detectado"
            )))
        }
    }
}

#[async_trait]
impl DiscBurner for FreeBsdBurner {
    fn backend(&self) -> BurnBackend {
        BurnBackend::Growisofs
    }

    async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError> {
        let output = Command::new(&self.camcontrol).arg("devlist").output().await;
        match output {
            Ok(output) if output.status.success() => {
                let devices = parse_camcontrol_devlist(&String::from_utf8_lossy(&output.stdout));
                if devices.is_empty() {
                    Err(BurnError::NoDevices)
                } else {
                    Ok(devices)
                }
            }
            Ok(output) => Err(BurnError::ProcessFailed {
                tool: self.camcontrol.display().to_string(),
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
            Err(error) => {
                let fallback = fallback_devices();
                if fallback.is_empty() {
                    Err(map_command_error(&self.camcontrol, error))
                } else {
                    Ok(fallback)
                }
            }
        }
    }

    async fn probe_media(&self, device: &DeviceId) -> Result<MediaInfo, BurnError> {
        let output = Command::new(&self.dvd_rw_mediainfo)
            .arg(device.as_str())
            .output()
            .await
            .map_err(|error| map_command_error(&self.dvd_rw_mediainfo, error))?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        parse_media_info(&text)
    }

    async fn burn(
        &self,
        request: BurnRequest,
        progress: ProgressSink,
        cancel: CancellationToken,
    ) -> Result<BurnReport, BurnError> {
        if request.simulation {
            return Err(BurnError::InvalidRequest(
                "la simulación de grabación no está soportada por growisofs".to_string(),
            ));
        }
        let iso_size = std::fs::metadata(&request.iso_path).map_err(map_io)?.len();
        self.ensure_detected_device(&request.device).await?;
        let media = self.probe_media(&request.device).await?;
        validate_burn_guards(&request, &media, iso_size)?;
        let started = Instant::now();
        let base_speed = media.kind.bytes_per_second_at_1x();
        let growisofs_args = build_growisofs_args_for_media(&request, media.kind);
        let mut backend = BurnBackend::Growisofs;
        match self
            .run_burn_command(
                &self.growisofs,
                &growisofs_args,
                iso_size,
                base_speed,
                &progress,
                &cancel,
            )
            .await
        {
            Ok(()) => {}
            Err(BurnError::ToolMissing(_)) => {
                let xorriso_args = build_xorriso_args(&request);
                self.run_burn_command(
                    &self.xorriso,
                    &xorriso_args,
                    iso_size,
                    base_speed,
                    &progress,
                    &cancel,
                )
                .await?;
                backend = BurnBackend::Xorriso;
            }
            Err(error) => return Err(error),
        }
        let mut verified = false;
        if request.verify {
            let report = tokio::select! {
                _ = cancel.cancelled() => return Err(BurnError::Cancelled),
                report = self.verify(&request.device, &request.iso_path, progress.clone()) => report?,
            };
            if report.mismatches > 0 {
                return Err(BurnError::VerifyFailed {
                    iso: report.sha256_iso,
                    disc: report.sha256_disc,
                });
            }
            verified = true;
        }
        if request.eject {
            self.eject(&request.device).await?;
        }
        let elapsed = started.elapsed();
        let average_speed_x = if elapsed.as_secs_f64() > 0.0 {
            Some(((iso_size as f64 / elapsed.as_secs_f64()) / base_speed) as f32)
        } else {
            None
        };
        Ok(BurnReport {
            backend,
            elapsed_ms: elapsed.as_millis() as u64,
            average_speed_x,
            verified,
        })
    }

    async fn verify(
        &self,
        device: &DeviceId,
        iso: &Path,
        progress: ProgressSink,
    ) -> Result<VerifyReport, BurnError> {
        let iso_size = std::fs::metadata(iso).map_err(map_io)?.len();
        if iso_size == 0 {
            return Err(BurnError::InvalidRequest("el ISO está vacío".to_string()));
        }
        let sectors_checked = iso_size.div_ceil(DVD_SECTOR_BYTES);
        let iso_hash = tokio::task::spawn_blocking({
            let iso = iso.to_path_buf();
            move || hash_file(&iso)
        });
        let mut file = tokio::fs::File::open(device.as_str())
            .await
            .map_err(map_io)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut remaining_iso = iso_size;
        let mut remaining_read = sectors_checked * DVD_SECTOR_BYTES;
        let mut read_total = 0u64;
        while remaining_read > 0 {
            let chunk = remaining_read.min(buffer.len() as u64) as usize;
            let read = file.read(&mut buffer[..chunk]).await.map_err(map_io)?;
            if read == 0 {
                break;
            }
            if remaining_iso > 0 {
                let take = remaining_iso.min(read as u64) as usize;
                hasher.update(&buffer[..take]);
                remaining_iso -= take as u64;
            }
            remaining_read -= read as u64;
            read_total += read as u64;
            let percent = (read_total.min(iso_size) as f32 / iso_size as f32) * 100.0;
            let _ = progress
                .send(BurnProgress {
                    written_bytes: read_total.min(iso_size),
                    total_bytes: iso_size,
                    percent,
                    speed_x: None,
                    eta_secs: None,
                })
                .await;
        }
        if remaining_read > 0 || remaining_iso > 0 {
            return Err(BurnError::InvalidRequest(
                "la lectura de verificación terminó antes de cubrir el ISO".to_string(),
            ));
        }
        let iso_hash = iso_hash
            .await
            .map_err(|error| BurnError::Io(std::io::Error::other(error)))??;
        let disc_hash = hex::encode(hasher.finalize());
        Ok(VerifyReport {
            sectors_checked,
            mismatches: u64::from(iso_hash != disc_hash),
            sha256_iso: iso_hash,
            sha256_disc: disc_hash,
        })
    }

    async fn eject(&self, device: &DeviceId) -> Result<(), BurnError> {
        let name = device
            .as_str()
            .trim()
            .trim_start_matches("/dev/")
            .to_string();
        let output = Command::new(&self.camcontrol)
            .args(["eject", &name])
            .output()
            .await
            .map_err(|error| map_command_error(&self.camcontrol, error))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BurnError::ProcessFailed {
                tool: self.camcontrol.display().to_string(),
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

impl FreeBsdBurner {
    async fn run_burn_command(
        &self,
        binary: &Path,
        args: &[String],
        total_bytes: u64,
        bytes_per_second_at_1x: f64,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<(), BurnError> {
        let mut child = Command::new(binary)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| map_command_error(binary, error))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BurnError::InvalidRequest("no se pudo capturar stderr".to_string()))?;
        let reader = tokio::spawn(read_progress(
            stderr,
            total_bytes,
            bytes_per_second_at_1x,
            progress.clone(),
        ));
        let status = tokio::select! {
            _ = cancel.cancelled() => {
                terminate_child(&mut child).await;
                reader.abort();
                return Err(BurnError::Cancelled);
            }
            status = child.wait() => status.map_err(map_io)?,
        };
        let stderr_log = reader.await.unwrap_or_default();
        if status.success() {
            let _ = progress
                .send(BurnProgress {
                    written_bytes: total_bytes,
                    total_bytes,
                    percent: 100.0,
                    speed_x: None,
                    eta_secs: Some(0),
                })
                .await;
            Ok(())
        } else {
            Err(BurnError::ProcessFailed {
                tool: binary.display().to_string(),
                code: status.code().unwrap_or(-1),
                stderr: stderr_log,
            })
        }
    }
}

/// Parses `camcontrol devlist` output into optical devices.
pub fn parse_camcontrol_devlist(text: &str) -> Vec<OpticalDevice> {
    let mut devices = Vec::new();
    for line in text.lines() {
        let Some(open) = line.find('<') else { continue };
        let Some(close) = line[open + 1..].find('>').map(|offset| open + 1 + offset) else {
            continue;
        };
        let descriptor = line[open + 1..close].trim();
        let rest = &line[close + 1..];
        let Some(open_paren) = rest.find('(') else {
            continue;
        };
        let Some(close_paren) = rest[open_paren + 1..]
            .find(')')
            .map(|offset| open_paren + 1 + offset)
        else {
            continue;
        };
        let names: Vec<&str> = rest[open_paren + 1..close_paren]
            .split(',')
            .map(str::trim)
            .collect();
        let Some(name) = names
            .iter()
            .find(|name| name.starts_with("cd") || name.starts_with("acd"))
        else {
            continue;
        };
        let mut parts = descriptor.split_whitespace();
        let vendor = parts.next().unwrap_or("").to_string();
        let model = parts.collect::<Vec<_>>().join(" ");
        let id = if name.starts_with('/') {
            name.to_string()
        } else {
            format!("/dev/{name}")
        };
        devices.push(OpticalDevice {
            id: DeviceId::new(id),
            vendor,
            model,
            bus: "cam".to_string(),
            writable: true,
            media_present: false,
        });
    }
    devices.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    devices.dedup_by(|left, right| left.id == right.id);
    devices
}

fn fallback_devices() -> Vec<OpticalDevice> {
    let Ok(entries) = std::fs::read_dir("/dev") else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name
            .strip_prefix("cd")
            .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
            || name
                .strip_prefix("acd")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
        {
            devices.push(OpticalDevice {
                id: DeviceId::new(format!("/dev/{name}")),
                vendor: String::new(),
                model: String::new(),
                bus: "cam".to_string(),
                writable: true,
                media_present: false,
            });
        }
    }
    devices.sort_by(|left, right| left.id.0.cmp(&right.id.0));
    devices
}

async fn read_progress(
    mut stderr: ChildStderr,
    total_bytes: u64,
    bytes_per_second_at_1x: f64,
    progress: ProgressSink,
) -> String {
    let mut buffer = [0u8; 4096];
    let mut pending = String::new();
    let mut log = String::new();
    while let Ok(read) = stderr.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&buffer[..read]);
        log.push_str(&text);
        pending.push_str(&text);
        while let Some(index) = pending.find(['\n', '\r']) {
            let line = pending[..index].to_string();
            pending.drain(..=index);
            if let Some(update) =
                parse_progress_line_with_speed(&line, total_bytes, bytes_per_second_at_1x)
            {
                if progress.send(update).await.is_err() {
                    return log;
                }
            }
        }
    }
    log
}

async fn terminate_child(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: the PID belongs to the child process we spawned.
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    let _ = child.kill().await;
}

fn hash_file(path: &Path) -> Result<String, BurnError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(map_io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(map_io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn map_command_error(binary: &Path, error: std::io::Error) -> BurnError {
    if error.kind() == ErrorKind::NotFound {
        BurnError::ToolMissing(binary.display().to_string())
    } else if error.kind() == ErrorKind::PermissionDenied {
        BurnError::PermissionDenied(error.to_string())
    } else {
        BurnError::Io(error)
    }
}

fn map_io(error: std::io::Error) -> BurnError {
    if error.kind() == ErrorKind::PermissionDenied {
        BurnError::PermissionDenied(error.to_string())
    } else {
        BurnError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DiscMediaKind;

    #[test]
    fn parses_camcontrol_devlist_and_ignores_blocks() {
        let text = concat!(
            "<HL-DT-ST DVDRAM GH24NSC0 L1.00> at scbus1 target 0 lun 0 (cd0,pass0)\n",
            "<Samsung SSD 980 PRO 1B6Q> at scbus0 target 0 lun 0 (ada0,pass1)\n",
            "<MATSHITA BD-MLT UJ272 1.00> at scbus2 target 0 lun 0 (cd1,pass2)\n",
        );
        let devices = parse_camcontrol_devlist(text);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id.as_str(), "/dev/cd0");
        assert_eq!(devices[0].vendor, "HL-DT-ST");
        assert_eq!(devices[0].model, "DVDRAM GH24NSC0 L1.00");
        assert_eq!(devices[1].id.as_str(), "/dev/cd1");
    }

    #[test]
    fn parses_media_info_shared_with_linux() {
        let text = "INQUIRY:\nMounted Media: 1Bh, DVD+R\nDisk status: Complete\n";
        let media = parse_media_info(text).expect("media");
        assert_eq!(media.kind, DiscMediaKind::DvdPlusR);
    }

    #[test]
    fn platform_names_freebsd() {
        assert_eq!(crate::Platform::FreeBsd.name(), "freebsd");
    }
}
