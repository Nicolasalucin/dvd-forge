use crate::{
    BurnBackend, BurnError, BurnProgress, BurnReport, BurnRequest, DeviceId, DiscBurner,
    DiscMediaKind, MediaInfo, MediaStatus, OpticalDevice, ProgressSink, VerifyReport,
};
use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStderr, Command};
use tokio_util::sync::CancellationToken;

use crate::{DVD_1X_BYTES_PER_SECOND, DVD_SECTOR_BYTES};

#[derive(Debug, Clone)]
pub struct LinuxBurner {
    pub lsblk: PathBuf,
    pub dvd_rw_mediainfo: PathBuf,
    pub growisofs: PathBuf,
    pub xorriso: PathBuf,
    pub eject: PathBuf,
}

impl Default for LinuxBurner {
    fn default() -> Self {
        Self {
            lsblk: PathBuf::from("lsblk"),
            dvd_rw_mediainfo: PathBuf::from("dvd+rw-mediainfo"),
            growisofs: PathBuf::from("growisofs"),
            xorriso: PathBuf::from("xorriso"),
            eject: PathBuf::from("eject"),
        }
    }
}

impl LinuxBurner {
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
impl DiscBurner for LinuxBurner {
    fn backend(&self) -> BurnBackend {
        BurnBackend::Growisofs
    }

    async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError> {
        let output = command_output(
            &self.lsblk,
            &[
                "--json".to_string(),
                "-o".to_string(),
                "NAME,KNAME,TYPE,TRAN,VENDOR,MODEL,RM,SIZE,MOUNTPOINTS".to_string(),
            ],
        )
        .await?;
        if !output.status.success() {
            return Err(BurnError::ProcessFailed {
                tool: self.lsblk.display().to_string(),
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        parse_lsblk(&String::from_utf8_lossy(&output.stdout))
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
            let report = self
                .verify(&request.device, &request.iso_path, progress.clone())
                .await?;
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
        let iso_hash = hash_file(iso).await?;

        let mut file = tokio::fs::File::open(device.as_str())
            .await
            .map_err(map_io)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
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

        let disc_hash = hex::encode(hasher.finalize());
        Ok(VerifyReport {
            sectors_checked,
            mismatches: u64::from(iso_hash != disc_hash),
            sha256_iso: iso_hash,
            sha256_disc: disc_hash,
        })
    }

    async fn eject(&self, device: &DeviceId) -> Result<(), BurnError> {
        let output = Command::new(&self.eject)
            .arg(device.as_str())
            .output()
            .await
            .map_err(|error| map_command_error(&self.eject, error))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(BurnError::ProcessFailed {
                tool: self.eject.display().to_string(),
                code: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            })
        }
    }
}

async fn command_output(binary: &Path, args: &[String]) -> Result<std::process::Output, BurnError> {
    Command::new(binary)
        .args(args)
        .output()
        .await
        .map_err(|error| map_command_error(binary, error))
}

fn map_command_error(binary: &Path, error: std::io::Error) -> BurnError {
    if error.kind() == ErrorKind::NotFound {
        BurnError::ToolMissing(binary.display().to_string())
    } else {
        map_io(error)
    }
}

fn map_io(error: std::io::Error) -> BurnError {
    if error.kind() == ErrorKind::PermissionDenied {
        BurnError::PermissionDenied(error.to_string())
    } else {
        BurnError::Io(error)
    }
}

#[derive(Debug, Deserialize)]
struct LsblkOutput {
    #[serde(default)]
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Debug, Deserialize)]
struct LsblkDevice {
    #[serde(default)]
    name: String,
    #[serde(default)]
    kname: Option<String>,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    tran: Option<String>,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

pub fn parse_lsblk(json: &str) -> Result<Vec<OpticalDevice>, BurnError> {
    let output: LsblkOutput = serde_json::from_str(json)?;
    let mut devices = Vec::new();
    for device in output.blockdevices {
        if device.kind != "rom" {
            continue;
        }
        let name = device
            .kname
            .filter(|value| !value.is_empty())
            .unwrap_or(device.name);
        if name.is_empty() {
            continue;
        }
        let id = if name.starts_with("/dev/") {
            name
        } else {
            format!("/dev/{name}")
        };
        let model = device.model.unwrap_or_default().trim().to_string();
        let vendor = device.vendor.unwrap_or_default().trim().to_string();
        devices.push(OpticalDevice {
            id: DeviceId::new(id),
            vendor,
            model,
            bus: device.tran.unwrap_or_else(|| "unknown".to_string()),
            writable: true,
            media_present: false,
        });
    }
    devices.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    Ok(devices)
}

pub fn parse_media_info(text: &str) -> Result<MediaInfo, BurnError> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("no media mounted")
        || lower.contains("no medium found")
        || lower.contains("no media")
        || lower.contains("unable to open")
    {
        return Err(BurnError::NoMedia);
    }

