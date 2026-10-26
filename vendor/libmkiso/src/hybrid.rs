//! Deterministic ISO system-area partition layouts, using 512-byte disk sectors.
use crate::iso9660::{Error, Result};
use std::path::PathBuf;

/// Disk partition tables in addition to the optical filesystem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum HybridLayout {
    /// An ISO-covering MBR partition, optionally with an overlapping EFI partition.
    #[default]
    Mbr,
    /// A protective MBR and primary/backup GPT exposing the embedded EFI partition.
    Gpt,
    /// GPT plus a redundant EFI MBR entry for older firmware (hybrid MBR).
    MbrGpt,
}
/// Optional address patch for caller-supplied MBR boot code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MbrBootPatch {
    /// Leave boot code unchanged.
    #[default]
    None,
    /// Write the BIOS image's 512-byte LBA at offset 432 (SYSLINUX isohybrid).
    Syslinux,
    /// Write BIOS image LBA plus four at offset 432 (GRUB2 convention).
    Grub2,
}
/// Configure a disk-bootable hybrid image without creating or installing a loader.
#[derive(Debug, Clone)]
pub struct HybridOptions {
    /// Partition table layout.
    pub layout: HybridLayout,
    /// Caller-supplied MBR bootstrap bytes, at most 440 bytes.
    pub mbr_boot_code: Vec<u8>,
    /// Loader-specific address patch; requires a bootable x86 El Torito entry.
    pub mbr_patch: MbrBootPatch,
    /// Reproducible MBR disk signature.
    pub disk_signature: u32,
    /// MBR type of the ISO-covering partition (MBR-only layout).
    pub iso_partition_type: u8,
    /// Mark the ISO-covering MBR partition active.
    pub active: bool,
    /// EFI FAT image relative to the source tree, exposed without copying payload.
    pub efi_partition: Option<PathBuf>,
    /// GPT disk GUID in on-disk byte order; must be nonzero.
    pub disk_guid: [u8; 16],
    /// EFI partition GUID in on-disk byte order; must be nonzero and distinct.
    pub efi_guid: [u8; 16],
}
impl Default for HybridOptions {
    fn default() -> Self {
        Self {
            layout: HybridLayout::Mbr,
            mbr_boot_code: Vec::new(),
            mbr_patch: MbrBootPatch::None,
            disk_signature: 0x49534f31,
            iso_partition_type: 0x17,
            active: true,
            efi_partition: None,
            disk_guid: *b"ISODISK_GUID_001",
            efi_guid: *b"ISOEFI_GUID_0001",
        }
    }
}
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
fn partition(bytes: &mut [u8], active: bool, kind: u8, start: u32, count: u32) {
    bytes[0] = if active { 0x80 } else { 0 };
    bytes[1..4].copy_from_slice(&[0xfe, 0xff, 0xff]);
    bytes[4] = kind;
    bytes[5..8].copy_from_slice(&[0xfe, 0xff, 0xff]);
    bytes[8..12].copy_from_slice(&start.to_le_bytes());
    bytes[12..16].copy_from_slice(&count.to_le_bytes());
}
fn header(
    options: &HybridOptions,
    current: u64,
    alternate: u64,
    array: u64,
    last: u64,
    array_crc: u32,
) -> [u8; 512] {
    let mut bytes = [0; 512];
    bytes[..8].copy_from_slice(b"EFI PART");
    bytes[8..12].copy_from_slice(&0x10000u32.to_le_bytes());
    bytes[12..16].copy_from_slice(&92u32.to_le_bytes());
    for (offset, value) in [
        (24, current),
        (32, alternate),
        (40, 34),
        (48, last - 33),
        (72, array),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[56..72].copy_from_slice(&options.disk_guid);
    bytes[80..84].copy_from_slice(&128u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&128u32.to_le_bytes());
    bytes[88..92].copy_from_slice(&array_crc.to_le_bytes());
    let crc = crc32(&bytes[..92]);
    bytes[16..20].copy_from_slice(&crc.to_le_bytes());
    bytes
}
/// System area and appended backup table; GPT's backup header is the final disk sector.
pub(crate) fn build(
    options: &HybridOptions,
    iso_blocks: u32,
    efi: Option<(u32, u64)>,
    bios: Option<u32>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let fail = || Error::Unsupported("invalid hybrid boot layout".into());
    let gpt = options.layout != HybridLayout::Mbr;
    if options.mbr_boot_code.len() > 440
        || options.iso_partition_type == 0
        || (options.mbr_patch != MbrBootPatch::None
            && (options.mbr_boot_code.len() < 432 || bios.is_none()))
        || (gpt
            && (efi.is_none()
                || options.disk_guid == [0; 16]
                || options.efi_guid == [0; 16]
                || options.disk_guid == options.efi_guid))
    {
        return Err(fail());
    }
    let extra = if gpt { 36u64 } else { 0 }; // 33 sectors for GPT, plus 3 for ISO block alignment.
    let count = u64::from(iso_blocks) * 4 + extra;
    let count32 = u32::try_from(count).map_err(|_| fail())?;
    let mut system = vec![0; 32768];
    system[..options.mbr_boot_code.len()].copy_from_slice(&options.mbr_boot_code);
    if options.mbr_patch != MbrBootPatch::None {
        let lba = u64::from(bios.ok_or_else(fail)?) * 4
            + if options.mbr_patch == MbrBootPatch::Grub2 {
                4
            } else {
                0
            };
        system[432..440].copy_from_slice(&lba.to_le_bytes());
    }
    system[440..444].copy_from_slice(&options.disk_signature.to_le_bytes());
    system[510..512].copy_from_slice(&[0x55, 0xaa]);
    if gpt {
        partition(&mut system[446..462], false, 0xee, 1, count32 - 1);
        system[447..450].copy_from_slice(&[0, 2, 0]);
    } else {
        partition(
            &mut system[446..462],
            options.active,
            options.iso_partition_type,
            0,
            count32,
        );
    }
    let efi = efi
        .map(|(block, size)| {
            if size == 0 || !size.is_multiple_of(512) {
                return Err(fail());
            }
            let start = block.checked_mul(4).ok_or_else(fail)?;
            let length = u32::try_from(size / 512).map_err(|_| fail())?;
            if start < 64 || u64::from(start) + u64::from(length) > u64::from(iso_blocks) * 4 {
                return Err(fail());
            }
            Ok((start, length))
        })
        .transpose()?;
    if options.layout != HybridLayout::Gpt
        && let Some((start, length)) = efi
    {
        partition(&mut system[462..478], false, 0xef, start, length);
    }
    if !gpt {
        return Ok((system, Vec::new()));
    }
    let (start, length) = efi.ok_or_else(fail)?;
    let mut array = vec![0; 16384];
    array[..16].copy_from_slice(&[
        0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b,
    ]);
    array[16..32].copy_from_slice(&options.efi_guid);
    array[32..40].copy_from_slice(&u64::from(start).to_le_bytes());
    array[40..48].copy_from_slice(&(u64::from(start) + u64::from(length) - 1).to_le_bytes());
    for (pair, unit) in array[56..128]
        .chunks_exact_mut(2)
        .zip("EFI System Partition".encode_utf16())
    {
        pair.copy_from_slice(&unit.to_le_bytes());
    }
    let crc = crc32(&array);
    let last = count - 1;
    system[512..1024].copy_from_slice(&header(options, 1, last, 2, last, crc));
    system[1024..17408].copy_from_slice(&array);
    let mut tail = vec![0; 3 * 512];
    tail.extend(array);
    tail.extend(header(options, last, 1, last - 32, last, crc));
    Ok((system, tail))
}
