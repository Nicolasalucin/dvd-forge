//! Bounded short/long/extended UDF allocation descriptors, including sparse data and AED chains.
use std::collections::HashSet;

use super::{Error, Limits, Result, bad, tag, u16_at, u32_at};

#[derive(Debug, Clone, Copy)]
pub(super) struct Extent {
    pub offset: Option<u64>,
    pub allocated_unrecorded: bool,
    pub length: u64,
    pub logical_byte: u64,
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

/// Assemble only recorded metadata; sparse blocks cannot carry descriptors.
pub(super) fn read_recorded(
    bytes: &dyn crate::source::ReadAt,
    extents: &[Extent],
    length: u64,
) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    for extent in extents {
        if (result.len() as u64)
            .checked_add(extent.length)
            .is_none_or(|end| end > length)
        {
            return Err(bad("resolved metadata size exceeds allocation"));
        }
        let offset = extent
            .offset
            .ok_or_else(|| bad("unrecorded metadata extent"))?;
        let start = result.len();
        let end = start
            .checked_add(
                usize::try_from(extent.length).map_err(|_| bad("metadata size conversion"))?,
            )
            .ok_or_else(|| bad("metadata size overflow"))?;
        result.resize(end, 0);
        bytes.read_exact_at(offset, &mut result[start..end])?;
    }
    if result.len() as u64 != length {
        return Err(bad("resolved metadata size mismatch"));
    }
    Ok(result)
}

