//! Physical and UDF 2.50/2.60 metadata partition address translation.
use super::{
    Error, Limits, Result,
    allocations::{self, Extent},
    bad, region, tag, u16_at, u32_at, u64_at,
};

const BLOCK: u64 = 2048;

struct Metadata {
    size: u64,
    extents: Vec<Extent>,
}
enum Map {
    Physical(u64, u64),
    Virtual(u64, u64, Vec<u32>),
    Sparable(u64, u64, u16, Vec<(u32, u32)>),
    Metadata(Metadata, Option<Metadata>),
}
pub(super) struct VolumeMap {
    maps: Vec<Map>,
    max_translation_extents: u64,
}
impl VolumeMap {
    pub(super) fn open(
        bytes: &dyn crate::source::ReadAt,
        partitions: &[Vec<u8>],
        logical: &[u8],
        limits: Limits,
        budget: &mut u64,
    ) -> Result<Self> {
        let map_length = u32_at(logical, 264)? as usize;
        let map_count = u32_at(logical, 268)? as usize;
        let table = logical
            .get(
                440..440usize
                    .checked_add(map_length)
                    .ok_or_else(|| bad("partition table overflow"))?,
            )
            .ok_or_else(|| bad("partition table range"))?;
        if map_count == 0
            || map_count > 256
            || 16 + usize::from(u16_at(logical, 10)?) < 440 + map_length
        {
            return Err(bad("partition map count or checksum coverage"));
        }
        let mut raw = Vec::new();
        let mut position = 0;
        while position < table.len() {
            let remaining = &table[position..];
            let length = usize::from(
                *remaining
                    .get(1)
                    .ok_or_else(|| bad("truncated partition map"))?,
            );
            if length < 2 {
                return Err(bad("partition map length"));
            }
            let map = remaining
                .get(..length)
                .ok_or_else(|| bad("partition map range"))?;
            if !matches!((map[0], length), (1, 6) | (2, 64)) {
                return Err(Error::Unsupported("UDF partition map type".into()));
            }
            let number = u16_at(map, if map[0] == 1 { 4 } else { 38 })?;
            let partition = partitions
                .iter()
                .find(|p| u16_at(p, 22).ok() == Some(number))
                .ok_or_else(|| bad("partition map references missing descriptor"))?;
            if u16_at(map, if map[0] == 1 { 2 } else { 36 })? != 1 {
                return Err(Error::Unsupported("multiple volume sequences".into()));
            }
            let start = u64::from(u32_at(partition, 188)?) * BLOCK;
            if start > bytes.len() {
                return Err(bad("partition begins outside image"));
            }
            raw.push(map);
            position += length;
        }
        if raw.len() != map_count {
            return Err(bad("partition map count"));
        }
        for partition in partitions {
            let number = u16_at(partition, 22)?;
            let has_virtual = raw.iter().any(|map| {
                map[0] == 2
                    && &map[5..27] == b"*UDF Virtual Partition"
                    && u16_at(map, 38).ok() == Some(number)
            });
            if !has_virtual {
                super::check_region(
                    bytes,
                    u64::from(u32_at(partition, 188)?) * BLOCK,
                    u64::from(u32_at(partition, 192)?) * BLOCK,
                )?;
            }
        }
        let mut maps = Vec::new();
        for map in &raw {
            let number = u16_at(map, if map[0] == 1 { 4 } else { 38 })?;
            let partition = partitions
                .iter()
                .find(|p| u16_at(p, 22).ok() == Some(number))
                .ok_or_else(|| bad("missing partition"))?;
            let start = u64::from(u32_at(partition, 188)?) * BLOCK;
            let size = u64::from(u32_at(partition, 192)?) * BLOCK;
            if map[0] == 1 {
                maps.push(Map::Physical(start, size));
                continue;
            }
            if &map[5..27] == b"*UDF Virtual Partition" {
                maps.push(Map::Virtual(
                    start,
                    size,
                    vat(
                        bytes,
                        start,
                        size,
                        physical_reference_for(&raw, number)?,
                        limits,
                        budget,
                    )?,
                ));
                continue;
            }
            if &map[5..28] == b"*UDF Sparable Partition" {
                let (packet, entries) = sparing(bytes, map, limits, budget)?;
                maps.push(Map::Sparable(start, size, packet, entries));
                continue;
            }
            if &map[5..28] != b"*UDF Metadata Partition" {
                return Err(Error::Unsupported("UDF type 2 partition".into()));
            }
            if !matches!(u16_at(logical, 240)?, 0x250 | 0x260) {
                return Err(bad("metadata partition requires UDF 2.50 or 2.60"));
            }
            if !matches!(u32_at(partition, 184)?, 0 | 1 | 4) {
                return Err(bad("metadata partition access type"));
            }
            let physical_reference = raw
                .iter()
                .position(|candidate| {
                    (candidate[0] == 1 && u16_at(candidate, 4).ok() == Some(number))
                        || (candidate[0] == 2
                            && &candidate[5..28] == b"*UDF Sparable Partition"
                            && u16_at(candidate, 38).ok() == Some(number))
                })
                .ok_or_else(|| bad("metadata partition requires underlying map"))?
                as u16;
            let backing = if raw[usize::from(physical_reference)][0] == 1 {
                Map::Physical(start, size)
            } else {
                let (packet, entries) =
                    sparing(bytes, raw[usize::from(physical_reference)], limits, budget)?;
                Map::Sparable(start, size, packet, entries)
            };
            let unit = u32_at(map, 52)?;
            let alignment = u16_at(map, 56)?;
            if unit == 0 || !unit.is_multiple_of(32) || alignment == 0 {
                return Err(bad("metadata allocation units"));
            }
            if matches!(u32_at(partition, 184)?, 0 | 1) && u32_at(map, 48)? != u32::MAX {
                return Err(bad("read-only metadata partition bitmap"));
            }
            let primary_location = u32_at(map, 40)?;
            let mirror_location = u32_at(map, 44)?;
            if primary_location == mirror_location {
                return Err(bad("metadata mirror aliases primary ICB"));
            }
            let primary = Self::metadata(
                bytes,
                start,
                size,
                primary_location,
                250,
                physical_reference,
                &backing,
                unit,
                alignment,
                limits,
                budget,
            );
            let mirror = Self::metadata(
                bytes,
                start,
                size,
                mirror_location,
                251,
                physical_reference,
                &backing,
                unit,
                alignment,
                limits,
                budget,
            );
            for result in [&primary, &mirror] {
                if let Err(Error::ResourceLimit(reason)) = result {
                    return Err(Error::ResourceLimit(reason));
                }
            }
            let (metadata, fallback) = match (primary, mirror) {
                (Ok(primary), Ok(mirror)) => {
                    if primary.size != mirror.size {
                        return Err(bad("metadata mirror size mismatch"));
                    }
                    if map[58] & 1 == 0 && !same_mapping(&primary.extents, &mirror.extents) {
                        return Err(bad("shared metadata mirror allocation mismatch"));
                    }
                    if map[58] & 1 != 0 {
                        *budget = budget
                            .checked_add(
                                (primary.extents.len() as u64 + mirror.extents.len() as u64) * 16,
                            )
                            .ok_or(Error::ResourceLimit("metadata bytes"))?;
                        if *budget > limits.max_metadata_bytes {
                            return Err(Error::ResourceLimit("metadata bytes"));
                        }
                    }
                    if map[58] & 1 != 0 && overlaps(&primary.extents, &mirror.extents) {
                        return Err(bad("duplicated metadata mirror allocation overlaps"));
                    }
                    (primary, Some(mirror))
                }
                (Err(error @ Error::Io(_)), _) | (_, Err(error @ Error::Io(_))) => {
                    return Err(error);
                }
                (Ok(primary), Err(Error::ResourceLimit(reason)))
                | (Err(Error::ResourceLimit(reason)), Ok(primary)) => {
                    let _ = primary;
                    return Err(Error::ResourceLimit(reason));
                }
                (Ok(primary), Err(_)) => (primary, None),
                (Err(_), Ok(mirror)) => (mirror, None),
                (Err(error), Err(_)) => return Err(error),
            };
            maps.push(Map::Metadata(metadata, fallback));
        }
        Ok(Self {
            maps,
            max_translation_extents: limits.max_metadata_bytes
                / std::mem::size_of::<Extent>() as u64,
        })
    }
    pub(super) fn resolve(&self, partition: u16, block: u32, length: u64) -> Result<Vec<Extent>> {
        let map = self
            .maps
            .get(usize::from(partition))
            .ok_or_else(|| bad("unknown partition reference"))?;
        Self::resolve_map(map, block, length, self.max_translation_extents)
    }
    fn resolve_map(map: &Map, block: u32, length: u64, max_extents: u64) -> Result<Vec<Extent>> {
        let requested = u64::from(block) * BLOCK;
        match map {
            Map::Physical(start, size) => physical(*start, *size, block, length),
            Map::Virtual(start, size, entries) => translate(block, length, max_extents, |block| {
                let mapped = *entries
                    .get(block as usize)
                    .ok_or_else(|| bad("VAT block outside table"))?;
                if mapped == u32::MAX {
                    return Err(bad("unallocated VAT block"));
                }
                Ok(physical(*start, *size, mapped, BLOCK)?[0]
                    .offset
                    .unwrap_or(0))
            }),
            Map::Sparable(start, size, packet, entries) => {
                translate(block, length, max_extents, |block| {
                    physical(*start, *size, block, BLOCK)?;
                    let base = block - block % u32::from(*packet);
                    match entries.binary_search_by_key(&base, |entry| entry.0) {
                        Ok(index) => Ok(u64::from(entries[index].1 + block - base) * BLOCK),
                        Err(_) => Ok(*start + u64::from(block) * BLOCK),
                    }
                })
            }
            Map::Metadata(metadata, _) => Self::resolve_metadata(metadata, requested, length),
        }
    }
    pub(super) fn resolve_mirror(
        &self,
        partition: u16,
        block: u32,
        length: u64,
    ) -> Result<Option<Vec<Extent>>> {
        match self
            .maps
            .get(usize::from(partition))
            .ok_or_else(|| bad("unknown partition reference"))?
        {
            Map::Metadata(_, Some(mirror)) => {
                Self::resolve_metadata(mirror, u64::from(block) * BLOCK, length).map(Some)
            }
            _ => Ok(None),
        }
    }
    fn resolve_metadata(metadata: &Metadata, requested: u64, length: u64) -> Result<Vec<Extent>> {
        let end = requested
            .checked_add(length)
            .ok_or_else(|| bad("metadata address overflow"))?;
        if end > metadata.size {
            return Err(bad("extent outside metadata partition"));
        }
        let mut result = Vec::new();
        let first = metadata
            .extents
            .partition_point(|extent| extent.logical_byte + extent.length <= requested);
        for extent in &metadata.extents[first..] {
            let position = extent.logical_byte;
            let extent_end = position + extent.length;
            let from = requested.max(position);
            let to = end.min(extent_end);
            if from < to {
                let offset = extent
                    .offset
                    .ok_or_else(|| bad("referenced unallocated metadata block"))?;
                result.push(Extent {
                    offset: Some(offset + from - position),
                    allocated_unrecorded: false,
                    length: to - from,
                    logical_byte: from,
                });
            }
            if extent_end >= end {
                break;
            }
        }
        Ok(result)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "metadata bootstrap has explicit validated physical geometry"
    )]
    fn metadata(
        bytes: &dyn crate::source::ReadAt,
        start: u64,
        _size: u64,
        location: u32,
        file_type: u8,
        physical_reference: u16,
        backing: &Map,
        unit: u32,
        alignment: u16,
        limits: Limits,
        budget: &mut u64,
    ) -> Result<Metadata> {
        *budget = budget
            .checked_add(BLOCK)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        if *budget > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        let extent = Self::resolve_map(
            backing,
            location,
            BLOCK,
            limits.max_metadata_bytes / std::mem::size_of::<Extent>() as u64,
        )?;
        let descriptor = region(
            bytes,
            extent[0].offset.ok_or_else(|| bad("metadata ICB"))?,
            BLOCK,
        )?;
        let kind = u16_at(&descriptor, 0)?;
        if !matches!(kind, 261 | 266) {
            return Err(bad("metadata file entry type"));
        }
        tag(&descriptor, kind, location)?;
        let allocation_type = u16_at(&descriptor, 34)? & 7;
        if descriptor[27] != file_type
            || u16_at(&descriptor, 20)? != 4
            || !matches!(allocation_type, 0 | 1)
            || (kind == 261 && allocation_type != 0)
        {
            return Err(bad(
                "metadata file requires supported allocations and correct file type",
            ));
        }
        let header = if kind == 266 { 216usize } else { 176 };
        if u64_at(&descriptor, header - 16)? != 0
            || u32_at(&descriptor, if kind == 266 { 136 } else { 112 })? != 0
            || (kind == 266
                && (u32_at(&descriptor, 152)? != 0
                    || u64_at(&descriptor, 64)? != u64_at(&descriptor, 56)?))
        {
            return Err(bad("metadata file unique ID, attributes or streams"));
        }
        let file_size = u64_at(&descriptor, 56)?;
        let allocations_start = header
            .checked_add(u32_at(&descriptor, header - 8)? as usize)
            .ok_or_else(|| bad("metadata allocations overflow"))?;
        let allocations_length = u32_at(&descriptor, header - 4)? as usize;
        let initial = descriptor
            .get(
                allocations_start
                    ..allocations_start
                        .checked_add(allocations_length)
                        .ok_or_else(|| bad("metadata allocations overflow"))?,
            )
            .ok_or_else(|| bad("metadata allocations range"))?;
        *budget = budget
            .checked_add(initial.len() as u64)
            .ok_or(Error::ResourceLimit("metadata bytes"))?;
        if *budget > limits.max_metadata_bytes {
            return Err(Error::ResourceLimit("metadata bytes"));
        }
        let mut extents = allocations::decode_metadata(
            bytes,
            initial,
            allocation_type,
            physical_reference,
            file_size,
            budget,
            limits,
            |partition, block, length| {
                if partition != physical_reference {
                    return Err(bad("metadata allocation partition"));
                }
                Self::resolve_map(
                    backing,
                    block,
                    length,
                    limits.max_metadata_bytes / std::mem::size_of::<Extent>() as u64,
                )
            },
        )?;
        let allocation_unit = u64::from(unit) * BLOCK;
        if file_size == 0 || !file_size.is_multiple_of(allocation_unit) {
            return Err(bad("metadata size not allocation unit multiple"));
        }
        for extent in &extents {
            if !extent.length.is_multiple_of(allocation_unit) {
                return Err(bad("metadata extent length not allocation unit multiple"));
            }
            if let Some(offset) = extent.offset
                && !((offset - start) / BLOCK).is_multiple_of(u64::from(alignment))
            {
                return Err(bad("metadata extent alignment"));
            }
        }
        // Cache each file-relative start for binary-search address translation.
        let mut logical_byte = 0;
        for extent in &mut extents {
            extent.logical_byte = logical_byte;
            logical_byte += extent.length;
        }
        Ok(Metadata {
            size: file_size,
            extents,
        })
    }
}
fn physical(start: u64, size: u64, block: u32, length: u64) -> Result<Vec<Extent>> {
    let offset = u64::from(block) * BLOCK;
    if offset.checked_add(length).is_none_or(|end| end > size) {
        return Err(bad("extent outside physical partition"));
    }
    Ok(vec![Extent {
        offset: Some(start + offset),
        allocated_unrecorded: false,
        length,
        logical_byte: offset,
    }])
}
fn same_mapping(left: &[Extent], right: &[Extent]) -> bool {
    // Segmentation can differ while the byte mapping remains identical.
    let (mut li, mut ri, mut lo, mut ro) = (0, 0, 0, 0);
    while li < left.len() && ri < right.len() {
        if left[li].offset.map(|offset| offset + lo) != right[ri].offset.map(|offset| offset + ro) {
            return false;
        }
        let length = (left[li].length - lo).min(right[ri].length - ro);
        lo += length;
        ro += length;
        if lo == left[li].length {
            li += 1;
            lo = 0;
        }
        if ro == right[ri].length {
            ri += 1;
            ro = 0;
        }
    }
    li == left.len() && ri == right.len()
}
fn overlaps(left: &[Extent], right: &[Extent]) -> bool {
    let mut ranges: Vec<_> = left
        .iter()
        .chain(right)
        .filter_map(|extent| extent.offset.map(|offset| (offset, offset + extent.length)))
        .collect();
    ranges.sort_unstable();
    ranges.windows(2).any(|pair| pair[0].1 > pair[1].0)
}
fn charge(budget: &mut u64, amount: u64, limits: Limits) -> Result<()> {
    *budget = budget
        .checked_add(amount)
        .ok_or(Error::ResourceLimit("metadata bytes"))?;
    if *budget > limits.max_metadata_bytes {
        return Err(Error::ResourceLimit("metadata bytes"));
    }
    Ok(())
}
fn translate(
    block: u32,
    length: u64,
    max_extents: u64,
    mut offset: impl FnMut(u32) -> Result<u64>,
) -> Result<Vec<Extent>> {
    let count = length.div_ceil(BLOCK);
    let mut extents: Vec<Extent> = Vec::new();
    for index in 0..count {
        let block = block
            .checked_add(u32::try_from(index).map_err(|_| bad("block range overflow"))?)
            .ok_or_else(|| bad("block range overflow"))?;
        let absolute = offset(block)?;
        let amount = (length - index * BLOCK).min(BLOCK);
        if let Some(previous) = extents.last_mut()
            && previous
                .offset
                .and_then(|offset| offset.checked_add(previous.length))
                == Some(absolute)
        {
            previous.length += amount;
        } else {
            if extents.len() as u64 >= max_extents {
                return Err(Error::ResourceLimit("partition translation extents"));
            }
            extents.push(Extent {
                offset: Some(absolute),
                allocated_unrecorded: false,
                length: amount,
                logical_byte: u64::from(block) * BLOCK,
            });
        }
    }
    Ok(extents)
}
fn vat(
    bytes: &dyn crate::source::ReadAt,
    start: u64,
    partition_size: u64,
    physical_reference: u16,
    limits: Limits,
    budget: &mut u64,
) -> Result<Vec<u32>> {
    // The VAT ICB is the final recorded block. Search a bounded tail to accommodate padding.
    let end = (start + partition_size).min(bytes.len()) / BLOCK;
    let disc_end = bytes.len() / BLOCK;
    let candidates = (disc_end.saturating_sub(512).max(start / BLOCK)..disc_end)
        .rev()
        .chain((end.saturating_sub(512).max(start / BLOCK)..end).rev());
    let mut visited = std::collections::HashSet::new();
    for absolute in candidates {
        if !visited.insert(absolute) {
            continue;
        }
        charge(budget, BLOCK, limits)?;
        let descriptor = region(bytes, absolute * BLOCK, BLOCK)?;
        let kind = u16_at(&descriptor, 0)?;
        if !matches!(kind, 261 | 266) || !matches!(descriptor[27], 0 | 248) {
            continue;
        }
        let location = u32::try_from(absolute - start / BLOCK).map_err(|_| bad("VAT location"))?;
        if tag(&descriptor, kind, location).is_err() {
            continue;
        }
        let header = if kind == 266 { 216usize } else { 176 };
        let allocation_type = u16_at(&descriptor, 34)? & 7;
        let size = u64_at(&descriptor, 56)?;
        charge(budget, size, limits)?;
        let begin = header
            .checked_add(u32_at(&descriptor, header - 8)? as usize)
            .ok_or_else(|| bad("VAT attributes"))?;
        let allocation = descriptor
            .get(
                begin
                    ..begin
                        .checked_add(u32_at(&descriptor, header - 4)? as usize)
                        .ok_or_else(|| bad("VAT allocations"))?,
            )
            .ok_or_else(|| bad("VAT allocation range"))?;
        let data = if allocation_type == 3 {
            allocation
                .get(..usize::try_from(size).map_err(|_| bad("VAT size"))?)
                .ok_or_else(|| bad("VAT embedded size"))?
                .to_vec()
        } else {
            let extents = allocations::decode(
                bytes,
                allocation,
                allocation_type,
                physical_reference,
                size,
                budget,
                limits,
                |partition, block, length| {
                    if partition != physical_reference {
                        return Err(bad("VAT allocation partition"));
                    }
                    physical(start, partition_size, block, length)
                },
            )?;
            allocations::read_recorded(bytes, &extents, size)?
        };
        let table = if descriptor[27] == 248 {
            let header = usize::from(u16_at(&data, 0)?);
            if header < 152 || header != 152 + usize::from(u16_at(&data, 2)?) {
                return Err(bad("VAT header length"));
            }
            data.get(header..).ok_or_else(|| bad("VAT header range"))?
        } else {
            if data.len() < 36
                || &data[data.len() - 35..data.len() - 13] != b"*UDF Virtual Alloc Tbl"
            {
                return Err(bad("VAT 1.50 identifier"));
            }
            &data[..data.len() - 36]
        };
        if !table.len().is_multiple_of(4) {
            return Err(bad("VAT entry length"));
        }
        return table
            .chunks_exact(4)
            .map(|entry| u32_at(entry, 0))
            .collect();
    }
    Err(bad("VAT ICB not found in bounded recorded tail"))
}
fn sparing(
    bytes: &dyn crate::source::ReadAt,
    map: &[u8],
    limits: Limits,
    budget: &mut u64,
) -> Result<(u16, Vec<(u32, u32)>)> {
    let packet = u16_at(map, 40)?;
    let copies = usize::from(map[42]);
    let size = u32_at(map, 44)?;
    if packet == 0 || !packet.is_power_of_two() || !(1..=4).contains(&copies) || size < 56 {
        return Err(bad("sparing dimensions"));
    }
    let mut prevailing: Option<(u32, Vec<(u32, u32)>)> = None;
    for index in 0..copies {
        charge(budget, u64::from(size), limits)?;
        let location = u32_at(map, 48 + index * 4)?;
        let table = match region(bytes, u64::from(location) * BLOCK, u64::from(size)) {
            Ok(table) => table,
            Err(error @ Error::Io(_)) => return Err(error),
            Err(_) => continue,
        };
        if tag(&table, 0, location).is_err() || &table[17..35] != b"*UDF Sparing Table" {
            continue;
        }
        let count = usize::from(u16_at(&table, 48)?);
        if 56 + count * 8 > table.len() || 16 + usize::from(u16_at(&table, 10)?) < 56 + count * 8 {
            continue;
        }
        let sequence = u32_at(&table, 52)?;
        let mut entries = Vec::new();
        let mut replacements = std::collections::HashSet::new();
        for index in 0..count {
            let original = u32_at(&table, 56 + index * 8)?;
            let mapped = u32_at(&table, 60 + index * 8)?;
            if !replacements.insert(mapped) {
                return Err(bad("duplicate sparing replacement"));
            }
            super::check_region(bytes, u64::from(mapped) * BLOCK, u64::from(packet) * BLOCK)?;
            if original >= 0xfffffff0 {
                continue;
            }
            if !original.is_multiple_of(u32::from(packet)) {
                return Err(bad("sparing original alignment"));
            }
            entries.push((original, mapped));
        }
        let mut replacement_starts: Vec<_> = replacements.into_iter().collect();
        replacement_starts.sort_unstable();
        if replacement_starts
            .windows(2)
            .any(|pair| u64::from(pair[0]) + u64::from(packet) > u64::from(pair[1]))
        {
            return Err(bad("overlapping sparing replacement packets"));
        }
        entries.sort_unstable();
        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(bad("duplicate sparing original"));
        }
        match &prevailing {
            Some((previous, previous_entries))
                if *previous == sequence && *previous_entries != entries =>
            {
                return Err(bad("conflicting sparing copies"));
            }
            Some((previous, _)) if *previous > sequence => (),
            _ => prevailing = Some((sequence, entries)),
        }
    }
    Ok((
        packet,
        prevailing.ok_or_else(|| bad("no valid sparing table"))?.1,
    ))
}

