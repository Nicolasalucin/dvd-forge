//! macOS optical drive backend built on `system_profiler`, `drutil`,
//! `hdiutil` and `diskutil`.
//!
//! Parsers and command builders in this module are intentionally independent
//! from macOS so they can be unit-tested on Linux.

use crate::{
    validate_media_budget, BurnBackend, BurnError, BurnProgress, BurnReport, BurnRequest, DeviceId,
    DiscBurner, DiscMediaKind, MediaInfo, MediaStatus, OpticalDevice, ProgressSink, VerifyReport,
    DVD_SECTOR_BYTES,
};
use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct MacOsBurner {
    pub system_profiler: PathBuf,
    pub drutil: PathBuf,
    pub hdiutil: PathBuf,
    pub diskutil: PathBuf,
}

impl Default for MacOsBurner {
    fn default() -> Self {
        Self {
            system_profiler: PathBuf::from("system_profiler"),
            drutil: PathBuf::from("drutil"),
            hdiutil: PathBuf::from("hdiutil"),
            diskutil: PathBuf::from("diskutil"),
        }
    }
}

impl MacOsBurner {
    pub fn new() -> Self {
        Self::default()
    }

    async fn ensure_detected_device(&self, requested: &DeviceId) -> Result<(), BurnError> {
        let devices = self.list_devices().await?;
        let requested = normalize_macos_device_id(requested.as_str());
        let detected = devices.iter().map(|device| device.id.as_str().to_string());
        if crate::device_matches_allowlist(&requested, detected, normalize_macos_device_id) {
            Ok(())
        } else {
            Err(BurnError::InvalidRequest(format!(
                "la unidad {requested} no es un dispositivo óptico detectado"
            )))
        }
    }
}

#[async_trait]
impl DiscBurner for MacOsBurner {
    fn backend(&self) -> BurnBackend {
        BurnBackend::Drutil
    }

