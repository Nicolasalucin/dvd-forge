//! Optical disc burning abstractions and platform backends: Linux, macOS,
//! FreeBSD and Windows IMAPI2.

pub mod freebsd;
pub mod linux;
pub mod macos;
pub mod windows;

pub use freebsd::{parse_camcontrol_devlist, FreeBsdBurner};
pub use linux::{
    build_growisofs_args, build_growisofs_args_for_media, build_xorriso_args, parse_lsblk,
    parse_media_info, parse_progress_line, parse_progress_line_with_speed, validate_burn_guards,
    LinuxBurner,
};
pub use macos::{
    build_diskutil_eject_args, build_drutil_burn_args, build_drutil_eject_args,
    build_drutil_status_args, build_hdiutil_burn_args, device_node_path, normalize_macos_device_id,
    parse_drutil_list, parse_drutil_status, parse_system_profiler_devices, raw_device_path,
    resolve_drutil_selector, MacOsBurner,
};
pub use tokio_util::sync::CancellationToken;
#[cfg(target_os = "windows")]
pub use windows::WindowsBurner;
pub use windows::{
    imapi_media_kind, imapi_media_status, imapi_media_writable, imapi_sectors_to_bytes,
    normalize_drive_id, raw_volume_path, sectors_per_second_to_speed_x,
    speed_x_to_sectors_per_second, stream_progress, StreamProgressTracker,
};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::mpsc;

pub const CRATE_NAME: &str = "dvdforge-burn";

pub const DVD_SECTOR_BYTES: u64 = 2_048;
pub const DVD_1X_BYTES_PER_SECOND: f64 = 1_385_000.0;
/// Blu-ray 1x is 4495 KB/s (36 Mbit/s), per the BD-ROM specification.
pub const BD_1X_BYTES_PER_SECOND: f64 = 4_495_000.0;

/// Shared media/speed validation used by the macOS and Windows backends.
pub fn validate_media_budget(
    media: &MediaInfo,
    iso_size: u64,
    speed_x: Option<u16>,
) -> Result<(), BurnError> {
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
    if let Some(speed) = speed_x {
        if speed == 0 {
            return Err(BurnError::InvalidRequest(
                "la velocidad debe ser mayor que cero".to_string(),
            ));
        }
    }
    Ok(())
}

/// Resolves a device node to its canonical path when it exists, falling back to
/// the trimmed input. Used to compare the requested device with the backend's
/// detected optical-drive allowlist.
pub fn canonical_device_path(device: &str) -> String {
    std::fs::canonicalize(device)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| device.trim().to_string())
}

/// Checks a requested device against the optical drives reported by a backend.
/// Both sides go through the same normalization so `/dev/sr0` and
/// `/dev/disk/by-id/...` (or `D` and `D:`) compare equal when they point at the
/// same drive.
pub fn device_matches_allowlist(
    requested: &str,
    detected: impl IntoIterator<Item = String>,
    normalize: impl Fn(&str) -> String,
) -> bool {
    let requested = normalize(requested);
    detected
        .into_iter()
        .any(|device| normalize(&device) == requested)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    FreeBsd,
    MacOs,
    Windows,
}

impl Platform {
    pub const fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(target_os = "freebsd")]
        {
            Self::FreeBsd
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "macos",
            target_os = "windows"
        )))]
        {
            Self::Linux
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::FreeBsd => "freebsd",
            Self::MacOs => "macos",
            Self::Windows => "windows",
        }
    }
}

