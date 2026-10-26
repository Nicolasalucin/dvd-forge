//! Rock Ridge RRIP attributes and bounded System Use Sharing Protocol parsing.
use crate::iso9660::{Error, Result};
use std::{
    collections::HashSet,
    io::{Read, Seek, SeekFrom},
};

/// POSIX attributes stored in a Rock Ridge PX entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnixMetadata {
    /// POSIX mode, including file-type bits.
    pub mode: u32,
    /// Number of links.
    pub links: u32,
    /// Owner identifier.
    pub uid: u32,
    /// Group identifier.
    pub gid: u32,
    /// RRIP 1.12 file serial number, when present.
    pub serial: Option<u32>,
    /// Raw high and low device-number fields from a PN entry. Their encoding
    /// depends on the producer's Unix device-number representation.
    pub device: Option<(u32, u32)>,
    /// Timestamp fields in ISO short (7-byte) or long (17-byte) representation.
    /// Keys are TF flag bits: creation 1, modification 2, access 4, attributes 8,
    /// backup 16, expiration 32 and effective 64. The last byte is a signed UTC
    /// offset in 15-minute units; these fields describe metadata, not authenticity.
    pub timestamps: Vec<(u8, Vec<u8>)>,
}

#[derive(Default)]
pub(crate) struct Attributes {
    pub name: Option<String>,
    pub metadata: Option<UnixMetadata>,
    pub link: Option<String>,
    pub rrip: bool,
}
fn malformed(message: &str) -> Error {
    Error::Malformed(message.into())
}
fn number(bytes: &[u8]) -> Result<u32> {
    if bytes.len() != 8 {
        return Err(malformed("truncated SUSP integer"));
    }
    let le = u32::from_le_bytes(
        bytes[..4]
            .try_into()
            .map_err(|_| malformed("SUSP integer"))?,
    );
    let be = u32::from_be_bytes(
        bytes[4..]
            .try_into()
            .map_err(|_| malformed("SUSP integer"))?,
    );
    if le != be {
        return Err(malformed("inconsistent SUSP integer byte orders"));
    }
    Ok(le)
}
pub(crate) fn system_use(record: &[u8]) -> Result<&[u8]> {
    let length = usize::from(
        *record
            .get(32)
            .ok_or_else(|| malformed("SUSP directory record"))?,
    );
    record
        .get(33 + length + usize::from(length.is_multiple_of(2))..)
        .ok_or_else(|| malformed("SUSP identifier padding"))
}
pub(crate) fn discover(bytes: &[u8]) -> Option<usize> {
    // XA records may prepend their 14-byte system-use header.
    for prefix in [0, 14] {
        if bytes.get(prefix..prefix + 6) == Some(&[b'S', b'P', 7, 1, 0xbe, 0xef]) {
            return bytes.get(prefix + 6).map(|&byte| usize::from(byte));
        }
    }
    None
}
/// Parse one SUSP record, including bounded, cycle-checked continuation areas.
pub(crate) fn parse<R: Read + Seek>(
    reader: &mut R,
    bytes: &[u8],
    volume_bytes: u64,
    budget: &mut u64,
    maximum: u64,
) -> Result<Attributes> {
    let mut areas = vec![bytes.to_vec()];
    let mut visited = HashSet::new();
    let mut attributes = Attributes::default();
    let mut name = Vec::new();
    let mut name_pending = false;
    let mut name_seen = false;
    let mut components = Vec::new();
    let mut component = Vec::new();
    let mut absolute = false;
    let mut link_pending = false;
    let mut link_seen = false;
    let mut device = None;
    let mut timestamps = Vec::new();
    while let Some(area) = areas.pop() {
        let mut position = 0;
        while position < area.len() {
            if area[position..].iter().all(|&byte| byte == 0) {
                break;
            }
            let header = area
                .get(position..position + 4)
                .ok_or_else(|| malformed("truncated SUSP entry"))?;
            let length = usize::from(header[2]);
            if length < 4 || header[3] != 1 {
                return Err(malformed("invalid SUSP length or version"));
            }
            let entry = area
                .get(position..position + length)
                .ok_or_else(|| malformed("SUSP entry exceeds area"))?;
            position += length;
            match &entry[..2] {
                b"ST" => {
                    if length != 4 {
                        return Err(malformed("invalid SUSP terminator"));
                    }
                    break;
                }
                b"SP" => {
                    if length != 7 || entry[4..6] != [0xbe, 0xef] {
                        return Err(malformed("invalid SUSP indicator"));
                    }
                }
                b"ER" => {
                    if length < 8
                        || length
                            != 8 + usize::from(entry[4])
                                + usize::from(entry[5])
                                + usize::from(entry[6])
                    {
                        return Err(malformed("invalid SUSP extension reference"));
                    }
                    let id = &entry[8..8 + usize::from(entry[4])];
                    if matches!(id, b"RRIP_1991A" | b"IEEE_P1282" | b"IEEE_1282") {
                        attributes.rrip = true;
                    }
                }
                b"CE" => {
                    if length != 28 || visited.len() >= 64 {
                        return Err(malformed("invalid or excessive SUSP continuation chain"));
                    }
                    let block = u64::from(number(&entry[4..12])?);
                    let offset = u64::from(number(&entry[12..20])?);
                    let size = u64::from(number(&entry[20..28])?);
                    let start = block
                        .checked_mul(2048)
                        .and_then(|value| value.checked_add(offset))
                        .ok_or_else(|| malformed("SUSP continuation overflow"))?;
                    if offset >= 2048
                        || size == 0
                        || start.checked_add(size).is_none_or(|end| end > volume_bytes)
                        || !visited.insert((start, size))
                    {
                        return Err(malformed("cyclic or out-of-volume SUSP continuation"));
                    }
                    *budget = budget
                        .checked_add(size)
                        .ok_or(Error::ResourceLimit("Rock Ridge continuation bytes"))?;
                    if *budget > maximum {
                        return Err(Error::ResourceLimit("Rock Ridge continuation bytes"));
                    }
                    let mut next = vec![
                        0;
                        usize::try_from(size).map_err(|_| Error::ResourceLimit(
                            "SUSP continuation size"
                        ))?
                    ];
                    reader.seek(SeekFrom::Start(start))?;
                    reader.read_exact(&mut next)?;
                    // A CE replaces the rest of this area; ST may follow it, but
                    // entries after CE are processed after the referenced area.
                    if position < area.len() {
                        areas.push(area[position..].to_vec());
                    }
                    areas.push(next);
                    break;
                }
                b"PX" => {
                    if !matches!(length, 36 | 44) || attributes.metadata.is_some() {
                        return Err(malformed("invalid or duplicate Rock Ridge PX"));
                    }
                    attributes.metadata = Some(UnixMetadata {
                        mode: number(&entry[4..12])?,
                        links: number(&entry[12..20])?,
                        uid: number(&entry[20..28])?,
                        gid: number(&entry[28..36])?,
                        serial: if length == 44 {
                            Some(number(&entry[36..44])?)
                        } else {
                            None
                        },
                        device: None,
                        timestamps: Vec::new(),
                    });
                }
                b"PN" => {
                    if length != 20 || device.is_some() {
                        return Err(malformed("invalid Rock Ridge PN"));
                    }
                    device = Some((number(&entry[4..12])?, number(&entry[12..20])?));
                }
                b"NM" => {
                    if length < 5 || entry[4] & !7 != 0 || (name_seen && !name_pending) {
                        return Err(malformed("invalid Rock Ridge name continuation"));
                    }
                    name_seen = true;
                    if entry[4] & 6 != 0 {
                        return Err(Error::Unsupported(
                            "Rock Ridge current/parent alternate identifier".into(),
                        ));
                    }
                    name.extend_from_slice(&entry[5..]);
                    name_pending = entry[4] & 1 != 0;
                }
                b"SL" => {
                    if length < 5 || entry[4] & !1 != 0 || (link_seen && !link_pending) {
                        return Err(malformed("invalid Rock Ridge SL continuation"));
                    }
                    link_seen = true;
                    link_pending = entry[4] & 1 != 0;
                    let mut cursor = 5;
                    while cursor < length {
                        let flags = *entry.get(cursor).ok_or_else(|| malformed("SL flags"))?;
                        let size = usize::from(
                            *entry
                                .get(cursor + 1)
                                .ok_or_else(|| malformed("SL component length"))?,
                        );
                        let text = entry
                            .get(cursor + 2..cursor + 2 + size)
                            .ok_or_else(|| malformed("SL component exceeds entry"))?;
                        cursor += 2 + size;
                        match flags {
                            0 | 1 => {
                                if text.is_empty() || text.contains(&0) || text.contains(&b'/') {
                                    return Err(malformed("invalid SL component"));
                                }
                                component.extend_from_slice(text);
                                if flags == 0 {
                                    components.push(
                                        String::from_utf8(std::mem::take(&mut component))
                                            .map_err(|_| malformed("non-UTF8 symbolic link"))?,
                                    );
                                }
                            }
                            2 | 4 | 8 if size == 0 && component.is_empty() => {
                                if flags == 8 {
                                    if absolute || !components.is_empty() {
                                        return Err(malformed("misplaced SL root"));
                                    }
                                    absolute = true;
                                } else {
                                    components.push(if flags == 2 { "." } else { ".." }.into());
                                }
                            }
                            _ => {
                                return Err(Error::Unsupported(
                                    "Rock Ridge symbolic link component flags".into(),
                                ));
                            }
                        }
                    }
                }
                b"TF" => {
                    if length < 5 {
                        return Err(malformed("invalid Rock Ridge TF"));
                    }
                    let width = if entry[4] & 0x80 != 0 { 17 } else { 7 };
                    if length != 5 + width * (entry[4] & 0x7f).count_ones() as usize {
                        return Err(malformed("invalid TF timestamp count"));
                    }
                    let mut cursor = 5;
                    for bit in [1, 2, 4, 8, 16, 32, 64] {
                        if entry[4] & bit != 0 {
                            if timestamps.iter().any(|(key, _)| *key == bit) {
                                return Err(malformed("duplicate TF timestamp"));
                            }
                            timestamps.push((bit, entry[cursor..cursor + width].to_vec()));
                            cursor += width;
                        }
                    }
                }
                b"CL" | b"PL" | b"RE" => {
                    return Err(Error::Unsupported(
                        "Rock Ridge relocated directories".into(),
                    ));
                }
                b"ZF" => {
                    return Err(Error::Unsupported(
                        "Rock Ridge compressed zisofs payload".into(),
                    ));
                }
                _ => {}
            }
        }
    }
    if name_pending || link_pending || !component.is_empty() {
        return Err(malformed("unfinished Rock Ridge continuation"));
    }
    if name_seen {
        if name.is_empty() || name.iter().any(|&b| matches!(b, 0 | b'/' | b'\\')) {
            return Err(malformed("invalid Rock Ridge leaf name"));
        }
        attributes.name =
            Some(String::from_utf8(name).map_err(|_| malformed("non-UTF8 Rock Ridge name"))?);
    }
    if link_seen {
        attributes.link = Some(format!(
            "{}{}",
            if absolute { "/" } else { "" },
            components.join("/")
        ));
    }
    if let Some(metadata) = &mut attributes.metadata {
        metadata.device = device;
        metadata.timestamps = timestamps;
    } else if device.is_some() || !timestamps.is_empty() {
        return Err(malformed("Rock Ridge attributes without PX"));
    }
    Ok(attributes)
}

