//! El Torito platform catalogs, firmware emulation and boot-image configuration.
use crate::iso9660::{Error, Result};
use std::path::{Component, PathBuf};

/// A caller-supplied boot image stored as a regular file in the source tree.
/// EFI images must contain an EFI system partition; this crate does not create
/// boot loaders or validate the embedded filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootImage {
    /// Relative source path containing only normal path components.
    pub path: PathBuf,
    /// BIOS load segment (zero requests the firmware default, 0x07c0).
    pub load_segment: u16,
    /// Number of 512-byte sectors to load. For EFI, count one means the partition
    /// extends from the image's beginning to the end of the optical device,
    /// as specified by UEFI section 13.3.2.
    pub load_sectors: u16,
}

impl BootImage {
    /// Configure a BIOS no-emulation image, initially loading 2048 bytes.
    pub fn bios(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            load_segment: 0,
            load_sectors: 4,
        }
    }

    /// Configure an EFI no-emulation system-partition image.
    pub fn efi(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            load_segment: 0,
            load_sectors: 1,
        }
    }

    pub(crate) fn validate(&self, size: u32) -> Result<()> {
        if self.path.as_os_str().is_empty()
            || self
                .path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Error::Unsupported(
                "boot image path must be a normal relative source path".into(),
            ));
        }
        if size == 0 || !size.is_multiple_of(512) {
            return Err(Error::Unsupported(
                "boot image must be nonempty and 512-byte aligned".into(),
            ));
        }
        if self.load_sectors == 0 || u32::from(self.load_sectors) * 512 > size {
            return Err(Error::Unsupported(
                "boot image load count must be nonzero and fit the image".into(),
            ));
        }
        Ok(())
    }
}

/// Optional BIOS and EFI boot images, using no-emulation El Torito entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootOptions {
    /// Legacy x86 BIOS boot image.
    pub bios: Option<BootImage>,
    /// UEFI system-partition image (platform ID 0xef).
    pub efi: Option<BootImage>,
}

impl BootOptions {
    /// Whether no boot images have been selected.
    pub fn is_empty(&self) -> bool {
        self.bios.is_none() && self.efi.is_none()
    }
}

pub(crate) struct ResolvedBootImage<'a> {
    pub image: &'a BootImage,
    pub sector: u32,
    pub size: u32,
}