    let media_line = line_containing(text, "mounted media:").ok_or(BurnError::NoMedia)?;
    let kind = parse_media_kind(media_line);
    if kind == DiscMediaKind::Unknown {
        return Err(BurnError::UnsupportedMedia(media_line.to_string()));
    }
    let status = parse_status(text);
    // Multi-track BD-R/BD-RE reports one READ TRACK INFORMATION block per
    // track. Track sizes are disjoint, so their sum is the media capacity;
    // summing free blocks gives the remaining writable space.
    let free_blocks = sum_values_after(text, "free blocks:");
    let track_blocks = sum_values_after(text, "track size:");
    let read_capacity = value_after(text, "read capacity:")
        .and_then(parse_number_prefix)
        .unwrap_or(0);
    let last_lba = value_after(text, "last logical block address:")
        .or_else(|| value_after(text, "last recorded address:"))
        .and_then(parse_number_prefix);
    let free_bytes = free_blocks.saturating_mul(DVD_SECTOR_BYTES);
    let capacity_blocks = if track_blocks > 0 {
        track_blocks
    } else if read_capacity > 0 {
        read_capacity
    } else if free_blocks > 0 {
        free_blocks
    } else {
        last_lba.map(|lba| lba + 1).unwrap_or(0)
    };
    let capacity_bytes = capacity_blocks.saturating_mul(DVD_SECTOR_BYTES);
    let max_speed_x = parse_max_speed(text);

    Ok(MediaInfo {
        kind,
        status,
        capacity_bytes,
        free_bytes,
        writable: kind.writable(),
        rewritable: kind.rewritable(),
        max_speed_x,
    })
}

fn line_containing<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains(needle))
        .map(str::trim)
}

fn value_after<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    line_containing(text, needle)
        .and_then(|line| line.split_once(':'))
        .map(|(_, value)| value.trim())
}

fn sum_values_after(text: &str, needle: &str) -> u64 {
    text.lines()
        .filter(|line| line.to_ascii_lowercase().contains(needle))
        .filter_map(|line| line.split_once(':'))
        .filter_map(|(_, value)| parse_number_prefix(value))
        .fold(0u64, u64::saturating_add)
}

fn parse_media_profile(line: &str) -> Option<u8> {
    let value = line.split_once(':').map(|(_, value)| value).unwrap_or(line);
    let token = value.split(',').next()?.trim();
    let token = token.strip_suffix('h')?;
    u8::from_str_radix(token, 16).ok()
}

