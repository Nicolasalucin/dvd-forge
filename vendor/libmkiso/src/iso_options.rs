//! Options for deterministic ISO9660 image creation.
use crate::el_torito::BootOptions;
use crate::iso9660::{Error, Result};

/// ISO9660 interchange level. Level 3 permits files composed of multiple extents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IsoLevel {
    /// DOS-compatible 8.3 file identifiers and eight-character directory names.
    Level1,
    /// Identifiers up to 31 characters and one extent per file.
    #[default]
    Level2,
    /// Level 2 identifiers with multiple extents per file.
    Level3,
}

/// Rock Ridge ownership and permission policy.
#[derive(Debug, Clone)]
pub struct UnixMetadataOptions {
    /// Preserve source mode, uid and gid on Unix hosts. Unsupported on other hosts.
    pub preserve: bool,
    /// Owner used when source ownership is not preserved.
    pub uid: u32,
    /// Group used when source ownership is not preserved.
    pub gid: u32,
    /// Regular-file permissions, without file-type bits.
    pub file_mode: u32,
    /// Directory permissions, without file-type bits.
    pub directory_mode: u32,
}
impl Default for UnixMetadataOptions {
    fn default() -> Self {
        Self {
            preserve: false,
            uid: 0,
            gid: 0,
            file_mode: 0o644,
            directory_mode: 0o755,
        }
    }
}

/// Treatment of source names in the primary ISO9660 namespace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FilenamePolicy {
    /// Uppercase representable names; reject unsupported names and collisions.
    #[default]
    Strict,
    /// Generate deterministic, unique ASCII aliases for unsupported names.
    /// With Joliet enabled, the supplementary namespace retains original names.
    Mangle,
}

