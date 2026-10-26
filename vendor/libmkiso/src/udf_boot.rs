//! ISO9660 boot-discovery view over payloads in a UDF image.
use crate::el_torito::{ResolvedBootImage, boot_catalog, boot_descriptor};
use crate::{BootOptions, IsoTimestamp};
use anyhow::{Context, Result, ensure};
use std::io::{Seek, SeekFrom, Write};

const BLOCK: usize = 2048;

pub(crate) fn write_bridge(
    output: &mut std::fs::File,
    sectors: u32,
    options: &BootOptions,
    images: &[(String, u32, u64)],
    timestamp: IsoTimestamp,
    label: &str,
) -> Result<()> {
    if options.is_empty() {
        return Ok(());
    }
    write_boot_bridge(
        output,
        sectors,
        label,
        timestamp,
        options
            .bios
            .as_ref()
            .map(|image| resolve(image, images))
            .transpose()?,
        options
            .efi
            .as_ref()
            .map(|image| resolve(image, images))
            .transpose()?,
    )
}

fn resolve<'a>(
    image: &'a crate::BootImage,
    images: &[(String, u32, u64)],
) -> Result<ResolvedBootImage<'a>> {
    let path = image
        .path
        .components()
        .map(|component| {
            if let std::path::Component::Normal(name) = component {
                name.to_str().context("boot image path is not Unicode")
            } else {
                anyhow::bail!("boot image must use normal relative path components")
            }
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    let (_, sector, size) = images
        .iter()
        .find(|(name, _, _)| name == &path)
        .context("boot image must name a contiguous regular file")?;
    let size = u32::try_from(*size).context("boot image exceeds 4 GiB")?;
    image.validate(size)?;
    Ok(ResolvedBootImage {
        image,
        sector: *sector,
        size,
    })
}

fn both16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 2..offset + 4].copy_from_slice(&value.to_be_bytes());
}
fn both32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    bytes[offset + 4..offset + 8].copy_from_slice(&value.to_be_bytes());
}
fn root_record(id: u8, timestamp: IsoTimestamp) -> [u8; 34] {
    let mut bytes = [0; 34];
    bytes[0] = 34;
    both32(&mut bytes, 2, 32);
    both32(&mut bytes, 10, BLOCK as u32);
    bytes[18..25].copy_from_slice(&timestamp.directory_bytes());
    bytes[25] = 2;
    both16(&mut bytes, 28, 1);
    bytes[32] = 1;
    bytes[33] = id;
    bytes
}
fn sector(output: &mut (impl Write + Seek), block: u32, bytes: &[u8; BLOCK]) -> Result<()> {
    output.seek(SeekFrom::Start(u64::from(block) * BLOCK as u64))?;
    output.write_all(bytes)?;
    Ok(())
}

/// The UDF volume recognition sequence must start after sector 18.
pub(crate) fn write_boot_bridge(
    output: &mut (impl Write + Seek),
    sectors: u32,
    label: &str,
    timestamp: IsoTimestamp,
    bios: Option<ResolvedBootImage<'_>>,
    efi: Option<ResolvedBootImage<'_>>,
) -> Result<()> {
    timestamp.validate()?;
    ensure!(sectors > 256, "bootable UDF image is too small");
    let catalog = boot_catalog(bios, efi)?;
    let mut primary = [0; BLOCK];
    primary[0] = 1;
    primary[1..7].copy_from_slice(b"CD001\x01");
    primary[8..72].fill(b' ');
    // UDF labels allow Unicode; this discovery namespace uses a deterministic ASCII alias.
    let alias: Vec<_> = label
        .chars()
        .take(32)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase() as u8
            } else {
                b'_'
            }
        })
        .collect();
    primary[40..40 + alias.len()].copy_from_slice(&alias);
    both32(&mut primary, 80, sectors);
    both16(&mut primary, 120, 1);
    both16(&mut primary, 124, 1);
    both16(&mut primary, 128, BLOCK as u16);
    both32(&mut primary, 132, 10);
    primary[140..144].copy_from_slice(&33u32.to_le_bytes());
    primary[148..152].copy_from_slice(&34u32.to_be_bytes());
    primary[156..190].copy_from_slice(&root_record(0, timestamp));
    primary[190..813].fill(b' ');
    primary[847..863].fill(b'0');
    for offset in [813, 830, 864] {
        primary[offset..offset + 17].copy_from_slice(&timestamp.volume_bytes());
    }
    primary[881] = 1;
    sector(output, 16, &primary)?;
    sector(output, 17, &boot_descriptor(35))?;
    let mut terminator = [0; BLOCK];
    terminator[0] = 255;
    terminator[1..7].copy_from_slice(b"CD001\x01");
    sector(output, 18, &terminator)?;
    let mut directory = [0; BLOCK];
    directory[..34].copy_from_slice(&root_record(0, timestamp));
    directory[34..68].copy_from_slice(&root_record(1, timestamp));
    sector(output, 32, &directory)?;
    for (location, big_endian) in [(33, false), (34, true)] {
        let mut table = [0; BLOCK];
        table[0] = 1;
        table[2..6].copy_from_slice(&if big_endian {
            32u32.to_be_bytes()
        } else {
            32u32.to_le_bytes()
        });
        table[6..8].copy_from_slice(&if big_endian {
            1u16.to_be_bytes()
        } else {
            1u16.to_le_bytes()
        });
        sector(output, location, &table)?;
    }
    sector(output, 35, &catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iso9660::{IsoReader, Limits};

    #[test]
    fn boot_bridge_has_readable_primary_root_and_valid_dual_catalog() {
        let mut output = tempfile::tempfile().unwrap();
        output.set_len(600 * BLOCK as u64).unwrap();
        let options = BootOptions {
            bios: Some(crate::BootImage::bios("boot/bios.bin")),
            efi: Some(crate::BootImage::efi("boot/efi.bin")),
        };
        write_bridge(
            &mut output,
            600,
            &options,
            &[
                ("boot/bios.bin".into(), 400, 4096),
                ("boot/efi.bin".into(), 402, 4096),
            ],
            IsoTimestamp::default(),
            "日本語",
        )
        .unwrap();
        output.seek(SeekFrom::Start(0)).unwrap();
        let reader = IsoReader::open(output.try_clone().unwrap(), Limits::default()).unwrap();
        assert!(reader.entries().is_empty());
        use std::io::Read;
        let mut catalog = [0u8; BLOCK];
        output.seek(SeekFrom::Start(35 * BLOCK as u64)).unwrap();
        output.read_exact(&mut catalog).unwrap();
        let checksum = catalog[..32].chunks_exact(2).fold(0u16, |sum, word| {
            sum.wrapping_add(u16::from_le_bytes(word.try_into().unwrap()))
        });
        assert_eq!(checksum, 0);
        assert_eq!(u32::from_le_bytes(catalog[40..44].try_into().unwrap()), 400);
        assert_eq!(
            u32::from_le_bytes(catalog[104..108].try_into().unwrap()),
            402
        );
    }
}