pub(crate) fn boot_descriptor(catalog_sector: u32) -> [u8; 2048] {
    let mut bytes = [0; 2048];
    bytes[1..7].copy_from_slice(b"CD001\x01");
    bytes[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
    bytes[71..75].copy_from_slice(&catalog_sector.to_le_bytes());
    bytes
}

pub(crate) fn boot_catalog(
    bios: Option<ResolvedBootImage<'_>>,
    efi: Option<ResolvedBootImage<'_>>,
) -> Result<[u8; 2048]> {
    let first = bios
        .as_ref()
        .or(efi.as_ref())
        .ok_or_else(|| Error::Unsupported("empty boot catalog".into()))?;
    let mut bytes = [0; 2048];
    bytes[0] = 1;
    bytes[1] = if bios.is_some() { 0 } else { 0xef };
    bytes[30..32].copy_from_slice(&[0x55, 0xaa]);
    let sum = bytes[..32].chunks_exact(2).fold(0u16, |sum, word| {
        sum.wrapping_add(u16::from_le_bytes([word[0], word[1]]))
    });
    bytes[28..30].copy_from_slice(&sum.wrapping_neg().to_le_bytes());
    boot_entry(&mut bytes[32..64], first)?;
    if bios.is_some()
        && let Some(efi) = efi.as_ref()
    {
        bytes[64] = 0x91;
        bytes[65] = 0xef;
        bytes[66..68].copy_from_slice(&1u16.to_le_bytes());
        boot_entry(&mut bytes[96..128], efi)?;
    }
    Ok(bytes)
}

fn boot_entry(bytes: &mut [u8], resolved: &ResolvedBootImage<'_>) -> Result<()> {
    resolved.image.validate(resolved.size)?;
    bytes[0] = 0x88;
    bytes[2..4].copy_from_slice(&resolved.image.load_segment.to_le_bytes());
    bytes[6..8].copy_from_slice(&resolved.image.load_sectors.to_le_bytes());
    bytes[8..12].copy_from_slice(&resolved.sector.to_le_bytes());
    Ok(())
}

/// Firmware emulation requested by an El Torito entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BootEmulation {
    /// Load the image directly.
    #[default]
    None = 0,
    /// A 1.2 MiB floppy image.
    Floppy1200 = 1,
    /// A 1.44 MiB floppy image.
    Floppy1440 = 2,
    /// A 2.88 MiB floppy image.
    Floppy2880 = 3,
    /// A partitioned hard-disk image.
    HardDisk = 4,
}
/// One explicit El Torito platform entry.
#[derive(Debug, Clone)]
pub struct BootEntry {
    /// Source image, load segment and load-sector count.
    pub image: BootImage,
    /// Platform identifier: 0 (x86), 1 (PowerPC), 2 (Mac) or 0xef (EFI).
    pub platform: u8,
    /// Firmware emulation type. EFI requires no emulation.
    pub emulation: BootEmulation,
    /// Whether firmware may boot this entry.
    pub bootable: bool,
    /// Partition system type for hard-disk emulation; zero for other types.
    pub system_type: u8,
    /// Vendor selection bytes for section entries; the default entry requires zeros.
    pub selection: [u8; 20],
    /// Patch the boot-info table at bytes 8–63 in the output copy.
    pub boot_info_table: bool,
    /// Patch GRUB2's disk address at bytes 2548–2555 in the output copy.
    pub grub2_boot_info: bool,
}
impl BootEntry {
    /// A bootable x86 entry with no emulation or address patches.
    pub fn bios(path: impl Into<PathBuf>) -> Self {
        Self {
            image: BootImage::bios(path),
            platform: 0,
            emulation: BootEmulation::None,
            bootable: true,
            system_type: 0,
            selection: [0; 20],
            boot_info_table: false,
            grub2_boot_info: false,
        }
    }
    /// A bootable EFI entry with no emulation.
    pub fn efi(path: impl Into<PathBuf>) -> Self {
        Self {
            image: BootImage::efi(path),
            platform: 0xef,
            ..Self::bios(PathBuf::new())
        }
    }
    pub(crate) fn validate(&self, size: u32) -> Result<()> {
        self.image.validate(size)?;
        let expected = match self.emulation {
            BootEmulation::Floppy1200 => Some(1200 * 1024),
            BootEmulation::Floppy1440 => Some(1440 * 1024),
            BootEmulation::Floppy2880 => Some(2880 * 1024),
            _ => None,
        };
        if !matches!(self.platform, 0 | 1 | 2 | 0xef)
            || (self.platform == 0xef && self.emulation != BootEmulation::None)
            || expected.is_some_and(|value| value != size)
            || (self.emulation != BootEmulation::HardDisk && self.system_type != 0)
            || (self.boot_info_table && size < 64)
            || (self.grub2_boot_info && size < 2556)
        {
            return Err(Error::Unsupported(
                "invalid El Torito platform, emulation or patch profile".into(),
            ));
        }
        Ok(())
    }
}
/// Extended ISO boot settings; legacy `BootOptions` remains unchanged.
#[derive(Debug, Clone, Default)]
pub struct AdvancedBootOptions {
    /// Entries appended after any legacy BIOS/EFI entries. At most 31 total entries.
    pub entries: Vec<BootEntry>,
    /// ASCII manufacturer/developer identifier, at most 24 characters.
    pub catalog_id: String,
}
pub(crate) fn advanced_catalog(entries: &[(BootEntry, u32, u32)], id: &str) -> Result<[u8; 2048]> {
    if entries.is_empty()
        || entries.len() > 31
        || id.len() > 24
        || !id.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(Error::Unsupported(
            "invalid boot catalog entry count or identifier".into(),
        ));
    }
    if entries[0].0.selection != [0; 20] {
        return Err(Error::Unsupported(
            "default boot entry cannot contain selection criteria".into(),
        ));
    }
    let mut bytes = [0; 2048];
    bytes[0] = 1;
    bytes[1] = entries[0].0.platform;
    bytes[4..4 + id.len()].copy_from_slice(id.as_bytes());
    bytes[30..32].copy_from_slice(&[0x55, 0xaa]);
    let sum = bytes[..32].chunks_exact(2).fold(0u16, |sum, w| {
        sum.wrapping_add(u16::from_le_bytes([w[0], w[1]]))
    });
    bytes[28..30].copy_from_slice(&sum.wrapping_neg().to_le_bytes());
    for (index, (entry, sector, size)) in entries.iter().enumerate() {
        entry.validate(*size)?;
        let offset = if index == 0 {
            32
        } else {
            let header = 64 + (index - 1) * 64;
            bytes[header] = if index + 1 == entries.len() {
                0x91
            } else {
                0x90
            };
            bytes[header + 1] = entry.platform;
            bytes[header + 2] = 1;
            header + 32
        };
        bytes[offset] = if entry.bootable { 0x88 } else { 0 };
        bytes[offset + 1] = entry.emulation as u8;
        bytes[offset + 2..offset + 4].copy_from_slice(&entry.image.load_segment.to_le_bytes());
        bytes[offset + 4] = entry.system_type;
        bytes[offset + 6..offset + 8].copy_from_slice(&entry.image.load_sectors.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&sector.to_le_bytes());
        if index != 0 {
            bytes[offset + 12..offset + 32].copy_from_slice(&entry.selection);
        }
    }
    Ok(bytes)
}