/// A fixed UTC timestamp stored in all ISO directory records and volume metadata.
/// ISO9660 directory timestamps represent years from 1900 through 2155.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IsoTimestamp {
    /// Calendar year, from 1900 through 2155.
    pub year: u16,
    /// Calendar month, from 1 through 12.
    pub month: u8,
    /// Day of the month, validated against month and leap year.
    pub day: u8,
    /// Hour, from 0 through 23.
    pub hour: u8,
    /// Minute, from 0 through 59.
    pub minute: u8,
    /// Second, from 0 through 59.
    pub second: u8,
}
impl Default for IsoTimestamp {
    fn default() -> Self {
        Self {
            year: 2000,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}
impl IsoTimestamp {
    pub(crate) fn validate(&self) -> Result<()> {
        let leap = self.year.is_multiple_of(4)
            && (!self.year.is_multiple_of(100) || self.year.is_multiple_of(400));
        let days = match self.month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        if !(1900..=2155).contains(&self.year)
            || self.day == 0
            || self.day > days
            || self.hour > 23
            || self.minute > 59
            || self.second > 59
        {
            return Err(Error::Unsupported("invalid ISO9660 UTC timestamp".into()));
        }
        Ok(())
    }
    pub(crate) fn directory_bytes(&self) -> [u8; 7] {
        [
            (self.year - 1900) as u8,
            self.month,
            self.day,
            self.hour,
            self.minute,
            self.second,
            0,
        ]
    }
    pub(crate) fn volume_bytes(&self) -> [u8; 17] {
        let text = format!(
            "{:04}{:02}{:02}{:02}{:02}{:02}00",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        let mut bytes = [0; 17];
        bytes[..16].copy_from_slice(text.as_bytes());
        bytes
    }
}

/// Settings for the primary ISO9660 filesystem, optional Joliet and boot catalog.
/// Defaults preserve deterministic, strict, non-bootable ISO9660 creation.
#[derive(Debug, Clone)]
pub struct IsoOptions {
    /// Interchange level; defaults to level 2.
    pub level: IsoLevel,
    /// ISO9660 volume identifier: 1–32 uppercase ASCII letters, digits or underscores.
    pub volume_label: String,
    /// Fixed UTC timestamp used independently of source file modification times.
    pub timestamp: IsoTimestamp,
    /// Policy for primary ISO9660 filenames.
    pub filename_policy: FilenamePolicy,
    /// Include a Joliet supplementary filesystem preserving Unicode names.
    pub joliet: bool,
    /// Joliet Unicode conformance level, 1 through 3.
    pub joliet_level: u8,
    /// Maximum UCS-2 characters per Joliet identifier, 1 through 103.
    /// Values above 64 are a compatibility extension rather than standard Joliet.
    pub joliet_max_name: usize,
    /// Include Rock Ridge names, permissions, owners, timestamps and symbolic links.
    pub rock_ridge: bool,
    /// Unix ownership and permission policy for Rock Ridge.
    pub unix_metadata: UnixMetadataOptions,
    /// Maximum directory depth, including the root. Values above 8 relax ISO limits.
    pub max_directory_depth: usize,
    /// Maximum stored primary path length. Values above 255 relax ISO limits.
    pub max_path_bytes: usize,
    /// Maximum file extent size in level 3; must be a nonzero multiple of 2048.
    pub extent_bytes: u32,
    /// Bound tree entries before allocating directory metadata.
    pub max_entries: usize,
    /// Bound generated directory, continuation and path-table metadata.
    pub max_metadata_bytes: usize,
    /// Maximum final image size, including any GPT backup table.
    pub max_image_bytes: u64,
    /// Additional El Torito entries and boot-loader patch options.
    pub advanced_boot: crate::el_torito::AdvancedBootOptions,
    /// Optional MBR/GPT disk layout in the otherwise unused ISO system area.
    pub hybrid: Option<crate::hybrid::HybridOptions>,
    /// Additional volume descriptor text fields.
    pub volume_metadata: VolumeMetadata,
    /// Optional BIOS and EFI no-emulation boot images, relative to the source tree.
    pub boot: BootOptions,
}
impl Default for IsoOptions {
    fn default() -> Self {
        Self {
            level: IsoLevel::default(),
            volume_label: "ISOIMAGE".into(),
            timestamp: IsoTimestamp::default(),
            filename_policy: FilenamePolicy::Strict,
            joliet: false,
            joliet_level: 3,
            joliet_max_name: 64,
            rock_ridge: false,
            unix_metadata: UnixMetadataOptions::default(),
            max_directory_depth: 8,
            max_path_bytes: 255,
            extent_bytes: 0xffff_f800,
            max_entries: 100_000,
            max_metadata_bytes: 16 << 20,
            max_image_bytes: 8 << 40,
            advanced_boot: Default::default(),
            hybrid: None,
            volume_metadata: Default::default(),
            boot: BootOptions::default(),
        }
    }
}
impl IsoOptions {
    pub(crate) fn validate(&self) -> Result<()> {
        if !(1..=3).contains(&self.joliet_level)
            || !(1..=103).contains(&self.joliet_max_name)
            || self.extent_bytes == 0
            || !self.extent_bytes.is_multiple_of(2048)
            || self.max_directory_depth == 0
            || self.max_directory_depth > 256
            || self.max_path_bytes == 0
            || self.max_entries == 0
            || self.unix_metadata.file_mode & !0o7777 != 0
            || self.unix_metadata.directory_mode & !0o7777 != 0
        {
            return Err(Error::Unsupported(
                "invalid ISO authoring limits or profile".into(),
            ));
        }
        if self.unix_metadata.preserve && !cfg!(unix) {
            return Err(Error::Unsupported(
                "preserving Unix metadata requires a Unix host".into(),
            ));
        }
        self.volume_metadata.validate()?;
        if self.joliet
            && [
                &self.volume_metadata.system_id,
                &self.volume_metadata.volume_set_id,
                &self.volume_metadata.publisher,
                &self.volume_metadata.preparer,
                &self.volume_metadata.application,
            ]
            .into_iter()
            .zip([16, 64, 64, 64, 64])
            .any(|(value, maximum)| value.len() > maximum)
        {
            return Err(Error::Unsupported(
                "volume metadata exceeds Joliet UCS-2 field capacity".into(),
            ));
        }
        if self.volume_label.is_empty()
            || self.volume_label.len() > 32
            || !self
                .volume_label
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(Error::Unsupported(
                "ISO9660 volume label requires 1–32 uppercase ASCII letters, digits or underscores"
                    .into(),
            ));
        }
        if self.joliet && self.volume_label.len() > 16 {
            return Err(Error::Unsupported(
                "Joliet volume label is limited to 16 UCS-2 characters".into(),
            ));
        }
        self.timestamp.validate()
    }
}

/// Optional ASCII identifiers in the ISO volume descriptors.
#[derive(Debug, Clone, Default)]
pub struct VolumeMetadata {
    /// System identifier (32 characters).
    pub system_id: String,
    /// Volume set identifier (128 characters).
    pub volume_set_id: String,
    /// Publisher identifier (128 characters).
    pub publisher: String,
    /// Data preparer identifier (128 characters).
    pub preparer: String,
    /// Application identifier (128 characters).
    pub application: String,
}
impl VolumeMetadata {
    fn validate(&self) -> Result<()> {
        for (value, maximum) in [
            (&self.system_id, 32),
            (&self.volume_set_id, 128),
            (&self.publisher, 128),
            (&self.preparer, 128),
            (&self.application, 128),
        ] {
            if value.len() > maximum || !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                return Err(Error::Unsupported(
                    "volume metadata must fit its printable ASCII field".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_validate_calendar_and_directory_year_bounds() {
        for year in [1900, 2100] {
            assert!(
                IsoTimestamp {
                    year,
                    month: 2,
                    day: 29,
                    ..IsoTimestamp::default()
                }
                .validate()
                .is_err()
            );
        }
        for year in [2000, 2024] {
            assert!(
                IsoTimestamp {
                    year,
                    month: 2,
                    day: 29,
                    ..IsoTimestamp::default()
                }
                .validate()
                .is_ok()
            );
        }
        for value in [
            IsoTimestamp {
                year: 1899,
                ..IsoTimestamp::default()
            },
            IsoTimestamp {
                year: 2156,
                ..IsoTimestamp::default()
            },
            IsoTimestamp {
                month: 0,
                ..IsoTimestamp::default()
            },
            IsoTimestamp {
                month: 4,
                day: 31,
                ..IsoTimestamp::default()
            },
            IsoTimestamp {
                minute: 60,
                ..IsoTimestamp::default()
            },
        ] {
            assert!(value.validate().is_err());
        }
    }
    #[test]
    fn fixed_timestamp_has_matching_directory_and_volume_encodings() {
        let value = IsoTimestamp {
            year: 2026,
            month: 10,
            day: 5,
            hour: 12,
            minute: 34,
            second: 56,
        };
        assert_eq!(value.directory_bytes(), [126, 10, 5, 12, 34, 56, 0]);
        assert_eq!(&value.volume_bytes(), b"2026100512345600\0");
    }
}