fn parse_media_kind(line: &str) -> DiscMediaKind {
    let lower = line.to_ascii_lowercase();
    // dvd+rw-mediainfo prints the MMC profile as "41h, BD-R SRM" etc.
    match parse_media_profile(line) {
        Some(0x40) => return DiscMediaKind::BdRom,
        Some(0x41 | 0x42) => return DiscMediaKind::BdR,
        Some(0x43) => return DiscMediaKind::BdRe,
        _ => {}
    }
    if lower.contains("bd-re") {
        DiscMediaKind::BdRe
    } else if lower.contains("bd-r") {
        DiscMediaKind::BdR
    } else if lower.contains("bd-rom") || lower.contains("blu-ray rom") {
        DiscMediaKind::BdRom
    } else if lower.contains("dvd+rw") {
        DiscMediaKind::DvdPlusRw
    } else if lower.contains("dvd-rw") {
        DiscMediaKind::DvdRw
    } else if lower.contains("dvd+r") {
        DiscMediaKind::DvdPlusR
    } else if lower.contains("dvd-ram") {
        DiscMediaKind::DvdRam
    } else if lower.contains("dvd-rom") {
        DiscMediaKind::DvdRom
    } else if lower.contains("dvd-r") {
        DiscMediaKind::DvdR
    } else {
        DiscMediaKind::Unknown
    }
}

fn parse_status(text: &str) -> MediaStatus {
    let Some(value) = value_after(text, "disc status:") else {
        return MediaStatus::Unknown;
    };
    let value = value.to_ascii_lowercase();
    if value.contains("blank") {
        MediaStatus::Blank
    } else if value.contains("appendable") {
        MediaStatus::Appendable
    } else if value.contains("complete") {
        MediaStatus::Complete
    } else if value.contains("closed") {
        MediaStatus::Closed
    } else {
        MediaStatus::Unknown
    }
}