/// Returns the backend selected at compile time for the current OS.
pub fn platform_burner() -> Box<dyn DiscBurner> {
    #[cfg(target_os = "linux")]
    {
        Box::new(LinuxBurner::default())
    }
    #[cfg(target_os = "freebsd")]
    {
        Box::new(FreeBsdBurner::default())
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(MacOsBurner::default())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(WindowsBurner::new())
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "windows"
    )))]
    {
        Box::new(UnsupportedBurner)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos",
    target_os = "windows"
)))]
struct UnsupportedBurner;

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos",
    target_os = "windows"
)))]
#[async_trait::async_trait]
impl DiscBurner for UnsupportedBurner {
    fn backend(&self) -> BurnBackend {
        BurnBackend::Growisofs
    }
    async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError> {
        Err(BurnError::ToolMissing(
            "backend de grabación no soportado".to_string(),
        ))
    }
    async fn probe_media(&self, _device: &DeviceId) -> Result<MediaInfo, BurnError> {
        Err(BurnError::ToolMissing(
            "backend de grabación no soportado".to_string(),
        ))
    }
    async fn burn(
        &self,
        _request: BurnRequest,
        _progress: ProgressSink,
        _cancel: CancellationToken,
    ) -> Result<BurnReport, BurnError> {
        Err(BurnError::ToolMissing(
            "backend de grabación no soportado".to_string(),
        ))
    }
    async fn verify(
        &self,
        _device: &DeviceId,
        _iso: &std::path::Path,
        _progress: ProgressSink,
    ) -> Result<VerifyReport, BurnError> {
        Err(BurnError::ToolMissing(
            "backend de grabación no soportado".to_string(),
        ))
    }
    async fn eject(&self, _device: &DeviceId) -> Result<(), BurnError> {
        Err(BurnError::ToolMissing(
            "backend de grabación no soportado".to_string(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(transparent)]
#[specta(transparent)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<&str> for DeviceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OpticalDevice {
    pub id: DeviceId,
    pub vendor: String,
    pub model: String,
    pub bus: String,
    pub writable: bool,
    pub media_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DiscMediaKind {
    DvdR,
    DvdPlusR,
    DvdRw,
    DvdPlusRw,
    DvdRom,
    DvdRam,
    BdR,
    BdRe,
    BdRom,
    Unknown,
}

impl DiscMediaKind {
    pub fn writable(self) -> bool {
        matches!(
            self,
            Self::DvdR | Self::DvdPlusR | Self::DvdRw | Self::DvdPlusRw | Self::BdR | Self::BdRe
        )
    }

    pub fn rewritable(self) -> bool {
        matches!(self, Self::DvdRw | Self::DvdPlusRw | Self::BdRe)
    }

    pub fn read_only(self) -> bool {
        matches!(self, Self::DvdRom | Self::BdRom)
    }

    pub fn is_blu_ray(self) -> bool {
        matches!(self, Self::BdR | Self::BdRe | Self::BdRom)
    }

    /// Bytes per second at 1x for this media family.
    pub fn bytes_per_second_at_1x(self) -> f64 {
        if self.is_blu_ray() {
            BD_1X_BYTES_PER_SECOND
        } else {
            DVD_1X_BYTES_PER_SECOND
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MediaStatus {
    Blank,
    Appendable,
    Complete,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    pub kind: DiscMediaKind,
    pub status: MediaStatus,
    pub capacity_bytes: u64,
    pub free_bytes: u64,
    pub writable: bool,
    pub rewritable: bool,
    pub max_speed_x: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct BurnRequest {
    pub iso_path: PathBuf,
    pub device: DeviceId,
    pub speed_x: Option<u16>,
    pub verify: bool,
    pub eject: bool,
    pub close_session: bool,
    pub simulation: bool,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BurnProgress {
    pub written_bytes: u64,
    pub total_bytes: u64,
    pub percent: f32,
    pub speed_x: Option<f32>,
    pub eta_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum BurnBackend {
    Growisofs,
    Xorriso,
    Drutil,
    Hdiutil,
    Imapi2,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BurnReport {
    pub backend: BurnBackend,
    pub elapsed_ms: u64,
    pub average_speed_x: Option<f32>,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub sectors_checked: u64,
    pub mismatches: u64,
    pub sha256_iso: String,
    pub sha256_disc: String,
}

pub type ProgressSink = mpsc::Sender<BurnProgress>;

#[async_trait::async_trait]
pub trait DiscBurner: Send + Sync {
    fn backend(&self) -> BurnBackend;

    async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError>;

    async fn probe_media(&self, device: &DeviceId) -> Result<MediaInfo, BurnError>;

    async fn burn(
        &self,
        request: BurnRequest,
        progress: ProgressSink,
        cancel: CancellationToken,
    ) -> Result<BurnReport, BurnError>;

    async fn verify(
        &self,
        device: &DeviceId,
        iso: &std::path::Path,
        progress: ProgressSink,
    ) -> Result<VerifyReport, BurnError>;

    async fn eject(&self, device: &DeviceId) -> Result<(), BurnError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BurnError {
    #[error("no se detectaron unidades ópticas")]
    NoDevices,
    #[error("el dispositivo está ocupado o montado")]
    DeviceBusy,
    #[error("no hay medio insertado")]
    NoMedia,
    #[error("medio no soportado: {0}")]
    UnsupportedMedia(String),
    #[error("permiso denegado: {0}")]
    PermissionDenied(String),
    #[error("herramienta no encontrada: {0}")]
    ToolMissing(String),
    #[error("solicitud de grabación inválida: {0}")]
    InvalidRequest(String),
    #[error("{tool} falló con código {code}: {stderr}")]
    ProcessFailed {
        tool: String,
        code: i32,
        stderr: String,
    },
    #[error("grabación cancelada")]
    Cancelled,
    #[error("verificación fallida: ISO {iso} vs disco {disc}")]
    VerifyFailed { iso: String, disc: String },
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("error COM/IMAPI2: {0}")]
    Imapi(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_compares_normalized_devices() {
        let detected = vec!["/dev/sr0".to_string(), "D:".to_string()];
        assert!(device_matches_allowlist(
            " /dev/sr0 ",
            detected.clone(),
            |value| value.trim().to_string()
        ));
        assert!(!device_matches_allowlist("/dev/sr1", detected, |value| {
            value.trim().to_string()
        }));
        assert!(device_matches_allowlist(
            "rdisk3",
            vec!["disk3".to_string()],
            normalize_macos_device_id
        ));
        assert!(device_matches_allowlist(
            "d",
            vec!["D:".to_string()],
            |value| normalize_drive_id(value).unwrap_or_default()
        ));
    }

    #[test]
    fn canonical_device_path_keeps_missing_paths_trimmed() {
        assert_eq!(
            canonical_device_path("  /dev/dvdforge-does-not-exist  "),
            "/dev/dvdforge-does-not-exist"
        );
    }
}
