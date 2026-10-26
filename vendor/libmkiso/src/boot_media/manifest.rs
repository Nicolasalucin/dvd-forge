//! Version-one TOML manifests. Media entry paths are relative to the source tree;
//! source, output, ISO and asset paths are relative to the manifest directory.
use crate::boot_media::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

/// Output media kind; optical and USB layouts are distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaTarget {
    Optical,
    Usb,
}
/// Filesystem requested by a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Filesystem {
    #[serde(rename = "iso9660")]
    Iso9660,
    #[serde(rename = "udf")]
    Udf,
    #[serde(rename = "iso9660+udf")]
    Bridge,
}
/// Single optical image manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub image: Image,
    pub filesystem: FilesystemOptions,
    #[serde(default)]
    pub boot: Option<Boot>,
    #[serde(default)]
    pub windows: Option<Windows>,
    #[serde(default)]
    pub reproducibility: Option<Reproducibility>,
}
/// Source and destination values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub source: PathBuf,
    pub output: PathBuf,
    #[serde(default = "default_label")]
    pub label: String,
    #[serde(default = "optical_media")]
    pub media: Vec<MediaTarget>,
}
fn default_label() -> String {
    "ISOIMAGE".into()
}
fn optical_media() -> Vec<MediaTarget> {
    vec![MediaTarget::Optical]
}
/// Filesystem policy; unsupported revisions fail during planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemOptions {
    #[serde(rename = "type")]
    pub kind: Filesystem,
    #[serde(default)]
    pub udf_revision: Option<String>,
    #[serde(default)]
    pub joliet: bool,
    #[serde(default)]
    pub rock_ridge: bool,
}
/// Explicit architecture and optical boot entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Boot {
    pub profile: String,
    pub arch: String,
    #[serde(default)]
    pub assets: Option<PathBuf>,
    #[serde(default)]
    pub entries: Vec<BootEntry>,
}
/// A source-relative optical boot asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootEntry {
    pub id: String,
    pub firmware: Firmware,
    pub image: PathBuf,
}
/// Firmware capability, independent of installation results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Firmware {
    Bios,
    Uefi,
}
/// Source-relative Windows payload locations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Windows {
    pub boot_wim: PathBuf,
    pub install_image: PathBuf,
}
/// Deterministic build inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reproducibility {
    pub timestamp: String,
}
/// Separate-ISO multiboot manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultibootManifest {
    pub version: u32,
    pub menu: Menu,
    pub boot: MultibootBoot,
    #[serde(default)]
    pub entries: Vec<MultibootEntry>,
}
/// Menu policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    pub title: String,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default = "default_timeout")]
    /// Zero disables automatic boot and waits for a manual selection.
    pub timeout_seconds: u32,
}
fn default_timeout() -> u32 {
    15
}
/// An explicitly supplied runtime bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultibootBoot {
    pub backend: String,
    pub firmware: Vec<Firmware>,
    pub assets: PathBuf,
}
/// An original image kept separate from other installers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MultibootEntry {
    pub id: String,
    pub title: String,
    pub image: PathBuf,
}

fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 1024 * 1024 {
        return Err(Error::Resource("manifest exceeds 1 MiB".into()));
    }
    use std::io::Read;
    let mut content = String::new();
    file.take(1024 * 1024 + 1).read_to_string(&mut content)?;
    if content.len() > 1024 * 1024 {
        return Err(Error::Resource("manifest exceeds 1 MiB".into()));
    }
    toml::from_str(&content).map_err(|e| Error::InvalidInput(format!("{}: {e}", path.display())))
}
/// Load and validate a single-image manifest without writing media.
pub fn read_manifest(path: &Path) -> Result<Manifest> {
    let manifest: Manifest = load(path)?;
    manifest.validate()?;
    Ok(manifest)
}
/// Load and validate a multiboot manifest without provisioning devices.
pub fn read_multiboot(path: &Path) -> Result<MultibootManifest> {
    let manifest: MultibootManifest = load(path)?;
    manifest.validate()?;
    Ok(manifest)
}
/// Validate paths that must stay inside the source media tree.
pub fn source_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(Error::InvalidInput(format!(
            "{} must be relative to the source media tree, without traversal",
            path.display()
        )));
    }
    Ok(())
}
fn version_one(version: u32) -> Result<()> {
    if version != 1 {
        return Err(Error::InvalidInput(format!(
            "unsupported manifest version {version}; expected 1"
        )));
    }
    Ok(())
}
fn ids<'a>(values: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in values {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::InvalidInput(format!(
                "invalid entry identifier {id:?}"
            )));
        }
        if !seen.insert(id) {
            return Err(Error::InvalidInput(format!(
                "duplicate entry identifier {id}"
            )));
        }
    }
    Ok(())
}
impl Manifest {
    /// Reject schema inconsistencies independently of available backends.
    pub fn validate(&self) -> Result<()> {
        version_one(self.version)?;
        if self.image.source.as_os_str().is_empty() || self.image.output.as_os_str().is_empty() {
            return Err(Error::InvalidInput(
                "source and output paths are required".into(),
            ));
        }
        if self.image.media.is_empty() {
            return Err(Error::InvalidInput(
                "image.media must contain a target".into(),
            ));
        }
        if self.image.media.len() > 2
            || (self.image.media.len() == 2 && self.image.media[0] == self.image.media[1])
        {
            return Err(Error::InvalidInput("duplicate media target".into()));
        }
        if self.image.label.is_empty()
            || self.image.label.len() > 32
            || !self
                .image
                .label
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::InvalidInput(
                "image label requires 1..32 uppercase ASCII letters, digits or underscores".into(),
            ));
        }
        if let Some(boot) = &self.boot {
            ids(boot.entries.iter().map(|e| e.id.as_str()))?;
            if !matches!(boot.arch.as_str(), "x86_64" | "arm64") {
                return Err(Error::InvalidInput(format!(
                    "unknown architecture {}",
                    boot.arch
                )));
            }
            for entry in &boot.entries {
                source_relative(&entry.image)?;
                if boot.arch == "arm64" && entry.firmware == Firmware::Bios {
                    return Err(Error::Unsupported(format!(
                        "entry {}: arm64 BIOS boot",
                        entry.id
                    )));
                }
            }
        }
        if let Some(windows) = &self.windows {
            source_relative(&windows.boot_wim)?;
            source_relative(&windows.install_image)?;
        }
        if let Some(repro) = &self.reproducibility {
            validate_timestamp(&repro.timestamp)?;
        }
        Ok(())
    }
}
/// Parse an ISO-representable UTC RFC3339 timestamp, rejecting lost precision.
pub fn validate_timestamp(value: &str) -> Result<time::OffsetDateTime> {
    let timestamp =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .map_err(|e| Error::InvalidInput(format!("timestamp: {e}")))?
            .to_offset(time::UtcOffset::UTC);
    if !(1900..=2155).contains(&timestamp.year()) || timestamp.nanosecond() != 0 {
        return Err(Error::InvalidInput(
            "timestamp must have whole-second precision and UTC year 1900..2155".into(),
        ));
    }
    Ok(timestamp)
}
impl MultibootManifest {
    /// Validate menu identifiers and backend policy before inventory.
    pub fn validate(&self) -> Result<()> {
        version_one(self.version)?;
        ids(self.entries.iter().map(|e| e.id.as_str()))?;
        if self.entries.len() > 4096 {
            return Err(Error::Resource("multiboot entry count exceeds 4096".into()));
        }
        if let Some(default) = &self.menu.default
            && !self.entries.iter().any(|e| &e.id == default)
        {
            return Err(Error::InvalidInput(format!(
                "menu default {default} does not identify an entry"
            )));
        }
        if self.boot.firmware.is_empty() {
            return Err(Error::InvalidInput("boot.firmware is empty".into()));
        }
        if self.boot.firmware.len() > 2
            || (self.boot.firmware.len() == 2 && self.boot.firmware[0] == self.boot.firmware[1])
        {
            return Err(Error::InvalidInput("duplicate firmware target".into()));
        }
        if !matches!(self.boot.backend.as_str(), "ventoy" | "grub") {
            return Err(Error::Unsupported(format!(
                "unknown multiboot backend {}",
                self.boot.backend
            )));
        }
        Ok(())
    }
}