fn parse_number_prefix(value: &str) -> Option<u64> {
    let digits: String = value
        .trim()
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn parse_max_speed(text: &str) -> Option<f32> {
    text.lines()
        .filter(|line| line.to_ascii_lowercase().contains("write speed"))
        .filter_map(parse_speed_from_line)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

fn parse_speed_from_line(line: &str) -> Option<f32> {
    for token in line.split_whitespace() {
        let token = token.trim_matches(|character: char| {
            character == '(' || character == ')' || character == ',' || character == ';'
        });
        if let Some(index) = token.find('x') {
            let prefix = &token[..index];
            if let Ok(value) = prefix.parse::<f32>() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_progress_line(line: &str, total_bytes: u64) -> Option<BurnProgress> {
    parse_progress_line_with_speed(line, total_bytes, DVD_1X_BYTES_PER_SECOND)
}

/// Parses growisofs progress using the media family's 1x speed (DVD 1385 KB/s,
/// Blu-ray 4495 KB/s). growisofs prints the same `N*2KiB @ X.Xx` format for
/// both families, but x-factors are not comparable across them.
pub fn parse_progress_line_with_speed(
    line: &str,
    total_bytes: u64,
    bytes_per_second_at_1x: f64,
) -> Option<BurnProgress> {
    let marker = "builtin_dd:";
    let index = line.find(marker)?;
    let rest = line[index + marker.len()..].trim();
    let token = rest.split_whitespace().next()?;
    let count = token.split('*').next()?.parse::<u64>().ok()?;
    let written_bytes = count.saturating_mul(DVD_SECTOR_BYTES);
    let percent = if total_bytes == 0 {
        0.0
    } else {
        ((written_bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0) as f32
    };
    let mut speed_x = None;
    let mut bytes_per_second = None;
    for token in line.split_whitespace() {
        let token = token.trim_matches(|character: char| {
            character == '(' || character == ')' || character == ',' || character == ';'
        });
        if let Some(index) = token.find('x') {
            if let Ok(value) = token[..index].parse::<f32>() {
                speed_x = Some(value);
                bytes_per_second = Some(value as f64 * bytes_per_second_at_1x);
            }
        }
        let lower = token.to_ascii_lowercase();
        if let Some(value) = lower.strip_suffix("kb/s") {
            if let Ok(value) = value.parse::<f64>() {
                bytes_per_second = Some(value * 1_000.0);
            }
        }
    }
    let eta_secs = bytes_per_second.and_then(|speed| {
        if speed > 0.0 && written_bytes < total_bytes {
            Some(((total_bytes - written_bytes) as f64 / speed) as u64)
        } else {
            None
        }
    });
    Some(BurnProgress {
        written_bytes,
        total_bytes,
        percent,
        speed_x,
        eta_secs,
    })
}

pub fn build_growisofs_args(request: &BurnRequest) -> Vec<String> {
    build_growisofs_args_for_media(request, DiscMediaKind::DvdR)
}

/// `-dvd-compat` closes DVD media but is not a Blu-ray option. growisofs
/// finalizes BD-R/BD-RE with `-Z` alone.
pub fn build_growisofs_args_for_media(
    request: &BurnRequest,
    media_kind: DiscMediaKind,
) -> Vec<String> {
    let mut args = vec![
        "-Z".to_string(),
        format!("{}={}", request.device, request.iso_path.display()),
    ];
    if request.close_session && !media_kind.is_blu_ray() {
        args.push("-dvd-compat".to_string());
    }
    if let Some(speed) = request.speed_x {
        args.push(format!("-speed={speed}"));
    }
    args
}

pub fn build_xorriso_args(request: &BurnRequest) -> Vec<String> {
    let mut args = vec![
        "-as".to_string(),
        "cdrecord".to_string(),
        "-v".to_string(),
        format!("dev={}", request.device),
    ];
    if let Some(speed) = request.speed_x {
        args.push(format!("-speed={speed}"));
    }
    args.push(request.iso_path.display().to_string());
    args
}

pub fn validate_burn_guards(
    request: &BurnRequest,
    media: &MediaInfo,
    iso_size: u64,
) -> Result<(), BurnError> {
    if !request.device.as_str().starts_with("/dev/") {
        return Err(BurnError::InvalidRequest(
            "el dispositivo debe ser una ruta /dev/...".to_string(),
        ));
    }
    if iso_size == 0 {
        return Err(BurnError::InvalidRequest("el ISO está vacío".to_string()));
    }
    if !media.writable || media.kind.read_only() {
        return Err(BurnError::UnsupportedMedia(
            "el medio no es grabable".to_string(),
        ));
    }
    if media.capacity_bytes == 0 {
        return Err(BurnError::InvalidRequest(
            "no se pudo determinar la capacidad del medio".to_string(),
        ));
    }
    if iso_size > media.capacity_bytes {
        return Err(BurnError::InvalidRequest(format!(
            "el ISO ({} bytes) supera la capacidad del medio ({} bytes)",
            iso_size, media.capacity_bytes
        )));
    }
    if media.free_bytes > 0 && iso_size > media.free_bytes {
        return Err(BurnError::InvalidRequest(format!(
            "el ISO ({} bytes) supera el espacio libre del medio ({} bytes)",
            iso_size, media.free_bytes
        )));
    }
    if let Some(speed) = request.speed_x {
        if speed == 0 {
            return Err(BurnError::InvalidRequest(
                "la velocidad debe ser mayor que cero".to_string(),
            ));
        }
    }
    Ok(())
}

impl LinuxBurner {
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

async fn hash_file(path: &Path) -> Result<String, BurnError> {
    let mut file = tokio::fs::File::open(path).await.map_err(map_io)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await.map_err(map_io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BurnRequest, BD_1X_BYTES_PER_SECOND};

    const LSBLK: &str = r#"
    {
      "blockdevices": [
        {
          "name": "sda",
          "kname": "sda",
          "type": "disk",
          "tran": "sata",
          "vendor": "ATA",
          "model": "WDC",
          "rm": false,
          "size": "1T",
          "mountpoints": [null]
        },
        {
          "name": "sr0",
          "kname": "sr0",
          "type": "rom",
          "tran": "sata",
          "vendor": "HL-DT-ST",
          "model": "DVDRAM GH24NSC0",
          "rm": true,
          "size": "1024M",
          "mountpoints": []
        }
      ]
    }
    "#;

    const DVD_R_BLANK: &str = r#"
    INQUIRY:                [HL-DT-ST][DVDRAM GH24NSC0 ][LI00]
    GET [CURRENT] CONFIGURATION:
     Mounted Media:         11h, DVD-R Sequential
     Current Write Speed:   8.0x1385=11080KB/s
     Write Speed #0:        8.0x1385=11080KB/s
     Write Speed #1:        4.0x1385=5540KB/s
    READ DISC INFORMATION:
     Disc status:           blank
     Number of Sessions:    1
     State of Last Session: empty
    READ TRACK INFORMATION:
     Track State:           invisible
     Free Blocks:           2295104*2KB
     Next Writable Address: 0*2KB
    "#;

    const DVD_RW_FORMATTED: &str = r#"
     Mounted Media:         1Ah, DVD+RW
     Current Write Speed:   4.0x1385=5540KB/s
    READ DISC INFORMATION:
     Disc status:           blank
     Free Blocks:           2295104*2KB
    "#;

    const EMPTY_TRAY: &str = r#"
    :-( no media mounted, exiting...
    INQUIRY:                [HL-DT-ST][DVDRAM GH24NSC0 ][LI00]
    GET [CURRENT] CONFIGURATION:
    "#;

    const BD_R_25_BLANK: &str = r#"
    INQUIRY:                [HL-DT-ST][BD-RE WH16NS60  ][1.02]
    GET [CURRENT] CONFIGURATION:
     Mounted Media:         41h, BD-R SRM
     Media ID:              VERBAT/IMc
     Current Write Speed:   6.0x4495=26970KB/s
     Write Speed #0:        6.0x4495=26970KB/s
     Write Speed #1:        4.0x4495=17980KB/s
     Write Speed #2:        2.0x4495=8990KB/s
    READ DISC INFORMATION:
     Disc status:           blank
     Number of Sessions:    1
     State of Last Session: empty
     Number of Tracks:      1
    READ TRACK INFORMATION[#1]:
     Track State:           invisible incremental
     Track Start Address:   0*2KB
     Next Writable Address: 0*2KB
     Free Blocks:           12219392*2KB
     Track Size:            12219392*2KB
    READ CAPACITY:          0*2048=0
    "#;

    const BD_RE_25_FORMATTED: &str = r#"
    INQUIRY:                [HL-DT-ST][BD-RE WH16NS60  ][1.02]
    GET [CURRENT] CONFIGURATION:
     Mounted Media:         43h, BD-RE
     Current Write Speed:   2.0x4495=8990KB/s
     Write Speed #0:        2.0x4495=8990KB/s
    READ DISC INFORMATION:
     Disc status:           blank
     Number of Sessions:    1
     State of Last Session: empty
     Number of Tracks:      1
    READ TRACK INFORMATION[#1]:
     Track State:           invisible incremental
     Track Start Address:   0*2KB
     Free Blocks:           11826176*2KB
     Track Size:            11826176*2KB
    READ CAPACITY:          0*2048=0
    "#;

    const BD_R_DL_50_BLANK: &str = r#"
    INQUIRY:                [PIONEER ][BD-RW   BDR-209D][1.30]
    GET [CURRENT] CONFIGURATION:
     Mounted Media:         41h, BD-R SRM+POW
     Media ID:              VERBAT/IMe
     Current Write Speed:   4.0x4495=17980KB/s
     Write Speed #0:        4.0x4495=17980KB/s
     Write Speed #1:        2.0x4495=8990KB/s
    READ DISC INFORMATION:
     Disc status:           blank
     Number of Sessions:    1
     State of Last Session: empty
     Number of Tracks:      1
    READ TRACK INFORMATION[#1]:
     Track State:           invisible incremental
     Track Start Address:   0*2KB
     Next Writable Address: 0*2KB
     Free Blocks:           24438784*2KB
     Track Size:            24438784*2KB
    READ CAPACITY:          0*2048=0
    "#;

    const BD_R_25_APPENDABLE: &str = r#"
    INQUIRY:                [PIONEER ][BD-RW   BDR-209D][1.30]
    GET [CURRENT] CONFIGURATION:
     Mounted Media:         41h, BD-R SRM+POW
     Current Write Speed:   12.0x4495=53940KB/s
    READ DISC INFORMATION:
     Disc status:           appendable
     Number of Sessions:    1
     State of Last Session: incomplete
     Number of Tracks:      2
    READ TRACK INFORMATION[#1]:
     Track State:           partial incremental
     Track Start Address:   0*2KB
     Free Blocks:           0*2KB
     Track Size:            12032000*2KB
    READ TRACK INFORMATION[#2]:
     Track State:           invisible incremental
     Track Start Address:   12032000*2KB
     Next Writable Address: 12032000*2KB
     Free Blocks:           56320*2KB
     Track Size:            56320*2KB
    READ CAPACITY:          12088320*2048=24756879360
    "#;

    const BD_ROM: &str = r#"
     Mounted Media:         40h, BD-ROM
    READ DISC INFORMATION:
     Disc status:           complete
    READ TRACK INFORMATION:
     Track Size:            12088320*2KB
     Free Blocks:           0*2KB
    "#;

    const DVD_ROM: &str = r#"
     Mounted Media:         00h, DVD-ROM
    READ DISC INFORMATION:
     Disc status:           complete
    READ TRACK INFORMATION:
     Track Size:            1000000*2KB
     Free Blocks:           0*2KB
    "#;

    #[test]
    fn parses_lsblk_and_filters_optical_devices() {
        let devices = parse_lsblk(LSBLK).expect("lsblk");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id.as_str(), "/dev/sr0");
        assert_eq!(devices[0].vendor, "HL-DT-ST");
        assert_eq!(devices[0].model, "DVDRAM GH24NSC0");
        assert_eq!(devices[0].bus, "sata");
    }

    #[test]
    fn parses_empty_lsblk_as_empty_array() {
        assert!(parse_lsblk(r#"{"blockdevices":[]}"#)
            .expect("lsblk")
            .is_empty());
    }

    #[test]
    fn parses_blank_dvd_r() {
        let media = parse_media_info(DVD_R_BLANK).expect("media");
        assert_eq!(media.kind, DiscMediaKind::DvdR);
        assert_eq!(media.status, MediaStatus::Blank);
        assert_eq!(media.capacity_bytes, 2_295_104 * 2048);
        assert_eq!(media.free_bytes, 2_295_104 * 2048);
        assert!(media.writable);
        assert!(!media.rewritable);
        assert_eq!(media.max_speed_x, Some(8.0));
    }

    #[test]
    fn parses_formatted_dvd_plus_rw() {
        let media = parse_media_info(DVD_RW_FORMATTED).expect("media");
        assert_eq!(media.kind, DiscMediaKind::DvdPlusRw);
        assert!(media.writable);
        assert!(media.rewritable);
        assert_eq!(media.max_speed_x, Some(4.0));
    }

    #[test]
    fn empty_tray_is_no_media() {
        assert!(matches!(
            parse_media_info(EMPTY_TRAY),
            Err(BurnError::NoMedia)
        ));
    }

    #[test]
    fn parses_dvd_rom_as_read_only() {
        let media = parse_media_info(DVD_ROM).expect("media");
        assert_eq!(media.kind, DiscMediaKind::DvdRom);
        assert_eq!(media.status, MediaStatus::Complete);
        assert!(!media.writable);
        assert!(!media.rewritable);
        assert_eq!(media.capacity_bytes, 1_000_000 * 2048);
    }

    #[test]
    fn parses_growisofs_progress() {
        let progress = parse_progress_line("builtin_dd: 1000*2KiB @ 4.0x (5540kB/s)", 10_240_000)
            .expect("progress");
        assert_eq!(progress.written_bytes, 2_048_000);
        assert!((progress.percent - 20.0).abs() < 0.01);
        assert_eq!(progress.speed_x, Some(4.0));
        assert_eq!(progress.eta_secs, Some(1));
    }

    #[test]
    fn ignores_unrelated_lines() {
        assert!(parse_progress_line("some other output", 10_000).is_none());
    }

    #[test]
    fn guards_reject_image_larger_than_media() {
        let media = parse_media_info(DVD_R_BLANK).expect("media");
        let request = BurnRequest {
            iso_path: PathBuf::from("/tmp/disc.iso"),
            device: DeviceId::new("/dev/sr0"),
            speed_x: Some(4),
            verify: false,
            eject: false,
            close_session: true,
            simulation: false,
            label: "TEST".to_string(),
        };
        let error = validate_burn_guards(&request, &media, media.capacity_bytes + 1)
            .expect_err("must reject");
        assert!(matches!(error, BurnError::InvalidRequest(_)));
    }

    #[test]
    fn guards_reject_non_writable_media() {
        let mut media = parse_media_info(DVD_R_BLANK).expect("media");
        media.writable = false;
        let request = BurnRequest {
            iso_path: PathBuf::from("/tmp/disc.iso"),
            device: DeviceId::new("/dev/sr0"),
            speed_x: None,
            verify: false,
            eject: false,
            close_session: true,
            simulation: false,
            label: "TEST".to_string(),
        };
        let error = validate_burn_guards(&request, &media, 1024).expect_err("must reject");
        assert!(matches!(error, BurnError::UnsupportedMedia(_)));
    }

    #[test]
    fn builds_growisofs_and_xorriso_commands() {
        let request = BurnRequest {
            iso_path: PathBuf::from("/tmp/disc.iso"),
            device: DeviceId::new("/dev/sr0"),
            speed_x: Some(4),
            verify: true,
            eject: true,
            close_session: true,
            simulation: false,
            label: "TEST".to_string(),
        };
        let grow = build_growisofs_args(&request);
        assert_eq!(grow[0], "-Z");
        assert_eq!(grow[1], "/dev/sr0=/tmp/disc.iso");
        assert!(grow.contains(&"-dvd-compat".to_string()));
        assert!(grow.contains(&"-speed=4".to_string()));
        let xor = build_xorriso_args(&request);
        assert!(xor.contains(&"dev=/dev/sr0".to_string()));
        assert!(xor.contains(&"/tmp/disc.iso".to_string()));
    }

    #[test]
    fn blu_ray_profiles_are_recognized_by_mmc_code() {
        assert_eq!(
            parse_media_kind(" Mounted Media:         41h, media sin nombre"),
            DiscMediaKind::BdR
        );
        assert_eq!(
            parse_media_kind(" Mounted Media:         43h, media sin nombre"),
            DiscMediaKind::BdRe
        );
        assert_eq!(
            parse_media_kind(" Mounted Media:         40h, media sin nombre"),
            DiscMediaKind::BdRom
        );
    }

    #[test]
    fn parses_blank_bd_r_25() {
        let media = parse_media_info(BD_R_25_BLANK).expect("media");
        assert_eq!(media.kind, DiscMediaKind::BdR);
        assert_eq!(media.status, MediaStatus::Blank);
        assert_eq!(media.capacity_bytes, 12_219_392 * 2048);
        assert_eq!(media.free_bytes, 12_219_392 * 2048);
        assert!(media.writable);
        assert!(!media.rewritable);
        assert_eq!(media.max_speed_x, Some(6.0));
    }

    #[test]
    fn parses_formatted_bd_re() {
        let media = parse_media_info(BD_RE_25_FORMATTED).expect("media");
        assert_eq!(media.kind, DiscMediaKind::BdRe);
        assert!(media.writable);
        assert!(media.rewritable);
        assert_eq!(media.capacity_bytes, 11_826_176 * 2048);
        assert_eq!(media.max_speed_x, Some(2.0));
    }

    #[test]
    fn parses_blank_bd_r_dl_50() {
        let media = parse_media_info(BD_R_DL_50_BLANK).expect("media");
        assert_eq!(media.kind, DiscMediaKind::BdR);
        assert_eq!(media.capacity_bytes, 24_438_784 * 2048);
        assert_eq!(media.free_bytes, 24_438_784 * 2048);
        assert_eq!(media.max_speed_x, Some(4.0));
    }

    #[test]
    fn sums_multitrack_bd_capacity_and_free_space() {
        let media = parse_media_info(BD_R_25_APPENDABLE).expect("media");
        assert_eq!(media.kind, DiscMediaKind::BdR);
        assert_eq!(media.status, MediaStatus::Appendable);
        assert_eq!(media.capacity_bytes, 12_088_320 * 2048);
        assert_eq!(media.free_bytes, 56_320 * 2048);
    }

    #[test]
    fn parses_bd_rom_as_read_only() {
        let media = parse_media_info(BD_ROM).expect("media");
        assert_eq!(media.kind, DiscMediaKind::BdRom);
        assert!(!media.writable);
        assert!(!media.rewritable);
        assert!(media.kind.read_only());
        assert!(media.kind.is_blu_ray());
    }

    #[test]
    fn bd_speed_uses_4495_kb_per_second() {
        assert_eq!(DiscMediaKind::BdR.bytes_per_second_at_1x(), 4_495_000.0);
        assert_eq!(
            DiscMediaKind::DvdR.bytes_per_second_at_1x(),
            DVD_1X_BYTES_PER_SECOND
        );
    }

    #[test]
    fn parses_bd_growisofs_progress() {
        let progress = parse_progress_line_with_speed(
            "builtin_dd: 1000*2KiB @ 2.0x",
            100_000_000,
            BD_1X_BYTES_PER_SECOND,
        )
        .expect("progress");
        assert_eq!(progress.written_bytes, 2_048_000);
        assert_eq!(progress.speed_x, Some(2.0));
        assert_eq!(progress.eta_secs, Some(10));
        assert!((progress.percent - 2.048).abs() < 0.01);
    }

    #[test]
    fn bd_growisofs_does_not_use_dvd_compat() {
        let request = BurnRequest {
            iso_path: PathBuf::from("/tmp/bd.iso"),
            device: DeviceId::new("/dev/sr0"),
            speed_x: Some(6),
            verify: false,
            eject: true,
            close_session: true,
            simulation: false,
            label: "BD".to_string(),
        };
        let bd = build_growisofs_args_for_media(&request, DiscMediaKind::BdR);
        assert!(!bd.contains(&"-dvd-compat".to_string()));
        assert!(bd.contains(&"-speed=6".to_string()));
        let dvd = build_growisofs_args_for_media(&request, DiscMediaKind::DvdR);
        assert!(dvd.contains(&"-dvd-compat".to_string()));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_stops_a_running_burn_command() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "dvdforge-burn-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let script = dir.join("slow-burner.sh");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").expect("script");
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let burner = LinuxBurner::default();
        let (progress, _receiver) = tokio::sync::mpsc::channel(8);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            trigger.cancel();
        });
        let result = burner
            .run_burn_command(
                &script,
                &[],
                1024,
                DVD_1X_BYTES_PER_SECOND,
                &progress,
                &cancel,
            )
            .await;
        assert!(matches!(result, Err(BurnError::Cancelled)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guards_accept_bd25_and_bd50_images() {
        let request = BurnRequest {
            iso_path: PathBuf::from("/tmp/bd.iso"),
            device: DeviceId::new("/dev/sr0"),
            speed_x: Some(4),
            verify: false,
            eject: false,
            close_session: true,
            simulation: false,
            label: "BD".to_string(),
        };
        let bd25 = parse_media_info(BD_R_25_BLANK).expect("media");
        validate_burn_guards(&request, &bd25, 20 << 30).expect("BD-25 de 20 GiB");
        assert!(matches!(
            validate_burn_guards(&request, &bd25, 26 << 30),
            Err(BurnError::InvalidRequest(_))
        ));
        let bd50 = parse_media_info(BD_R_DL_50_BLANK).expect("media");
        validate_burn_guards(&request, &bd50, 45 << 30).expect("BD-50 de 45 GiB");
        assert!(matches!(
            validate_burn_guards(&request, &bd50, 51 << 30),
            Err(BurnError::InvalidRequest(_))
        ));
    }
}