#[cfg(feature = "native-writer")]
pub(crate) mod writer {
    use super::UnixMetadata;
    fn number(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend(value.to_le_bytes());
        bytes.extend(value.to_be_bytes());
    }
    pub fn attributes(
        metadata: &UnixMetadata,
        timestamp: [u8; 7],
        name: Option<&str>,
        link: Option<&str>,
    ) -> Vec<u8> {
        let mut bytes = vec![b'P', b'X', 44, 1];
        for value in [
            metadata.mode,
            metadata.links,
            metadata.uid,
            metadata.gid,
            metadata.serial.unwrap_or(0),
        ] {
            number(&mut bytes, value);
        }
        bytes.extend([b'T', b'F', 26, 1, 0x0e]);
        for _ in 0..3 {
            bytes.extend(timestamp);
        }
        if let Some(name) = name {
            let chunks: Vec<_> = name.as_bytes().chunks(250).collect();
            for (index, chunk) in chunks.iter().enumerate() {
                bytes.extend([
                    b'N',
                    b'M',
                    (5 + chunk.len()) as u8,
                    1,
                    u8::from(index + 1 < chunks.len()),
                ]);
                bytes.extend_from_slice(chunk);
            }
        }
        if let Some(link) = link {
            let mut parts = Vec::new();
            if link.starts_with('/') {
                parts.push(vec![8, 0]);
            }
            for part in link.split('/').filter(|part| !part.is_empty()) {
                if matches!(part, "." | "..") {
                    parts.push(vec![if part == "." { 2 } else { 4 }, 0]);
                } else {
                    let chunks: Vec<_> = part.as_bytes().chunks(248).collect();
                    for (index, chunk) in chunks.iter().enumerate() {
                        let mut item = vec![u8::from(index + 1 < chunks.len()), chunk.len() as u8];
                        item.extend_from_slice(chunk);
                        parts.push(item);
                    }
                }
            }
            let mut groups = vec![Vec::new()];
            for part in parts {
                if groups
                    .last()
                    .is_some_and(|group| group.len() + part.len() > 250)
                {
                    groups.push(Vec::new());
                }
                if let Some(group) = groups.last_mut() {
                    group.extend(part);
                }
            }
            for (index, group) in groups.iter().enumerate() {
                bytes.extend([
                    b'S',
                    b'L',
                    (5 + group.len()) as u8,
                    1,
                    u8::from(index + 1 < groups.len()),
                ]);
                bytes.extend(group);
            }
        }
        bytes
    }
    pub fn discovery() -> Vec<u8> {
        let mut bytes = vec![
            b'S', b'P', 7, 1, 0xbe, 0xef, 0, b'E', b'R', 18, 1, 10, 0, 0, 1,
        ];
        bytes.extend(b"RRIP_1991A");
        bytes
    }
    pub fn continuation(block: u32, offset: u32, size: u32) -> Vec<u8> {
        let mut bytes = vec![b'C', b'E', 28, 1];
        for value in [block, offset, size] {
            number(&mut bytes, value);
        }
        bytes
    }
}