fn physical_reference_for(raw: &[&[u8]], number: u16) -> Result<u16> {
    raw.iter()
        .position(|candidate| candidate[0] == 1 && u16_at(candidate, 4).ok() == Some(number))
        .map(|index| index as u16)
        .ok_or_else(|| bad("virtual partition missing physical map"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_translation_preserves_logical_locations_and_rejects_holes() {
        let map = Map::Virtual(10 * BLOCK, 10 * BLOCK, vec![3, 1, u32::MAX]);
        let extents = VolumeMap::resolve_map(&map, 1, BLOCK, 16).unwrap();
        assert_eq!(extents[0].offset, Some(11 * BLOCK));
        assert_eq!(extents[0].logical_byte, BLOCK);
        assert!(VolumeMap::resolve_map(&map, 2, BLOCK, 16).is_err());
    }
    #[test]
    fn fragmented_virtual_translation_obeys_extent_budget() {
        let map = Map::Virtual(10 * BLOCK, 10 * BLOCK, vec![3, 1]);
        assert!(matches!(
            VolumeMap::resolve_map(&map, 0, 2 * BLOCK, 1),
            Err(Error::ResourceLimit(_))
        ));
    }
    #[test]
    fn sparing_replacement_addresses_are_absolute() {
        let map = Map::Sparable(10 * BLOCK, 20 * BLOCK, 4, vec![(4, 32)]);
        let extents = VolumeMap::resolve_map(&map, 5, BLOCK + 1, 16).unwrap();
        assert_eq!(extents[0].offset, Some(33 * BLOCK));
        assert_eq!(extents[0].length, BLOCK + 1);
    }
    #[test]
    fn physical_partitions_have_independent_geometry() {
        let volume = VolumeMap {
            max_translation_extents: 16,
            maps: vec![
                Map::Physical(10 * BLOCK, 2 * BLOCK),
                Map::Physical(30 * BLOCK, 3 * BLOCK),
            ],
        };
        assert_eq!(
            volume.resolve(1, 1, BLOCK).unwrap()[0].offset,
            Some(31 * BLOCK)
        );
        assert!(volume.resolve(0, 2, BLOCK).is_err());
    }
}