    async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError> {
        let system_profiler = command_output(
            &self.system_profiler,
            &["SPSDiscBurningDataType".to_string(), "-json".to_string()],
        )
        .await;
        let drutil = command_output(&self.drutil, &["list".to_string()]).await;

        let mut errors = Vec::new();
        let profiler_devices = match system_profiler {
            Ok(output) if output.status.success() => {
                let text = combined_output(&output);
                match parse_system_profiler_devices(&text) {
                    Ok(devices) => devices,
                    Err(error) => {
                        errors.push(error);
                        Vec::new()
                    }
                }
            }
            Ok(output) => {
                errors.push(BurnError::ProcessFailed {
                    tool: self.system_profiler.display().to_string(),
                    code: output.status.code().unwrap_or(-1),
                    stderr: combined_output(&output),
                });
                Vec::new()
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };

        let drutil_devices = match drutil {
            Ok(output) if output.status.success() => parse_drutil_list(&combined_output(&output)),
            Ok(output) => {
                errors.push(BurnError::ProcessFailed {
                    tool: self.drutil.display().to_string(),
                    code: output.status.code().unwrap_or(-1),
                    stderr: combined_output(&output),
                });
                Vec::new()
            }
            Err(error) => {
                errors.push(error);
                Vec::new()
            }
        };

        let devices = merge_devices(profiler_devices, drutil_devices);
        if devices.is_empty() && errors.len() >= 2 {
            return Err(errors.remove(0));
        }
        Ok(devices)
    }

    async fn probe_media(&self, device: &DeviceId) -> Result<MediaInfo, BurnError> {
        let selector = self.resolve_selector(device).await;
        let args = build_drutil_status_args(&selector);
        let output = command_output(&self.drutil, &args).await;
        let text = match output {
            Ok(output) => combined_output(&output),
            Err(error) => return Err(error),
        };
        match parse_drutil_status(&text) {
            Ok(media) => Ok(media),
            Err(BurnError::NoMedia) if !selector.is_empty() => {
                // `drutil status` without a selector reports all drives. It is
                // a useful fallback when the selector syntax differs.
                let output = command_output(&self.drutil, &["status".to_string()]).await?;
                parse_drutil_status(&combined_output(&output))
            }
            Err(error) => Err(error),
        }
    }

    async fn burn(
        &self,
        request: BurnRequest,
        progress: ProgressSink,
        cancel: CancellationToken,
    ) -> Result<BurnReport, BurnError> {
        if request.simulation {
            return Err(BurnError::InvalidRequest(
                "la simulación no está soportada por el backend macOS".to_string(),
            ));
        }
        let iso_size = std::fs::metadata(&request.iso_path).map_err(map_io)?.len();
        self.ensure_detected_device(&request.device).await?;
        let media = self.probe_media(&request.device).await?;
        validate_media_budget(&media, iso_size, request.speed_x)?;
        let selector = self.resolve_selector(&request.device).await;
        let started = Instant::now();

        let drutil_args = build_drutil_burn_args(&selector, &request);
        let mut backend = BurnBackend::Drutil;
        let drutil_result = run_with_progress(
            &self.drutil,
            &drutil_args,
            iso_size,
            media.kind,
            progress.clone(),
            cancel.clone(),
        )
        .await;
        if let Err(error) = drutil_result {
            match error {
                BurnError::ProcessFailed { .. } | BurnError::ToolMissing(_) => {
                    let args = build_hdiutil_burn_args(&request);
                    run_with_progress(
                        &self.hdiutil,
                        &args,
                        iso_size,
                        media.kind,
                        progress.clone(),
                        cancel.clone(),
                    )
                    .await?;
                    backend = BurnBackend::Hdiutil;
                }
                other => return Err(other),
            }
        }

        let mut verified = false;
        if request.verify {
            // Read-back takes as long as the burn: honour Cancel here too.
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

        let ejected_by_burn = request.eject && !request.verify && backend == BurnBackend::Drutil;
        if request.eject && !ejected_by_burn {
            self.eject(&request.device).await?;
        }

        let elapsed = started.elapsed();
        let average_speed_x = if elapsed.as_secs_f64() > 0.0 {
            Some(
                ((iso_size as f64 / elapsed.as_secs_f64()) / media.kind.bytes_per_second_at_1x())
                    as f32,
            )
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
        // Hash the ISO on a blocking thread while the drive is read back.
        let iso_hash = tokio::task::spawn_blocking({
            let iso = iso.to_path_buf();
            move || hash_file(&iso)
        });
        let raw = raw_device_path(device.as_str());
        let mut file = tokio::fs::File::open(&raw).await.map_err(map_io)?;
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
            let _ = progress
                .send(BurnProgress {
                    written_bytes: read_total.min(iso_size),
                    total_bytes: iso_size,
                    percent: (read_total.min(iso_size) as f32 / iso_size as f32) * 100.0,
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
        let selector = self.resolve_selector(device).await;
        let args = build_drutil_eject_args(&selector);
        match command_output(&self.drutil, &args).await {
            Ok(output) if output.status.success() => Ok(()),
            drutil_error => {
                let fallback = build_diskutil_eject_args(device.as_str());
                let output = command_output(&self.diskutil, &fallback).await?;
                if output.status.success() {
                    Ok(())
                } else {
                    match drutil_error {
                        Ok(output) => Err(BurnError::ProcessFailed {
                            tool: self.drutil.display().to_string(),
                            code: output.status.code().unwrap_or(-1),
                            stderr: combined_output(&output),
                        }),
                        Err(error) => Err(error),
                    }
                }
            }
        }
    }
}

impl MacOsBurner {
    async fn resolve_selector(&self, device: &DeviceId) -> String {
        if let Ok(output) = command_output(&self.drutil, &["list".to_string()]).await {
            if output.status.success() {
                if let Some(selector) =
                    resolve_drutil_selector(&combined_output(&output), device.as_str())
                {
                    return selector;
                }
            }
        }
        device.as_str().to_string()
    }
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

async fn command_output(binary: &Path, args: &[String]) -> Result<std::process::Output, BurnError> {
    Command::new(binary)
        .args(args)
        .output()
        .await
        .map_err(|error| map_command_error(binary, error))
}

fn combined_output(output: &std::process::Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        if !text.ends_with('\n') && !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    text
}

/// Extracts the drive list from `system_profiler SPSDiscBurningDataType -json`.
pub fn parse_system_profiler_devices(json: &str) -> Result<Vec<OpticalDevice>, BurnError> {
    let value: Value = serde_json::from_str(json)?;
    let entries = value
        .get("SPSDiscBurningDataType")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .cloned()
        .unwrap_or_default();
    let mut devices = Vec::new();
    for entry in entries {
        collect_system_profiler_entry(&entry, &mut devices);
    }
    devices.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    devices.dedup_by(|a, b| a.id == b.id);
    Ok(devices)
}

fn collect_system_profiler_entry(value: &Value, devices: &mut Vec<OpticalDevice>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(items) = object.get("_items").and_then(Value::as_array) {
        for item in items {
            collect_system_profiler_entry(item, devices);
        }
    }
    let detail = object
        .get("disc_burning")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let name = string_field(detail, "device_name")
        .or_else(|| string_field(object, "device_name"))
        .or_else(|| string_field(detail, "bsd_name"))
        .or_else(|| string_field(object, "bsd_name"));
    let Some(name) = name else {
        return;
    };
    let id = normalize_macos_device_id(&name);
    if id.is_empty() {
        return;
    }
    let display = string_field(object, "_name").unwrap_or_default();
    let vendor = string_field(detail, "vendor")
        .or_else(|| string_field(object, "vendor"))
        .unwrap_or_else(|| display.split_whitespace().next().unwrap_or("").to_string());
    let model = string_field(detail, "model")
        .or_else(|| string_field(object, "model"))
        .unwrap_or_else(|| {
            let mut words = display.split_whitespace();
            words.next();
            words.collect::<Vec<_>>().join(" ")
        });
    let bus = string_field(detail, "interconnect")
        .or_else(|| string_field(detail, "bus"))
        .unwrap_or_else(|| "unknown".to_string());
    let writable = detail
        .get("write_capabilities")
        .and_then(Value::as_array)
        .map(|values| !values.is_empty())
        .unwrap_or(true);
    devices.push(OpticalDevice {
        id: DeviceId::new(id),
        vendor: vendor.trim().to_string(),
        model: model.trim().to_string(),
        bus,
        writable,
        media_present: false,
    });
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Parses `drutil list`, whose column layout is stable across macOS releases.
pub fn parse_drutil_list(text: &str) -> Vec<OpticalDevice> {
    parse_drutil_records(text)
        .into_iter()
        .map(|record| OpticalDevice {
            id: DeviceId::new(record.bsd),
            vendor: record.vendor,
            model: record.product,
            bus: "unknown".to_string(),
            writable: true,
            media_present: false,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DrutilRecord {
    vendor: String,
    product: String,
    revision: String,
    bsd: String,
    drive: String,
}

fn parse_drutil_records(text: &str) -> Vec<DrutilRecord> {
    let lines: Vec<&str> = text.lines().collect();
    let header_index = lines.iter().position(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("vendor") && lower.contains("product") && lower.contains("bsd")
    });
    if let Some(index) = header_index {
        let header = lines[index];
        if let (Some(vendor_at), Some(product_at), Some(rev_at), Some(bsd_at)) = (
            header.find("Vendor"),
            header.find("Product"),
            header.find("Rev"),
            header.find("BSD"),
        ) {
            let drive_at = header[bsd_at..]
                .find("Dr")
                .map(|offset| bsd_at + offset)
                .unwrap_or(header.len());
            let mut records = Vec::new();
            for line in lines.iter().skip(index + 1) {
                if line.trim().is_empty() {
                    continue;
                }
                let padded = format!("{line:<width$}", width = header.len());
                let vendor = padded.get(vendor_at..product_at).unwrap_or("").trim();
                let product = padded.get(product_at..rev_at).unwrap_or("").trim();
                let revision = padded.get(rev_at..bsd_at).unwrap_or("").trim();
                let bsd = padded.get(bsd_at..drive_at).unwrap_or("").trim();
                let drive = padded.get(drive_at..).unwrap_or("").trim();
                if !bsd.contains("disk") {
                    continue;
                }
                records.push(DrutilRecord {
                    vendor: collapse_whitespace(vendor),
                    product: collapse_whitespace(product),
                    revision: revision.to_string(),
                    bsd: normalize_macos_device_id(bsd),
                    drive: drive.to_string(),
                });
            }
            if !records.is_empty() {
                return records;
            }
        }
    }

    text.lines()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            if tokens.len() < 2 {
                return None;
            }
            let bsd_index = tokens.iter().position(|token| token.contains("disk"))?;
            let vendor = tokens.first()?.to_string();
            let product = collapse_whitespace(
                &tokens
                    .iter()
                    .take(bsd_index)
                    .skip(1)
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            Some(DrutilRecord {
                vendor,
                product,
                revision: String::new(),
                bsd: normalize_macos_device_id(tokens[bsd_index]),
                drive: tokens.last().copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Finds the `drutil -drive` selector (1-based drive index, or vendor/product)
/// for a BSD device such as `disk2`.
pub fn resolve_drutil_selector(list_output: &str, device: &str) -> Option<String> {
    let wanted = normalize_macos_device_id(device);
    let record = parse_drutil_records(list_output)
        .into_iter()
        .find(|record| record.bsd == wanted)?;
    if !record.drive.is_empty() {
        Some(record.drive)
    } else if !record.vendor.is_empty() {
        Some(record.vendor)
    } else {
        None
    }
}

fn merge_devices(
    mut primary: Vec<OpticalDevice>,
    secondary: Vec<OpticalDevice>,
) -> Vec<OpticalDevice> {
    for device in &mut primary {
        if let Some(other) = secondary.iter().find(|other| other.id == device.id) {
            if device.vendor.is_empty() {
                device.vendor = other.vendor.clone();
            }
            if device.model.is_empty() {
                device.model = other.model.clone();
            }
            if device.bus == "unknown" {
                device.bus = other.bus.clone();
            }
            device.writable |= other.writable;
            device.media_present |= other.media_present;
        }
    }
    for device in secondary {
        if !primary.iter().any(|existing| existing.id == device.id) {
            primary.push(device);
        }
    }
    primary.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    primary
}

/// Parses the media section emitted by `drutil status`.
pub fn parse_drutil_status(text: &str) -> Result<MediaInfo, BurnError> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("no media")
        || lower.contains("no disc")
        || lower.contains("media not present")
        || lower.contains("no writable")
    {
        return Err(BurnError::NoMedia);
    }
    let type_line = text
        .lines()
        .find(|line| line.trim_start().starts_with("Type:"))
        .ok_or(BurnError::NoMedia)?;
    let type_value = type_line
        .split_once(':')
        .map(|(_, value)| value)
        .unwrap_or("");
    let type_value = type_value
        .split("Name:")
        .next()
        .unwrap_or(type_value)
        .trim();
    let kind = parse_macos_media_kind(type_value);
    if kind == DiscMediaKind::Unknown {
        return Err(BurnError::UnsupportedMedia(type_value.to_string()));
    }
    let writability = text
        .lines()
        .find(|line| line.trim_start().starts_with("Writability:"))
        .map(|line| line.split_once(':').map(|(_, value)| value).unwrap_or(""))
        .unwrap_or("")
        .to_ascii_lowercase();
    let status = if writability.contains("blank") {
        MediaStatus::Blank
    } else if writability.contains("appendable") || writability.contains("overwritable") {
        MediaStatus::Appendable
    } else if writability.contains("closed") || writability.contains("finalized") {
        MediaStatus::Closed
    } else if writability.contains("complete") {
        MediaStatus::Complete
    } else {
        MediaStatus::Unknown
    };
    let reported_free = line_block_count(text, "Space Free:").unwrap_or(0);
    let used_blocks = line_block_count(text, "Space Used:").unwrap_or(0);
    let overwritable = line_block_count(text, "Overwritable:").unwrap_or(0);
    let free_blocks = if reported_free == 0 {
        overwritable
    } else {
        reported_free
    };
    let free_bytes = free_blocks.saturating_mul(DVD_SECTOR_BYTES);
    let capacity_blocks = if used_blocks > 0 || free_blocks > 0 {
        used_blocks.saturating_add(free_blocks)
    } else {
        overwritable
    };
    let writable = kind.writable()
        && !writability.contains("read-only")
        && !writability.contains("read only")
        && !writability.contains("not writable")
        && !writability.contains("write protected");
    Ok(MediaInfo {
        kind,
        status,
        capacity_bytes: capacity_blocks.saturating_mul(DVD_SECTOR_BYTES),
        free_bytes,
        writable,
        rewritable: kind.rewritable(),
        max_speed_x: parse_write_speeds(text),
    })
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn line_block_count(text: &str, prefix: &str) -> Option<u64> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))?;
    let (_, value) = line.split_once("blocks:")?;
    parse_number_prefix(value)
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

fn parse_write_speeds(text: &str) -> Option<f32> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with("Write Speed"))?;
    line.split(|character: char| character == ',' || character.is_whitespace())
        .filter_map(|token| {
            let token = token.trim();
            let (value, _) = token.split_once('x')?;
            value.parse::<f32>().ok()
        })
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

fn parse_macos_media_kind(value: &str) -> DiscMediaKind {
    let lower = value.to_ascii_lowercase();
    if lower.contains("bd-re") {
        DiscMediaKind::BdRe
    } else if lower.contains("bd-rom") || lower.contains("blu-ray") || lower.contains("bluray") {
        DiscMediaKind::BdRom
    } else if lower.contains("bd-r") {
        DiscMediaKind::BdR
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

/// Normalizes `disk2`, `/dev/disk2` or `rdisk2` to `disk2`.
pub fn normalize_macos_device_id(value: &str) -> String {
    let lower = value.trim().to_ascii_lowercase();
    if let Some(index) = lower.find("disk") {
        let digits: String = lower[index + 4..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        if !digits.is_empty() {
            return format!("disk{digits}");
        }
    }
    value.trim().trim_start_matches("/dev/").to_string()
}

/// Returns the raw character device used for SHA-256 verification.
pub fn raw_device_path(device: &str) -> String {
    let normalized = normalize_macos_device_id(device);
    format!("/dev/r{normalized}")
}

/// Returns the block device node used by `hdiutil` and `diskutil`.
pub fn device_node_path(device: &str) -> String {
    let normalized = normalize_macos_device_id(device);
    format!("/dev/{normalized}")
}

pub fn build_drutil_status_args(selector: &str) -> Vec<String> {
    vec![
        "-drive".to_string(),
        selector.to_string(),
        "status".to_string(),
    ]
}

pub fn build_drutil_burn_args(selector: &str, request: &BurnRequest) -> Vec<String> {
    let mut args = vec![
        "-drive".to_string(),
        selector.to_string(),
        "burn".to_string(),
        "-noverify".to_string(),
    ];
    if request.eject && !request.verify {
        args.push("-eject".to_string());
    }
    if let Some(speed) = request.speed_x {
        args.push("-speed".to_string());
        args.push(speed.to_string());
    }
    args.push(request.iso_path.display().to_string());
    args
}

pub fn build_hdiutil_burn_args(request: &BurnRequest) -> Vec<String> {
    let mut args = vec![
        "burn".to_string(),
        request.iso_path.display().to_string(),
        "-device".to_string(),
        device_node_path(request.device.as_str()),
        // hdiutil ejects and verifies by default; DVD Forge runs its own
        // SHA-256 verify and eject afterwards and needs the disc in the tray.
        "-noeject".to_string(),
        "-noverifyburn".to_string(),
    ];
    if request.close_session {
        args.push("-forceclose".to_string());
    }
    if let Some(speed) = request.speed_x {
        args.push("-speed".to_string());
        args.push(speed.to_string());
    }
    args
}

pub fn build_drutil_eject_args(selector: &str) -> Vec<String> {
    vec![
        "-drive".to_string(),
        selector.to_string(),
        "eject".to_string(),
    ]
}

pub fn build_diskutil_eject_args(device: &str) -> Vec<String> {
    vec!["eject".to_string(), device_node_path(device)]
}

/// Parses percentage or phase lines emitted by drutil/hdiutil.
pub fn parse_progress_line(
    line: &str,
    total_bytes: u64,
    media_kind: DiscMediaKind,
) -> Option<BurnProgress> {
    let lower = line.to_ascii_lowercase();
    let percent = parse_percent(line).or_else(|| {
        if lower.contains("burn completed") {
            Some(100.0)
        } else if lower.contains("closing session") || lower.contains("finishing burn") {
            Some(98.0)
        } else if lower.contains("closing track") {
            Some(90.0)
        } else if lower.contains("writing track") {
            Some(5.0)
        } else if lower.contains("opening session") || lower.contains("opening track") {
            Some(1.0)
        } else {
            None
        }
    })?;
    let percent = percent.clamp(0.0, 100.0);
    let written_bytes = ((total_bytes as f64 * percent as f64 / 100.0) as u64).min(total_bytes);
    let speed = parse_speed(line);
    let bytes_per_second = media_kind.bytes_per_second_at_1x();
    Some(BurnProgress {
        written_bytes,
        total_bytes,
        percent,
        speed_x: speed,
        eta_secs: speed.and_then(|speed| {
            if speed > 0.0 && written_bytes < total_bytes {
                Some(
                    (((total_bytes - written_bytes) as f64) / (speed as f64 * bytes_per_second))
                        as u64,
                )
            } else {
                None
            }
        }),
    })
}

fn parse_percent(line: &str) -> Option<f32> {
    let index = line.find('%')?;
    let prefix = line[..index].trim_end();
    let token = prefix.split_whitespace().last()?;
    token
        .trim_matches(|character: char| !character.is_ascii_digit() && character != '.')
        .parse::<f32>()
        .ok()
}

fn parse_speed(line: &str) -> Option<f32> {
    line.split(|character: char| character.is_whitespace() || character == ',' || character == ';')
        .filter_map(|token| {
            let token = token.trim_matches(|character: char| {
                character == '(' || character == ')' || character == ','
            });
            let (value, _) = token.split_once('x')?;
            value.parse::<f32>().ok()
        })
        .next()
}

async fn run_with_progress(
    binary: &Path,
    args: &[String],
    total_bytes: u64,
    media_kind: DiscMediaKind,
    progress: ProgressSink,
    cancel: CancellationToken,
) -> Result<(), BurnError> {
    let mut child = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| map_command_error(binary, error))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BurnError::InvalidRequest("no se pudo capturar stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| BurnError::InvalidRequest("no se pudo capturar stderr".to_string()))?;
    let stdout_task = tokio::spawn(read_progress_stream(
        stdout,
        total_bytes,
        media_kind,
        progress.clone(),
    ));
    let stderr_task = tokio::spawn(read_progress_stream(
        stderr,
        total_bytes,
        media_kind,
        progress.clone(),
    ));
    let status = tokio::select! {
        _ = cancel.cancelled() => {
            terminate_child(&mut child).await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(BurnError::Cancelled);
        }
        status = child.wait() => status.map_err(map_io)?,
    };
    let stdout_log = stdout_task.await.unwrap_or_default();
    let stderr_log = stderr_task.await.unwrap_or_default();
    let log = format!("{stdout_log}\n{stderr_log}");
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
            stderr: log.trim().to_string(),
        })
    }
}

async fn read_progress_stream<R>(
    mut stream: R,
    total_bytes: u64,
    media_kind: DiscMediaKind,
    progress: ProgressSink,
) -> String
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buffer = [0u8; 4096];
    let mut pending = String::new();
    let mut log = String::new();
    while let Ok(read) = stream.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        let text = String::from_utf8_lossy(&buffer[..read]);
        log.push_str(&text);
        pending.push_str(&text);
        while let Some(index) = pending.find(['\n', '\r']) {
            let line = pending[..index].to_string();
            pending.drain(..=index);
            if let Some(update) = parse_progress_line(&line, total_bytes, media_kind) {
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
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(map_io)?;
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

    const LIST: &str = "\
 Vendor   Product           Rev   BSD Name  Dr
 MATSHITA DVD-R   UJ-8A8    HA13  disk1     1
 HL-DT-ST DVDRAM GP65NB60  1.00  disk2     2
";

    const STATUS: &str = "\
 Vendor   Product           Rev
 MATSHITA DVD-R   UJ-8A8    HA13

           Type: DVD-R                Name: /dev/disk1
   Write Speeds: 2x, 4x, 8x
   Overwritable:  510:38:38         blocks:  2297888 /   4.71GB /   4.38GiB
     Space Free:  510:38:38         blocks:  2297888 /   4.71GB /   4.38GiB
     Space Used:   00:00:00         blocks:        0 /   0.00MB /   0.00MiB
    Writability: appendable, blank, overwritable
      Book Type: DVD-R (v5)
";

    const EMPTY: &str = "No Media Inserted\n";

    const PROFILER: &str = r#"{
      "SPSDiscBurningDataType": [
        {
          "_name": "HL-DT-ST DVDRAM GP65NB60",
          "disc_burning": {
            "device_name": "disk2",
            "vendor": "HL-DT-ST",
            "model": "DVDRAM GP65NB60",
            "interconnect": "USB",
            "write_capabilities": ["CD-R", "DVD-R", "DVD+R"]
          }
        }
      ]
    }"#;

    fn request() -> BurnRequest {
        BurnRequest {
            iso_path: PathBuf::from("/tmp/disc.iso"),
            device: DeviceId::new("disk2"),
            speed_x: Some(4),
            verify: true,
            eject: true,
            close_session: true,
            simulation: false,
            label: "AVCHD".into(),
        }
    }

    #[test]
    fn parses_system_profiler_devices() {
        let devices = parse_system_profiler_devices(PROFILER).expect("json");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id.as_str(), "disk2");
        assert_eq!(devices[0].vendor, "HL-DT-ST");
        assert_eq!(devices[0].model, "DVDRAM GP65NB60");
        assert_eq!(devices[0].bus, "USB");
        assert!(devices[0].writable);
    }

    #[test]
    fn parses_drutil_list_and_resolves_selector() {
        let devices = parse_drutil_list(LIST);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id.as_str(), "disk1");
        assert_eq!(devices[0].vendor, "MATSHITA");
        assert_eq!(devices[0].model, "DVD-R UJ-8A8");
        assert_eq!(resolve_drutil_selector(LIST, "disk2").as_deref(), Some("2"));
        assert_eq!(
            resolve_drutil_selector(LIST, "/dev/disk1").as_deref(),
            Some("1")
        );
    }

    #[test]
    fn parses_drutil_status() {
        let media = parse_drutil_status(STATUS).expect("media");
        assert_eq!(media.kind, DiscMediaKind::DvdR);
        assert_eq!(media.status, MediaStatus::Blank);
        assert_eq!(media.capacity_bytes, 2_297_888 * DVD_SECTOR_BYTES);
        assert_eq!(media.free_bytes, 2_297_888 * DVD_SECTOR_BYTES);
        assert!(media.writable);
        assert!(!media.rewritable);
        assert_eq!(media.max_speed_x, Some(8.0));
    }

    #[test]
    fn parses_blu_ray_media_and_progress_speed() {
        let status = "\
           Type: BD-R                Name: /dev/disk3
   Writability: appendable
   Space Free: 11826176 blocks
   Space Used: 0 blocks
   Write Speeds: 2x, 4x, 6x
";
        let media = parse_drutil_status(status).expect("bd media");
        assert_eq!(media.kind, DiscMediaKind::BdR);
        assert!(media.writable);
        let progress = parse_progress_line("Writing: 50.0% at 4.0x", 100_000_000, media.kind)
            .expect("bd progress");
        let dvd_eta = (50_000_000.0f64 / (4.0 * crate::DVD_1X_BYTES_PER_SECOND)) as u64;
        let bd_eta = (50_000_000.0f64 / (4.0 * crate::BD_1X_BYTES_PER_SECOND)) as u64;
        assert_eq!(progress.eta_secs, Some(bd_eta));
        assert!(bd_eta < dvd_eta);
    }

    #[test]
    fn detects_empty_macos_tray() {
        assert!(matches!(
            parse_drutil_status(EMPTY),
            Err(BurnError::NoMedia)
        ));
    }

    #[test]
    fn builds_drutil_and_hdiutil_commands() {
        let mut request = request();
        request.verify = false;
        let drutil = build_drutil_burn_args("2", &request).join(" ");
        assert_eq!(
            drutil,
            "-drive 2 burn -noverify -eject -speed 4 /tmp/disc.iso"
        );
        let hdiutil = build_hdiutil_burn_args(&request).join(" ");
        assert_eq!(
            hdiutil,
            "burn /tmp/disc.iso -device /dev/disk2 -noeject -noverifyburn -forceclose -speed 4"
        );
        assert_eq!(build_drutil_status_args("2").join(" "), "-drive 2 status");
        assert_eq!(build_drutil_eject_args("2").join(" "), "-drive 2 eject");
        assert_eq!(
            build_diskutil_eject_args("disk2").join(" "),
            "eject /dev/disk2"
        );
        let mut verify = request.clone();
        verify.verify = true;
        assert!(!build_drutil_burn_args("2", &verify)
            .join(" ")
            .contains("-eject"));
    }

    #[test]
    fn normalizes_raw_devices() {
        assert_eq!(normalize_macos_device_id("/dev/disk2"), "disk2");
        assert_eq!(normalize_macos_device_id("rdisk3"), "disk3");
        assert_eq!(raw_device_path("disk2"), "/dev/rdisk2");
        assert_eq!(raw_device_path("/dev/disk4"), "/dev/rdisk4");
        assert_eq!(device_node_path("disk2"), "/dev/disk2");
    }

    #[test]
    fn parses_progress_percent_and_phases() {
        let progress = parse_progress_line("Writing: 42.5% complete", 1000, DiscMediaKind::DvdR)
            .expect("progress");
        assert!((progress.percent - 42.5).abs() < 0.01);
        assert_eq!(progress.written_bytes, 425);
        let phase =
            parse_progress_line("Closing session", 1000, DiscMediaKind::DvdR).expect("phase");
        assert!((phase.percent - 98.0).abs() < 0.01);
        assert_eq!(
            parse_progress_line("Burn completed successfully", 1000, DiscMediaKind::DvdR)
                .expect("completed")
                .percent,
            100.0
        );
    }

    #[test]
    fn merges_profiler_and_drutil_devices() {
        let profiler = parse_system_profiler_devices(PROFILER).expect("profiler");
        let drutil = parse_drutil_list(LIST);
        let merged = merge_devices(profiler, drutil);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id.as_str(), "disk1");
        assert_eq!(merged[1].id.as_str(), "disk2");
        assert_eq!(merged[1].bus, "USB");
    }
}
