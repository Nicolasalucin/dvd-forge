//! UDF volume descriptors and partition bootstrap metadata.
use super::{
    BLOCK, IcbStrategy, PART, UdfOptions, UdfPartition, UdfRevision, chars, dstring, long_ad,
    put16, put32, put64, reg, tag, timestamp,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
};

pub(super) struct VolumeLayout {
    pub partition_blocks: u32,
    pub files: u32,
    pub directories: u32,
    pub namespace_partition: u16,
    pub metadata: Option<MetadataLayout>,
    pub vat: Option<VatLayout>,
    pub bitmap: Option<u32>,
    pub bitmap_blocks: u32,
    pub payload_partition_start: Option<u32>,
    pub spare_packets: Vec<(u32, u32)>,
}
pub(super) struct MetadataLayout {
    pub primary_icb: u32,
    pub mirror_icb: u32,
    pub primary_data: u32,
    pub mirror_data: u32,
    pub blocks: u32,
    pub duplicated: bool,
    pub extent_blocks: u32,
    pub bitmap_icb: Option<u32>,
    pub bitmap_data: u32,
}
pub(super) struct VatLayout {
    pub entries: u32,
    pub data: u32,
    pub icb: u32,
    pub mapped_start: u32,
}
fn sector(output: &mut File, location: u32, data: &[u8]) -> Result<()> {
    output.seek(SeekFrom::Start(u64::from(location) * BLOCK))?;
    output.write_all(data)?;
    Ok(())
}
fn physical_map() -> [u8; 6] {
    let mut bytes = [1, 6, 0, 0, 0, 0];
    put16(&mut bytes, 2, 1);
    bytes
}
fn special_map(options: &UdfOptions, id: &[u8]) -> [u8; 64] {
    let mut bytes = [0; 64];
    bytes[0] = 2;
    bytes[1] = 64;
    reg(&mut bytes, 4, id, Some(options.revision));
    put16(&mut bytes, 36, 1);
    bytes
}
pub(super) fn sparing_locations(layout: &VolumeLayout) -> [u32; 2] {
    [
        288,
        (PART + layout.partition_blocks).next_multiple_of(32) + 32,
    ]
}
fn sparable_map(options: &UdfOptions, layout: &VolumeLayout, packet_blocks: u16) -> [u8; 64] {
    let mut map = special_map(options, b"*UDF Sparable Partition");
    put16(&mut map, 40, packet_blocks);
    map[42] = 2;
    put32(&mut map, 44, 56 + 8 * layout.spare_packets.len() as u32);
    let locations = sparing_locations(layout);
    put32(&mut map, 48, locations[0]);
    put32(&mut map, 52, locations[1]);
    map
}
fn maps(options: &UdfOptions, layout: &VolumeLayout) -> Result<Vec<u8>> {
    ensure!(
        layout.spare_packets.len() <= 8185,
        "sparing table exceeds reserved extent"
    );
    let mut result = Vec::new();
    if layout.payload_partition_start.is_some() {
        result.extend(physical_map());
        let mut second = physical_map();
        put16(&mut second, 4, 1);
        result.extend(second);
        return Ok(result);
    }
    match options.partition {
        UdfPartition::Physical | UdfPartition::PhysicalSplit => result.extend(physical_map()),
        UdfPartition::Virtual => {
            result.extend(physical_map());
            result.extend(special_map(options, b"*UDF Virtual Partition"));
        }
        UdfPartition::Metadata { .. } | UdfPartition::MetadataSparable { .. } => {
            if let UdfPartition::MetadataSparable { packet_blocks, .. } = options.partition {
                result.extend(sparable_map(options, layout, packet_blocks));
            } else {
                result.extend(physical_map());
            }
            let metadata = layout
                .metadata
                .as_ref()
                .context("missing metadata layout")?;
            let mut map = special_map(options, b"*UDF Metadata Partition");
            put32(&mut map, 40, metadata.primary_icb);
            put32(&mut map, 44, metadata.mirror_icb);
            put32(&mut map, 48, metadata.bitmap_icb.unwrap_or(u32::MAX));
            put32(&mut map, 52, 32);
            put16(&mut map, 56, 32);
            map[58] = u8::from(metadata.duplicated);
            result.extend(map);
        }
        UdfPartition::Sparable { packet_blocks } => {
            result.extend(sparable_map(options, layout, packet_blocks));
        }
    }
    Ok(result)
}
fn descriptor(
    options: &UdfOptions,
    layout: &VolumeLayout,
    id: u16,
    location: u32,
) -> Result<[u8; 2048]> {
    let mut bytes = [0; 2048];
    let length = match id {
        1 => {
            put32(&mut bytes, 16, 1);
            dstring(&mut bytes, 24, 32, &options.label)?;
            put16(&mut bytes, 56, 1);
            put16(&mut bytes, 58, 1);
            put16(&mut bytes, 60, 2);
            put16(&mut bytes, 62, 3);
            put32(&mut bytes, 64, 1);
            put32(&mut bytes, 68, 1);
            dstring(
                &mut bytes,
                72,
                128,
                &format!("0000000000000000{}", options.label),
            )?;
            chars(&mut bytes, 200);
            chars(&mut bytes, 264);
            timestamp(&mut bytes, 376, options.timestamp);
            reg(&mut bytes, 388, b"*libmkiso", None);
            512
        }
        4 => {
            put32(&mut bytes, 16, 2);
            reg(&mut bytes, 20, b"*UDF LV Info", Some(options.revision));
            chars(&mut bytes, 52);
            dstring(&mut bytes, 116, 128, &options.label)?;
            reg(&mut bytes, 352, b"*libmkiso", None);
            512
        }
        5 => {
            put32(&mut bytes, 16, 3);
            put16(&mut bytes, 20, 1);
            reg(
                &mut bytes,
                24,
                if options.revision.number() >= 0x200 {
                    b"+NSR03"
                } else {
                    b"+NSR02"
                },
                None,
            );
            let access = match options.partition {
                UdfPartition::Virtual => 2,
                UdfPartition::Sparable { .. } => 3,
                UdfPartition::MetadataSparable { .. } => 4,
                _ if options.icb_strategy == IcbStrategy::Strategy4096
                    || options.file_set_descriptors > 1 =>
                {
                    2
                }
                _ => 1,
            };
            put32(&mut bytes, 184, access);
            put32(&mut bytes, 188, PART);
            put32(
                &mut bytes,
                192,
                layout
                    .payload_partition_start
                    .unwrap_or(layout.partition_blocks),
            );
            if let Some(bitmap) = layout.bitmap {
                put32(&mut bytes, 64, layout.bitmap_blocks * BLOCK as u32);
                put32(&mut bytes, 68, bitmap);
            }
            reg(&mut bytes, 196, b"*libmkiso", None);
            512
        }
        6 => {
            put32(&mut bytes, 16, 4);
            chars(&mut bytes, 20);
            dstring(&mut bytes, 84, 128, &options.label)?;
            put32(&mut bytes, 212, BLOCK as u32);
            reg(
                &mut bytes,
                216,
                b"*OSTA UDF Compliant",
                Some(options.revision),
            );
            long_ad(&mut bytes, 248, BLOCK as u32, 0, layout.namespace_partition);
            let maps = maps(options, layout)?;
            put32(&mut bytes, 264, maps.len() as u32);
            put32(
                &mut bytes,
                268,
                if layout.namespace_partition == 1 || layout.payload_partition_start.is_some() {
                    2
                } else {
                    1
                },
            );
            reg(&mut bytes, 272, b"*libmkiso", None);
            put32(&mut bytes, 432, 4 * BLOCK as u32);
            put32(&mut bytes, 436, 272);
            bytes[440..440 + maps.len()].copy_from_slice(&maps);
            440 + maps.len()
        }
        7 => {
            put32(&mut bytes, 16, 5);
            24
        }
        8 => 512,
        _ => unreachable!("fixed volume descriptor sequence"),
    };
    tag(&mut bytes, id, location, length, options.revision);
    Ok(bytes)
}
fn integrity(options: &UdfOptions, layout: &VolumeLayout) -> Result<[u8; 2048]> {
    let mut bytes = [0; 2048];
    timestamp(&mut bytes, 16, options.timestamp);
    put32(&mut bytes, 28, u32::from(layout.vat.is_none()));
    put64(
        &mut bytes,
        40,
        u64::from(layout.files) + u64::from(layout.directories) + 16,
    );
    let count = if layout.namespace_partition == 1 || layout.payload_partition_start.is_some() {
        2
    } else {
        1
    };
    put32(&mut bytes, 72, count);
    put32(&mut bytes, 76, 46);
    let mut sizes = vec![
        layout
            .payload_partition_start
            .unwrap_or(layout.partition_blocks),
    ];
    if count == 2 {
        sizes.push(if let Some(split) = layout.payload_partition_start {
            layout.partition_blocks - split
        } else if let Some(metadata) = &layout.metadata {
            metadata.blocks
        } else {
            layout.vat.as_ref().context("missing VAT layout")?.entries
        });
    }
    for (index, size) in sizes.into_iter().enumerate() {
        let virtual_partition = index == 1 && layout.vat.is_some();
        put32(
            &mut bytes,
            80 + 4 * index,
            if virtual_partition { u32::MAX } else { 0 },
        );
        put32(
            &mut bytes,
            80 + 4 * count as usize + 4 * index,
            if virtual_partition { u32::MAX } else { size },
        );
    }
    let implementation = 80 + 8 * count as usize;
    reg(&mut bytes, implementation, b"*libmkiso", None);
    put32(&mut bytes, implementation + 32, layout.files);
    put32(&mut bytes, implementation + 36, layout.directories);
    put16(&mut bytes, implementation + 40, options.revision.number());
    put16(&mut bytes, implementation + 42, options.revision.number());
    put16(&mut bytes, implementation + 44, options.revision.number());
    tag(&mut bytes, 9, 272, implementation + 46, options.revision);
    Ok(bytes)
}
fn metadata_entry(
    options: &UdfOptions,
    location: u32,
    file_type: u8,
    data: u32,
    length: u32,
    extent_blocks: u32,
) -> Result<[u8; 2048]> {
    // ECMA-167 4/14.17 Extended File Entry. The metadata file must use an
    // extended entry with long allocation descriptors; libudfread (and the
    // Blu-ray UDF reader) reject the short File Entry encoding here.
    let mut bytes = [0; 2048];
    put16(&mut bytes, 20, 4);
    put16(&mut bytes, 24, 1);
    bytes[27] = file_type;
    put16(&mut bytes, 34, 1);
    put32(&mut bytes, 36, u32::MAX);
    put32(&mut bytes, 40, u32::MAX);
    put32(&mut bytes, 44, 0x1084);
    put16(&mut bytes, 48, 1);
    put64(&mut bytes, 56, u64::from(length));
    put64(&mut bytes, 64, u64::from(length));
    put64(&mut bytes, 72, u64::from(length.div_ceil(BLOCK as u32)));
    for offset in [80, 92, 104, 116] {
        timestamp(&mut bytes, offset, options.timestamp);
    }
    put32(&mut bytes, 128, 1);
    reg(&mut bytes, 168, b"*libmkiso", None);
    put64(&mut bytes, 200, 0);
    let chunk = if extent_blocks == 0 {
        length
    } else {
        extent_blocks
            .checked_mul(BLOCK as u32)
            .context("metadata extent size overflow")?
    };
    let mut remaining = length;
    let mut position = 216;
    let mut location_data = data;
    while remaining != 0 {
        ensure!(
            position + 16 <= bytes.len(),
            "metadata allocation descriptors exceed ICB block"
        );
        put32(&mut bytes, position, remaining.min(chunk));
        put32(&mut bytes, position + 4, location_data);
        put16(&mut bytes, position + 8, 0);
        let amount = remaining.min(chunk);
        remaining -= amount;
        if remaining != 0 {
            location_data = location_data
                .checked_add(amount / BLOCK as u32 + 32)
                .context("metadata extent location overflow")?;
        }
        position += 16;
    }
    put32(&mut bytes, 212, (position - 216) as u32);
    tag(&mut bytes, 266, location, position, options.revision);
    Ok(bytes)
}
fn write_metadata_entry(
    output: &mut File,
    options: &UdfOptions,
    location: u32,
    file_type: u8,
    data: u32,
    length: u32,
    extent_blocks: u32,
) -> Result<()> {
    let chunk = if extent_blocks == 0 {
        length
    } else {
        extent_blocks
            .checked_mul(BLOCK as u32)
            .context("metadata chunk overflow")?
    };
    ensure!(
        chunk > 0 && chunk <= 0x3fff_ffff,
        "invalid metadata allocation extent size"
    );
    let descriptors = length.div_ceil(chunk);
    // Extended entries hold 16-byte long allocation descriptors starting at
    // offset 216, so 114 fit in the entry itself.
    if descriptors <= 114 {
        return sector(
            output,
            PART + location,
            &metadata_entry(options, location, file_type, data, length, extent_blocks)?,
        );
    }
    ensure!(
        extent_blocks != 0,
        "metadata allocation chain requires fragmented storage"
    );
    // The entry keeps 113 descriptors plus the AED pointer; every following
    // allocation extent keeps 125 descriptors plus its own pointer, except the
    // last one, which can use all 126 slots.
    let aed_count = (descriptors - 114).div_ceil(125);
    ensure!(
        aed_count <= 32,
        "metadata allocation chain exceeds reserved gap"
    );
    let aed_start = data
        .checked_add(extent_blocks)
        .context("metadata AED address overflow")?;
    let mut bytes = metadata_entry(options, location, file_type, data, 0, 0)?;
    put64(&mut bytes, 56, u64::from(length));
    put64(&mut bytes, 64, u64::from(length));
    put64(&mut bytes, 72, u64::from(length.div_ceil(BLOCK as u32)));
    let mut remaining = length;
    let mut physical = data;
    let mut header = 216;
    let mut descriptor_location = location;
    let mut descriptor_type = 266;
    let mut next_aed = 0;
    loop {
        let capacity = (2048 - header) / 16;
        let remaining_descriptors = remaining.div_ceil(chunk) as usize;
        let has_next = remaining_descriptors > capacity;
        let count = remaining_descriptors.min(if has_next { capacity - 1 } else { capacity });
        let mut cursor = header;
        for _ in 0..count {
            let amount = remaining.min(chunk);
            put32(&mut bytes, cursor, amount);
            put32(&mut bytes, cursor + 4, physical);
            put16(&mut bytes, cursor + 8, 0);
            remaining -= amount;
            if remaining != 0 {
                physical = physical
                    .checked_add(amount / BLOCK as u32 + 32)
                    .context("metadata extent address overflow")?;
            }
            cursor += 16;
        }
        let next_location = aed_start
            .checked_add(next_aed)
            .context("metadata AED address overflow")?;
        if has_next {
            put32(&mut bytes, cursor, 0xc0000800);
            put32(&mut bytes, cursor + 4, next_location);
            put16(&mut bytes, cursor + 8, 0);
            cursor += 16;
        }
        put32(
            &mut bytes,
            if header == 216 { 212 } else { 20 },
            (cursor - header) as u32,
        );
        tag(
            &mut bytes,
            descriptor_type,
            descriptor_location,
            cursor,
            options.revision,
        );
        sector(output, PART + descriptor_location, &bytes)?;
        if !has_next {
            break;
        }
        bytes.fill(0);
        header = 24;
        descriptor_location = next_location;
        descriptor_type = 258;
        next_aed += 1;
    }
    Ok(())
}