#[expect(
    clippy::too_many_arguments,
    reason = "allocation context and partition resolver remain explicit"
)]
pub(super) fn decode(
    bytes: &dyn crate::source::ReadAt,
    initial: &[u8],
    allocation_type: u16,
    current_partition: u16,
    file_size: u64,
    budget: &mut u64,
    limits: Limits,
    resolve: impl FnMut(u16, u32, u64) -> Result<Vec<Extent>>,
) -> Result<Vec<Extent>> {
    decode_profile(
        bytes,
        initial,
        allocation_type,
        current_partition,
        file_size,
        budget,
        limits,
        resolve,
        false,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "allocation context and partition resolver remain explicit"
)]
pub(super) fn decode_metadata(
    bytes: &dyn crate::source::ReadAt,
    initial: &[u8],
    allocation_type: u16,
    current_partition: u16,
    file_size: u64,
    budget: &mut u64,
    limits: Limits,
    resolve: impl FnMut(u16, u32, u64) -> Result<Vec<Extent>>,
) -> Result<Vec<Extent>> {
    decode_profile(
        bytes,
        initial,
        allocation_type,
        current_partition,
        file_size,
        budget,
        limits,
        resolve,
        true,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "allocation context and partition resolver remain explicit"
)]
fn decode_profile(
    bytes: &dyn crate::source::ReadAt,
    initial: &[u8],
    allocation_type: u16,
    current_partition: u16,
    file_size: u64,
    budget: &mut u64,
    limits: Limits,
    mut resolve: impl FnMut(u16, u32, u64) -> Result<Vec<Extent>>,
    metadata: bool,
) -> Result<Vec<Extent>> {
    let width = match allocation_type {
        0 => 8,
        1 => 16,
        2 => 20,
        _ => return Err(Error::Unsupported("UDF allocation descriptor type".into())),
    };
    let mut list = initial;
    let mut continuation_data;
    let mut visited = HashSet::new();
    let mut extents = Vec::new();
    let mut allocated = 0u64;
    let mut previous_length = 0u64;
    loop {
        if !list.len().is_multiple_of(width) {
            return Err(bad("partial allocation descriptor"));
        }
        let mut continuation = None;
        for (index, allocation) in list.chunks_exact(width).enumerate() {
            let encoded = u32_at(allocation, 0)?;
            let kind = encoded >> 30;
            let length = u64::from(encoded & 0x3fff_ffff);
            let extended = allocation_type == 2;
            let block = u32_at(allocation, if extended { 12 } else { 4 })?;
            let partition = if allocation_type != 0 {
                u16_at(allocation, if extended { 16 } else { 8 })?
            } else {
                current_partition
            };
            let information = if extended {
                u64::from(u32_at(allocation, 8)?)
            } else {
                length
            };
            if extended {
                let recorded = u64::from(u32_at(allocation, 4)?);
                if recorded > length || information > length || recorded > 0x3fff_ffff {
                    return Err(bad("extended allocation lengths exceed extent"));
                }
                if kind == 0 && recorded != information {
                    // ECMA 167 4/14.14.3 permits implementation-defined compression.
                    return Err(Error::Unsupported(
                        "encoded extended allocation data".into(),
                    ));
                }
                if kind != 0 && recorded != 0 || kind == 3 && information != 0 {
                    return Err(bad(
                        "invalid extended allocation recorded/information length",
                    ));
                }
            }
            if length == 0 {
                if kind != 0 {
                    return Err(bad("zero-length typed allocation"));
                }
                continue;
            }
            if kind == 3 {
                if index + 1 != list.len() / width
                    || !(24..=2048).contains(&length)
                    || partition != current_partition
                {
                    return Err(bad("invalid allocation continuation extent"));
                }
                if !visited.insert((partition, block)) {
                    return Err(bad("allocation continuation cycle"));
                }
                charge(budget, length, limits)?;
                continuation = Some((partition, block, length));
                break;
            }
            if metadata && (kind == 1 || !length.is_multiple_of(2048)) {
                return Err(bad("invalid metadata file allocation"));
            }
            if allocated >= file_size {
                if kind != 1 || metadata || (extended && information != 0) {
                    return Err(bad("invalid preallocated file tail"));
                }
                let resolved = resolve(partition, block, length)?;
                let total = resolved.iter().try_fold(0u64, |total, extent| {
                    total
                        .checked_add(extent.length)
                        .ok_or_else(|| bad("tail size overflow"))
                })?;
                if total != length {
                    return Err(bad("preallocated tail size mismatch"));
                }
                continue;
            }
            if extended && information == 0 {
                return Err(bad("empty extended allocation in file body"));
            }
            if !previous_length.is_multiple_of(2048) {
                return Err(bad("non-final allocation is not block aligned"));
            }
            previous_length = if extended { information } else { length };
            allocated = allocated
                .checked_add(information)
                .ok_or_else(|| bad("allocation size overflow"))?;
            if kind == 2 {
                // Validate the partition reference without resolving an unallocated address.
                resolve(partition, 0, 0)?;
                charge(budget, std::mem::size_of::<Extent>() as u64, limits)?;
                extents.push(Extent {
                    offset: None,
                    allocated_unrecorded: false,
                    length: information,
                    logical_byte: u64::from(block) * 2048,
                });
            } else {
                let resolved = resolve(partition, block, length)?;
                let mut resolved_length = 0u64;
                let mut remaining_information = information;
                for mut extent in resolved {
                    resolved_length = resolved_length
                        .checked_add(extent.length)
                        .ok_or_else(|| bad("resolved size overflow"))?;
                    extent.length = extent.length.min(remaining_information);
                    remaining_information -= extent.length;
                    if kind == 1 {
                        extent.offset = None;
                        extent.allocated_unrecorded = true;
                    }
                    charge(budget, std::mem::size_of::<Extent>() as u64, limits)?;
                    extents.push(extent);
                }
                if resolved_length != length {
                    return Err(bad("resolved allocation size mismatch"));
                }
            }
        }
        let Some((partition, block, length)) = continuation else {
            break;
        };
        let resolved = resolve(partition, block, length)?;
        continuation_data = read_recorded(bytes, &resolved, length)?;
        tag(&continuation_data, 258, block)?;
        let ad_len = u32_at(&continuation_data, 20)? as usize;
        let crc_len = usize::from(u16_at(&continuation_data, 10)?);
        // UDF 2.3.11 permits CRC protection of just the AED header (8 bytes).
        if crc_len != 8 && crc_len != 8 + ad_len {
            return Err(bad("invalid allocation extent CRC coverage"));
        }
        list = continuation_data
            .get(
                24..24usize
                    .checked_add(ad_len)
                    .ok_or_else(|| bad("allocation extent overflow"))?,
            )
            .ok_or_else(|| bad("allocation extent descriptor range"))?;
    }
    if allocated < file_size
        || allocated - file_size >= 2048
        || ((metadata || allocation_type == 2) && allocated != file_size)
    {
        return Err(bad("allocation size mismatch or excess padding"));
    }
    let mut remaining = file_size;
    for extent in &mut extents {
        extent.length = extent.length.min(remaining);
        remaining -= extent.length;
    }
    extents.retain(|extent| extent.length != 0);
    Ok(extents)
}
