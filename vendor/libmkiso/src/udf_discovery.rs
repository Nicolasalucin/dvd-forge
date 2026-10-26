//! Bounded anchor, reserve-sequence and prevailing volume descriptor discovery.
use super::{Error, Limits, Result, bad, charge_metadata, region, tag, u16_at, u32_at};
use std::collections::{BTreeMap, HashSet};
const BLOCK: u64 = 2048;
const MAX_SEQUENCE_BLOCKS: u32 = 256;
const MAX_SEQUENCE_LINKS: usize = 64;

pub(super) struct Descriptors {
    pub partitions: Vec<Vec<u8>>,
    pub logical: Vec<u8>,
}
fn protected(bytes: &[u8], minimum: usize) -> Result<()> {
    if 16 + usize::from(u16_at(bytes, 10)?) < minimum {
        return Err(bad("descriptor CRC does not cover volume discovery fields"));
    }
    Ok(())
}
fn sequence(
    bytes: &dyn crate::source::ReadAt,
    start: u32,
    length: u32,
    limits: Limits,
    budget: &mut u64,
) -> Result<Descriptors> {
    let mut extent = (start, length);
    let mut seen = HashSet::new();
    let mut partitions = BTreeMap::<u16, (u32, Vec<u8>)>::new();
    let mut logical: Option<(u32, Vec<u8>)> = None;
    let mut total_blocks = 0u32;
    loop {
        if seen.len() >= MAX_SEQUENCE_LINKS || !seen.insert(extent) {
            return Err(bad("volume descriptor sequence chain cycle or limit"));
        }
        let (start, length) = extent;
        let count = length / BLOCK as u32;
        total_blocks = total_blocks
            .checked_add(count)
            .ok_or_else(|| bad("volume sequence overflow"))?;
        if length == 0 || !length.is_multiple_of(BLOCK as u32) || total_blocks > MAX_SEQUENCE_BLOCKS
        {
            return Err(bad("unbounded volume descriptor sequence"));
        }
        super::check_region(bytes, u64::from(start) * BLOCK, u64::from(length))?;
        let mut next = None;
        let mut terminated = false;
        for index in 0..count {
            let location = start
                .checked_add(index)
                .ok_or_else(|| bad("volume descriptor location overflow"))?;
            charge_metadata(budget, BLOCK, limits)?;
            let descriptor = region(bytes, u64::from(location) * BLOCK, BLOCK)?;
            let kind = u16_at(&descriptor, 0)?;
            tag(&descriptor, kind, location)?;
            match kind {
                3 => {
                    protected(&descriptor, 28)?;
                    next = Some((u32_at(&descriptor, 24)?, u32_at(&descriptor, 20)?));
                    break;
                }
                5 => {
                    protected(&descriptor, 196)?;
                    let number = u16_at(&descriptor, 22)?;
                    let serial = u32_at(&descriptor, 16)?;
                    if let Some((previous, _)) = partitions.get(&number)
                        && serial < *previous
                    {
                        continue;
                    }
                    partitions.insert(number, (serial, descriptor));
                }
                6 => {
                    protected(&descriptor, 20)?;
                    let serial = u32_at(&descriptor, 16)?;
                    if logical
                        .as_ref()
                        .is_none_or(|(previous, _)| serial >= *previous)
                    {
                        logical = Some((serial, descriptor));
                    }
                }
                8 => {
                    terminated = true;
                    break;
                }
                1 | 4 | 7 => {}
                _ => return Err(Error::Unsupported("UDF volume descriptor kind".into())),
            }
        }
        if let Some(next) = next {
            extent = next;
            continue;
        }
        if !terminated {
            return Err(bad("volume descriptor sequence lacks terminator"));
        }
        break;
    }
    if partitions.is_empty() {
        return Err(bad("missing partition descriptor"));
    }
    let logical = logical
        .ok_or_else(|| bad("missing logical volume descriptor"))?
        .1;
    Ok(Descriptors {
        partitions: partitions
            .into_values()
            .map(|(_, descriptor)| descriptor)
            .collect(),
        logical,
    })
}

pub(super) fn discover(
    bytes: &dyn crate::source::ReadAt,
    limits: Limits,
    budget: &mut u64,
) -> Result<Descriptors> {
    let blocks = bytes.len() / BLOCK;
    let mut candidates = vec![256u64];
    if let Some(last) = blocks.checked_sub(1) {
        candidates.extend([last, last.saturating_sub(256)]);
    }
    let mut seen = HashSet::new();
    let mut last_error = None;
    for location in candidates {
        if !seen.insert(location) {
            continue;
        }
        let anchor = match region(bytes, location * BLOCK, BLOCK) {
            Ok(anchor) => anchor,
            Err(error @ Error::Io(_)) => return Err(error),
            Err(error) => {
                last_error = Some(error);
                continue;
            }
        };
        let logical_location = match u32::try_from(location) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Err(error) = tag(&anchor, 2, logical_location).and_then(|()| protected(&anchor, 32))
        {
            last_error = Some(error);
            continue;
        }
        for offset in [16, 24] {
            let length = u32_at(&anchor, offset)?;
            let start = u32_at(&anchor, offset + 4)?;
            if length == 0 {
                continue;
            }
            match sequence(bytes, start, length, limits, budget) {
                Ok(descriptors) => return Ok(descriptors),
                Err(error @ (Error::ResourceLimit(_) | Error::Io(_))) => return Err(error),
                Err(error) => last_error = Some(error),
            }
        }
    }
    Err(last_error.unwrap_or_else(|| bad("no usable UDF anchor or volume sequence")))
}