fn vat(
    output: &mut File,
    options: &UdfOptions,
    layout: &VolumeLayout,
    vat: &VatLayout,
) -> Result<()> {
    let header = if options.revision == UdfRevision::V150 {
        0
    } else {
        152
    };
    let trailer = if header == 0 { 36 } else { 0 };
    let length = 4u64 * u64::from(vat.entries) + header + trailer;
    ensure!(length <= 0x3fff_ffff, "VAT exceeds one allocation extent");
    output.seek(SeekFrom::Start(u64::from(PART + vat.data) * BLOCK))?;
    if header != 0 {
        let mut bytes = [0; 152];
        put16(&mut bytes, 0, 152);
        dstring(&mut bytes, 4, 128, &options.label)?;
        put32(&mut bytes, 132, u32::MAX);
        put32(&mut bytes, 136, layout.files);
        put32(&mut bytes, 140, layout.directories);
        put16(&mut bytes, 144, options.revision.number());
        put16(&mut bytes, 146, options.revision.number());
        put16(&mut bytes, 148, options.revision.number());
        output.write_all(&bytes)?;
    }
    for entry in 0..vat.entries {
        output.write_all(
            &vat.mapped_start
                .checked_add(entry)
                .context("VAT mapping overflow")?
                .to_le_bytes(),
        )?;
    }
    if trailer != 0 {
        let mut bytes = [0; 36];
        reg(
            &mut bytes,
            0,
            b"*UDF Virtual Alloc Tbl",
            Some(options.revision),
        );
        put32(&mut bytes, 32, u32::MAX);
        output.write_all(&bytes)?;
    }
    let mut bytes = [0; 2048];
    let extended = options.revision.number() >= 0x200;
    let base = if extended { 216 } else { 176 };
    put16(&mut bytes, 20, 4);
    put16(&mut bytes, 24, 1);
    bytes[27] = if extended { 248 } else { 0 };
    put32(&mut bytes, 36, u32::MAX);
    put32(&mut bytes, 40, u32::MAX);
    put32(&mut bytes, 44, 0x1084);
    put16(&mut bytes, 48, 1);
    put64(&mut bytes, 56, length);
    if extended {
        put64(&mut bytes, 64, length);
    }
    put64(
        &mut bytes,
        if extended { 72 } else { 64 },
        length.div_ceil(BLOCK),
    );
    for offset in if extended {
        &[80, 92, 104, 116][..]
    } else {
        &[72, 84, 96][..]
    } {
        timestamp(&mut bytes, *offset, options.timestamp);
    }
    put32(&mut bytes, if extended { 128 } else { 108 }, 1);
    reg(&mut bytes, base - 48, b"*libmkiso", None);
    put64(&mut bytes, base - 16, 0);
    put32(&mut bytes, base - 4, 8);
    put32(&mut bytes, base, length as u32);
    put32(&mut bytes, base + 4, vat.data);
    tag(
        &mut bytes,
        if extended { 266 } else { 261 },
        vat.icb,
        base + 8,
        options.revision,
    );
    sector(output, PART + vat.icb, &bytes)
}
pub(super) fn emit(
    output: &mut File,
    options: &UdfOptions,
    layout: &VolumeLayout,
    total_blocks: u32,
    reserve: u32,
) -> Result<()> {
    for (offset, id) in [
        b"BEA01",
        if options.revision.number() >= 0x200 {
            b"NSR03"
        } else {
            b"NSR02"
        },
        b"TEA01",
    ]
    .into_iter()
    .enumerate()
    {
        let mut bytes = [0; 2048];
        bytes[1..6].copy_from_slice(id);
        bytes[6] = 1;
        sector(
            output,
            (if options.boot.is_empty() { 16 } else { 19 }) + offset as u32,
            &bytes,
        )?;
    }
    let sequence: &[u16] = if layout.payload_partition_start.is_some() {
        &[1, 4, 5, 5, 6, 7, 8]
    } else {
        &[1, 4, 5, 6, 7, 8]
    };
    for first in [257, reserve] {
        for (offset, id) in sequence.iter().copied().enumerate() {
            let location = first + offset as u32;
            let mut bytes = descriptor(options, layout, id, location)?;
            if offset == 3 && id == 5 {
                let split = layout
                    .payload_partition_start
                    .context("missing split partition")?;
                put16(&mut bytes, 22, 1);
                put32(&mut bytes, 188, PART + split);
                put32(&mut bytes, 192, layout.partition_blocks - split);
                tag(&mut bytes, id, location, 512, options.revision);
            }
            sector(output, location, &bytes)?;
        }
    }
    sector(output, 272, &integrity(options, layout)?)?;
    sector(output, 273, &descriptor(options, layout, 8, 273)?)?;
    for location in [256, total_blocks - 257, total_blocks - 1] {
        if matches!(options.partition, UdfPartition::Virtual) && location == total_blocks - 1 {
            continue;
        }
        let mut bytes = [0; 2048];
        put32(&mut bytes, 16, sequence.len() as u32 * BLOCK as u32);
        put32(&mut bytes, 20, 257);
        put32(&mut bytes, 24, sequence.len() as u32 * BLOCK as u32);
        put32(&mut bytes, 28, reserve);
        tag(&mut bytes, 2, location, 512, options.revision);
        sector(output, location, &bytes)?;
    }
    if let Some(metadata) = &layout.metadata {
        let length = metadata
            .blocks
            .checked_mul(BLOCK as u32)
            .context("metadata extent exceeds 32-bit bytes")?;

        for (location, file_type, data) in [
            (metadata.primary_icb, 250, metadata.primary_data),
            (metadata.mirror_icb, 251, metadata.mirror_data),
        ] {
            write_metadata_entry(
                output,
                options,
                location,
                file_type,
                data,
                length,
                metadata.extent_blocks,
            )?;
        }
    }
    if let Some(metadata) = &layout.metadata
        && let Some(location) = metadata.bitmap_icb
    {
        let length = metadata
            .blocks
            .div_ceil(8)
            .checked_add(24)
            .context("metadata bitmap size overflow")?;
        sector(
            output,
            PART + location,
            &metadata_entry(options, location, 252, metadata.bitmap_data, length, 0)?,
        )?;
        let mut bytes = [0; 2048];
        put32(&mut bytes, 16, metadata.blocks);
        put32(&mut bytes, 20, metadata.blocks.div_ceil(8));
        tag(&mut bytes, 264, metadata.bitmap_data, 24, options.revision);
        sector(output, PART + metadata.bitmap_data, &bytes)?;
    }
    if matches!(
        options.partition,
        UdfPartition::Sparable { .. } | UdfPartition::MetadataSparable { .. }
    ) {
        for location in sparing_locations(layout) {
            let length = 56 + 8 * layout.spare_packets.len();
            ensure!(
                length <= 32 * BLOCK as usize,
                "sparing table exceeds reserved extent"
            );
            let mut bytes = vec![0; length];
            reg(
                &mut bytes,
                16,
                b"*UDF Sparing Table",
                Some(options.revision),
            );
            put16(
                &mut bytes,
                48,
                u16::try_from(layout.spare_packets.len()).context("too many sparing entries")?,
            );
            for (index, (original, mapped)) in layout.spare_packets.iter().copied().enumerate() {
                put32(&mut bytes, 56 + 8 * index, original);
                put32(&mut bytes, 60 + 8 * index, mapped);
            }
            tag(&mut bytes, 0, location, length, options.revision);
            sector(output, location, &bytes)?;
        }
    }
    if let Some(location) = layout.bitmap {
        let mut bytes = [0; 2048];
        put32(&mut bytes, 16, layout.partition_blocks);
        put32(&mut bytes, 20, layout.partition_blocks.div_ceil(8));
        tag(&mut bytes, 264, location, 24, options.revision);
        sector(output, PART + location, &bytes)?;
    }
    if let Some(table) = &layout.vat {
        vat(output, options, layout, table)?;
    }
    Ok(())
}
