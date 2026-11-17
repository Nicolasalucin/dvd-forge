//! Native DVD-Video image writer (UDF 1.02 + ISO 9660 "UDF Bridge").
//!
//! DVD-Video players read the disc through UDF 1.02, while computers and
//! some players use the ISO 9660 view; both views point at the same sectors.
//! The IFO files store sector offsets that assume the canonical layout, so
//! the payload is written contiguously, without gaps, in this order:
//!
//! ```text
//! VIDEO_TS.IFO VIDEO_TS.VOB VIDEO_TS.BUP
//! VTS_01_0.IFO VTS_01_0.VOB VTS_01_1.VOB … VTS_01_0.BUP   (per title set)
//! ```
//!
//! This replaces `genisoimage -dvd-video -udf` / `mkisofs`, which are not
//! available on Windows. Layout (2048-byte sectors):
//!
//! ```text
//! 16      ISO 9660 primary volume descriptor     17 terminator
//! 18-20   BEA01 NSR02 TEA01
//! 32-37   UDF main volume descriptor sequence    48-53 reserve copy
//! 64-65   logical volume integrity sequence
//! 256     anchor volume descriptor pointer
//! 257-…   ISO path tables and directories
//! P…      UDF partition: FSD, TD, directory FEs/FIDs, file FEs, payload
//! last    anchor volume descriptor pointer
//! ```
//!
//! All sector math is `u64`; payload is streamed with a 1 MiB buffer.

use crate::{IsoError, IsoReport, SECTOR_BYTES};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const SECTOR: usize = SECTOR_BYTES as usize;
const MAIN_VDS: u32 = 32;
const RESERVE_VDS: u32 = 48;
const VDS_SECTORS: u32 = 16;
const LVID: u32 = 64;
const ANCHOR: u32 = 256;
const ISO_START: u32 = 257;
/// Largest short_ad extent (30-bit length, sector aligned).
const MAX_EXTENT: u64 = 0x3FFF_F800;
const IO_BUFFER: usize = 1024 * 1024;

const TAG_PVD: u16 = 1;
const TAG_AVDP: u16 = 2;
const TAG_IUVD: u16 = 4;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_USD: u16 = 7;
const TAG_TD: u16 = 8;
const TAG_LVID: u16 = 9;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_FE: u16 = 261;

/// One file of VIDEO_TS, in disc order.
#[derive(Debug, Clone)]
struct DvdFile {
    name: String,
    source: PathBuf,
    size: u64,
}

/// Canonical DVD-Video order key, or `None` for a name that does not belong
/// in VIDEO_TS.
fn order_key(name: &str) -> Option<(u32, u32, u32)> {
    let upper = name.to_ascii_uppercase();
    let (stem, extension) = upper.split_once('.')?;
    let kind = match extension {
        "IFO" => 0,
        "VOB" => 1,
        "BUP" => 2,
        _ => return None,
    };
    if stem == "VIDEO_TS" {
        return Some((0, kind, 0));
    }
    // VTS_nn_m
    let rest = stem.strip_prefix("VTS_")?;
    let (set, part) = rest.split_once('_')?;
    if set.len() != 2 || part.len() != 1 {
        return None;
    }
    let set: u32 = set.parse().ok()?;
    let part: u32 = part.parse().ok()?;
    if !(1..=99).contains(&set) || (kind != 1 && part != 0) {
        return None;
    }
    Some((set, kind, part))
}

fn collect_files(video_ts: &Path) -> Result<Vec<DvdFile>, IsoError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(video_ts)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_uppercase();
        if !entry.file_type()?.is_file() {
            return Err(IsoError::UnreadableIso(format!(
                "VIDEO_TS contiene una carpeta inesperada: {name}"
            )));
        }
        if order_key(&name).is_none() {
            return Err(IsoError::UnreadableIso(format!(
                "VIDEO_TS contiene un archivo que no es de DVD-Video: {name}"
            )));
        }
        let size = entry.metadata()?.len();
        if size > u64::from(u32::MAX) {
            return Err(IsoError::UnreadableIso(format!(
                "{name} supera 4 GiB, imposible en DVD-Video"
            )));
        }
        files.push(DvdFile {
            name,
            source: entry.path(),
            size,
        });
    }
    files.sort_by_key(|file| order_key(&file.name));
    if !files.iter().any(|file| file.name == "VIDEO_TS.IFO") {
        return Err(IsoError::MissingIsoEntry("VIDEO_TS/VIDEO_TS.IFO".into()));
    }
    Ok(files)
}

fn sectors(bytes: u64) -> u64 {
    bytes.div_ceil(SECTOR_BYTES)
}

fn sector32(value: u64) -> Result<u32, IsoError> {
    u32::try_from(value)
        .map_err(|_| IsoError::UnreadableIso("la imagen supera el límite de sectores".into()))
}

// ───────────────────────── byte helpers ─────────────────────────

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn both16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    b[at + 2..at + 4].copy_from_slice(&v.to_be_bytes());
}
fn both32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    b[at + 4..at + 8].copy_from_slice(&v.to_be_bytes());
}

/// CRC-ITU-T (polynomial 0x1021, initial 0) as required by ECMA-167 7.2.6.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Fills the 16-byte descriptor tag of `b` (ECMA-167 3/7.2).
fn tag(b: &mut [u8], id: u16, location: u32, crc_len: usize) {
    put16(b, 0, id);
    put16(b, 2, 2);
    b[5] = 0;
    put16(b, 6, 1);
    let crc = crc16(&b[16..16 + crc_len]);
    put16(b, 8, crc);
    put16(b, 10, crc_len as u16);
    put32(b, 12, location);
    b[4] = 0;
    b[4] = b[..16]
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 4)
        .fold(0u8, |sum, (_, value)| sum.wrapping_add(*value));
}

fn charspec(b: &mut [u8], at: usize) {
    b[at] = 0;
    let text = b"OSTA Compressed Unicode";
    b[at + 1..at + 1 + text.len()].copy_from_slice(text);
}

/// d-string of `len` bytes with OSTA CS0 8-bit compression.
fn dstring(b: &mut [u8], at: usize, len: usize, text: &str) {
    let bytes: Vec<u8> = text.bytes().take(len - 2).collect();
    if bytes.is_empty() {
        return;
    }
    b[at] = 8;
    b[at + 1..at + 1 + bytes.len()].copy_from_slice(&bytes);
    b[at + len - 1] = (bytes.len() + 1) as u8;
}

fn regid(b: &mut [u8], at: usize, id: &[u8], suffix: &[u8]) {
    b[at] = 0;
    b[at + 1..at + 1 + id.len()].copy_from_slice(id);
    b[at + 24..at + 24 + suffix.len()].copy_from_slice(suffix);
}

/// UDF revision 1.02 in identifier suffixes.
const UDF_SUFFIX: [u8; 2] = [0x02, 0x01];
const IMPL_ID: &[u8] = b"*Kinodisc";

#[derive(Debug, Clone, Copy)]
struct Clock {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl Clock {
    /// `SOURCE_DATE_EPOCH` when set (reproducible builds), else now (UTC).
    fn now() -> Self {
        let seconds = std::env::var("SOURCE_DATE_EPOCH")
            .ok()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs() as i64)
                    .unwrap_or(0)
            })
            .max(0);
        Self::from_unix(seconds)
    }

    fn from_unix(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let rest = seconds.rem_euclid(86_400);
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year: year.clamp(1970, 9999) as u16,
            month: month as u8,
            day: day as u8,
            hour: (rest / 3_600) as u8,
            minute: ((rest / 60) % 60) as u8,
            second: (rest % 60) as u8,
        }
    }

    fn udf(&self, b: &mut [u8], at: usize) {
        put16(b, at, 0x1000); // type 1 (local), UTC offset 0
        put16(b, at + 2, self.year);
        b[at + 4] = self.month;
        b[at + 5] = self.day;
        b[at + 6] = self.hour;
        b[at + 7] = self.minute;
        b[at + 8] = self.second;
    }

    fn iso7(&self, b: &mut [u8], at: usize) {
        b[at] = (self.year.saturating_sub(1900)).min(255) as u8;
        b[at + 1] = self.month;
        b[at + 2] = self.day;
        b[at + 3] = self.hour;
        b[at + 4] = self.minute;
        b[at + 5] = self.second;
        b[at + 6] = 0;
    }

    fn iso17(&self, b: &mut [u8], at: usize) {
        let text = format!(
            "{:04}{:02}{:02}{:02}{:02}{:02}00",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        );
        b[at..at + 16].copy_from_slice(text.as_bytes());
        b[at + 16] = 0;
    }
}

/// ISO 9660 d-characters for the volume identifier.
fn iso_volume_id(label: &str) -> String {
    label
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(32)
        .collect()
}

// ───────────────────────── ISO 9660 ─────────────────────────

/// Builds one directory record; returns its bytes.
fn iso_record(name: &[u8], extent: u32, size: u32, directory: bool, clock: &Clock) -> Vec<u8> {
    let len = 33 + name.len() + usize::from(name.len().is_multiple_of(2));
    let mut r = vec![0u8; len];
    r[0] = len as u8;
    both32(&mut r, 2, extent);
    both32(&mut r, 10, size);
    clock.iso7(&mut r, 18);
    r[25] = if directory { 2 } else { 0 };
    both16(&mut r, 28, 1);
    r[32] = name.len() as u8;
    r[33..33 + name.len()].copy_from_slice(name);
    r
}

/// Concatenates records into whole sectors (records never cross sectors).
fn iso_directory(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut used = 0usize;
    for record in records {
        if used + record.len() > SECTOR {
            out.resize(out.len() + (SECTOR - used), 0);
            used = 0;
        }
        out.extend_from_slice(record);
        used += record.len();
    }
    let padded = out.len().div_ceil(SECTOR).max(1) * SECTOR;
    out.resize(padded, 0);
    out
}

fn iso_directory_sectors(records: &[Vec<u8>]) -> u64 {
    (iso_directory(records).len() / SECTOR) as u64
}

// ───────────────────────── UDF ─────────────────────────

/// File Identifier Descriptor bytes (ECMA-167 4/14.4).
fn fid(
    name: Option<&str>,
    directory: bool,
    icb_lbn: u32,
    unique_id: u32,
    tag_location: u32,
) -> Vec<u8> {
    let identifier: Vec<u8> = match name {
        Some(name) => std::iter::once(8u8).chain(name.bytes()).collect(),
        None => Vec::new(),
    };
    let len = (38 + identifier.len()).div_ceil(4) * 4;
    let mut b = vec![0u8; len];
    put16(&mut b, 16, 1);
    b[18] = if name.is_none() {
        0x0A // directory + parent
    } else if directory {
        0x02
    } else {
        0x00
    };
    b[19] = identifier.len() as u8;
    // ICB long_ad: extent length, lb_addr (lbn, partition), impl use.
    put32(&mut b, 20, SECTOR as u32);
    put32(&mut b, 24, icb_lbn);
    put16(&mut b, 28, 0);
    put32(&mut b, 32, unique_id); // UDF unique ID in impl-use bytes 2..6
    put16(&mut b, 36, 0);
    b[38..38 + identifier.len()].copy_from_slice(&identifier);
    tag(&mut b, TAG_FID, tag_location, len - 16);
    b
}

/// Packs FIDs into directory blocks starting at `first_lbn`; FIDs never
/// cross a block boundary (simpler readers rely on that).
fn fid_blocks(entries: &[(Option<String>, bool, u32, u32)], first_lbn: u32) -> (Vec<u8>, u64) {
    let mut out: Vec<u8> = Vec::new();
    for (name, directory, icb, unique) in entries {
        let probe = fid(name.as_deref(), *directory, *icb, *unique, 0);
        let used = out.len() % SECTOR;
        if used != 0 && used + probe.len() > SECTOR {
            out.resize(out.len() + (SECTOR - used), 0);
        }
        let location = first_lbn + (out.len() / SECTOR) as u32;
        out.extend_from_slice(&fid(name.as_deref(), *directory, *icb, *unique, location));
    }
    let information_length = out.len() as u64;
    let padded = out.len().div_ceil(SECTOR).max(1) * SECTOR;
    out.resize(padded, 0);
    (out, information_length)
}

/// File Entry (ECMA-167 4/14.9) with short allocation descriptors.
#[allow(clippy::too_many_arguments)]
fn file_entry(
    location: u32,
    directory: bool,
    link_count: u16,
    unique_id: u64,
    information_length: u64,
    first_data_lbn: u32,
    clock: &Clock,
) -> Vec<u8> {
    let mut extents = Vec::new();
    let mut remaining = information_length;
    let mut lbn = u64::from(first_data_lbn);
    while remaining > 0 {
        let length = remaining.min(MAX_EXTENT);
        extents.push((length as u32, lbn as u32));
        lbn += sectors(length);
        remaining -= length;
    }
    let ad_len = extents.len() * 8;
    let total = 176 + ad_len;
    let mut b = vec![0u8; SECTOR];
    // ICB tag
    put16(&mut b, 16 + 4, 4); // strategy 4
    put16(&mut b, 16 + 8, 1); // max entries
    b[16 + 11] = if directory { 4 } else { 5 };
    put16(&mut b, 16 + 18, 0); // flags: short_ad
    put32(&mut b, 36, u32::MAX); // uid
    put32(&mut b, 40, u32::MAX); // gid
    let permissions: u32 = if directory { 0x14A5 } else { 0x1084 };
    put32(&mut b, 44, permissions);
    put16(&mut b, 48, link_count);
    put64(&mut b, 56, information_length);
    put64(&mut b, 64, sectors(information_length));
    clock.udf(&mut b, 72);
    clock.udf(&mut b, 84);
    clock.udf(&mut b, 96);
    put32(&mut b, 108, 1); // checkpoint
    regid(&mut b, 128, IMPL_ID, &[]);
    put64(&mut b, 160, unique_id);
    put32(&mut b, 168, 0);
    put32(&mut b, 172, ad_len as u32);
    for (index, (length, position)) in extents.iter().enumerate() {
        put32(&mut b, 176 + index * 8, *length);
        put32(&mut b, 176 + index * 8 + 4, *position);
    }
    tag(&mut b[..total], TAG_FE, location, total - 16);
    b
}

/// Plan of every structure's position, computed before writing.
struct Layout {
    files: Vec<DvdFile>,
    iso_root: u32,
    iso_audio: u32,
    iso_video: u32,
    iso_video_sectors: u32,
    partition_start: u32,
    partition_length: u32,
    total_sectors: u64,
    root_fe: u32,
    root_fids: u32,
    video_fe: u32,
    video_fids: u32,
    video_fid_sectors: u32,
    audio_fe: u32,
    audio_fids: u32,
    file_fes: u32,
    /// Partition-relative first block of each file's data.
    data: Vec<u32>,
}

fn file_records(files: &[DvdFile], clock: &Clock) -> Vec<Vec<u8>> {
    // Sizes only matter for counting sectors here.
    let mut names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    names.sort_unstable();
    let mut records = vec![
        iso_record(&[0], 0, 0, true, clock),
        iso_record(&[1], 0, 0, true, clock),
    ];
    for name in names {
        records.push(iso_record(
            format!("{name};1").as_bytes(),
            0,
            0,
            false,
            clock,
        ));
    }
    records
}

fn video_fid_entries(files: &[DvdFile]) -> Vec<(Option<String>, bool, u32, u32)> {
    std::iter::once((None, true, 0, 0))
        .chain(
            files
                .iter()
                .map(|file| (Some(file.name.clone()), false, 0, 0)),
        )
        .collect()
}

fn plan(files: Vec<DvdFile>, clock: &Clock) -> Result<Layout, IsoError> {
    let iso_root = ISO_START + 2;
    let iso_audio = iso_root + 1;
    let iso_video = iso_audio + 1;
    let iso_video_sectors = sector32(iso_directory_sectors(&file_records(&files, clock)))?;
    let partition_start = iso_video + iso_video_sectors;

    // Partition-relative blocks: 0 FSD, 1 TD.
    let root_fe = 2;
    let root_fids = 3;
    let video_fe = 4;
    let video_fids = 5;
    let (video_blocks, _) = fid_blocks(&video_fid_entries(&files), 0);
    let video_fid_sectors = (video_blocks.len() / SECTOR) as u32;
    let audio_fe = video_fids + video_fid_sectors;
    let audio_fids = audio_fe + 1;
    let file_fes = audio_fids + 1;
    let mut next = u64::from(file_fes) + files.len() as u64;
    let mut data = Vec::with_capacity(files.len());
    for file in &files {
        data.push(sector32(next)?);
        next += sectors(file.size);
    }
    let partition_length = sector32(next)?;
    let total_sectors = u64::from(partition_start) + next + 1; // + final AVDP
    Ok(Layout {
        files,
        iso_root,
        iso_audio,
        iso_video,
        iso_video_sectors,
        partition_start,
        partition_length,
        total_sectors,
        root_fe,
        root_fids,
        video_fe,
        video_fids,
        video_fid_sectors,
        audio_fe,
        audio_fids,
        file_fes,
        data,
    })
}

fn vds(layout: &Layout, label: &str, clock: &Clock, first: u32) -> Vec<u8> {
    let mut out = vec![0u8; SECTOR * 6];
    let at = |index: usize| index * SECTOR;
    // Primary Volume Descriptor
    {
        let b = &mut out[at(0)..at(1)];
        put32(b, 16, 1);
        put32(b, 20, 0);
        dstring(b, 24, 32, label);
        put16(b, 56, 1);
        put16(b, 58, 1);
        put16(b, 60, 2);
        put16(b, 62, 2);
        put32(b, 64, 1);
        put32(b, 68, 1);
        dstring(b, 72, 128, &format!("{:016X}{}", 0x00D0_F06E_u64, label));
        charspec(b, 200);
        charspec(b, 264);
        regid(b, 344, b"", &[]);
        clock.udf(b, 376);
        regid(b, 388, IMPL_ID, &[]);
        tag(&mut b[..512], TAG_PVD, first, 496);
    }
    // Implementation Use Volume Descriptor (LV Info)
    {
        let b = &mut out[at(1)..at(2)];
        put32(b, 16, 2);
        regid(b, 20, b"*UDF LV Info", &UDF_SUFFIX);
        charspec(b, 52);
        dstring(b, 116, 128, label);
        regid(b, 352, IMPL_ID, &[]);
        tag(&mut b[..512], TAG_IUVD, first + 1, 496);
    }
    // Partition Descriptor
    {
        let b = &mut out[at(2)..at(3)];
        put32(b, 16, 3);
        put16(b, 20, 1);
        put16(b, 22, 0);
        regid(b, 24, b"+NSR02", &[]);
        put32(b, 184, 1); // read-only
        put32(b, 188, layout.partition_start);
        put32(b, 192, layout.partition_length);
        regid(b, 196, IMPL_ID, &[]);
        tag(&mut b[..512], TAG_PD, first + 2, 496);
    }
    // Logical Volume Descriptor
    {
        let b = &mut out[at(3)..at(4)];
        put32(b, 16, 4);
        charspec(b, 20);
        dstring(b, 84, 128, label);
        put32(b, 212, SECTOR as u32);
        regid(b, 216, b"*OSTA UDF Compliant", &[0x02, 0x01, 0x00]);
        // FSD long_ad: length, lbn 0, partition 0.
        put32(b, 248, SECTOR as u32);
        put32(b, 252, 0);
        put16(b, 256, 0);
        put32(b, 264, 6);
        put32(b, 268, 1);
        regid(b, 272, IMPL_ID, &[]);
        put32(b, 432, (SECTOR * 2) as u32);
        put32(b, 436, LVID);
        // Type 1 partition map.
        b[440] = 1;
        b[441] = 6;
        put16(b, 442, 1);
        put16(b, 444, 0);
        tag(&mut b[..446], TAG_LVD, first + 3, 430);
    }
    // Unallocated Space Descriptor
    {
        let b = &mut out[at(4)..at(5)];
        put32(b, 16, 5);
        put32(b, 20, 0);
        tag(&mut b[..24], TAG_USD, first + 4, 8);
    }
    // Terminating Descriptor
    {
        let b = &mut out[at(5)..at(6)];
        tag(&mut b[..512], TAG_TD, first + 5, 496);
    }
    out
}

fn anchor(location: u32) -> Vec<u8> {
    let mut b = vec![0u8; SECTOR];
    put32(&mut b, 16, VDS_SECTORS * SECTOR as u32);
    put32(&mut b, 20, MAIN_VDS);
    put32(&mut b, 24, VDS_SECTORS * SECTOR as u32);
    put32(&mut b, 28, RESERVE_VDS);
    tag(&mut b[..512], TAG_AVDP, location, 496);
    b
}

/// Builds every non-payload sector from 0 up to the first file's data.
fn header(layout: &Layout, label: &str, clock: &Clock) -> Result<Vec<u8>, IsoError> {
    let p = layout.partition_start;
    let metadata_end = p + layout.data.first().copied().unwrap_or(layout.file_fes);
    let mut image = vec![0u8; metadata_end as usize * SECTOR];
    let sector = |image: &mut Vec<u8>, lba: u32, bytes: &[u8]| {
        let start = lba as usize * SECTOR;
        image[start..start + bytes.len()].copy_from_slice(bytes);
    };
    let total = sector32(layout.total_sectors)?;

    // ISO 9660 directories and path tables.
    let iso_files: Vec<(&DvdFile, u32)> = layout
        .files
        .iter()
        .zip(&layout.data)
        .map(|(file, lbn)| (file, p + lbn))
        .collect();
    let mut sorted = iso_files.clone();
    sorted.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    let video_size = layout.iso_video_sectors * SECTOR as u32;
    let mut video_records = vec![
        iso_record(&[0], layout.iso_video, video_size, true, clock),
        iso_record(&[1], layout.iso_root, SECTOR as u32, true, clock),
    ];
    for (file, extent) in &sorted {
        video_records.push(iso_record(
            format!("{};1", file.name).as_bytes(),
            if file.size == 0 { 0 } else { *extent },
            file.size as u32,
            false,
            clock,
        ));
    }
    let root_records = vec![
        iso_record(&[0], layout.iso_root, SECTOR as u32, true, clock),
        iso_record(&[1], layout.iso_root, SECTOR as u32, true, clock),
        iso_record(b"AUDIO_TS", layout.iso_audio, SECTOR as u32, true, clock),
        iso_record(b"VIDEO_TS", layout.iso_video, video_size, true, clock),
    ];
    let audio_records = vec![
        iso_record(&[0], layout.iso_audio, SECTOR as u32, true, clock),
        iso_record(&[1], layout.iso_root, SECTOR as u32, true, clock),
    ];
    sector(&mut image, layout.iso_root, &iso_directory(&root_records));
    sector(&mut image, layout.iso_audio, &iso_directory(&audio_records));
    sector(&mut image, layout.iso_video, &iso_directory(&video_records));

    let path_entry = |name: &[u8], extent: u32, parent: u16, big: bool| {
        let mut e = vec![0u8; 8 + name.len() + name.len() % 2];
        e[0] = name.len() as u8;
        if big {
            e[2..6].copy_from_slice(&extent.to_be_bytes());
            e[6..8].copy_from_slice(&parent.to_be_bytes());
        } else {
            e[2..6].copy_from_slice(&extent.to_le_bytes());
            e[6..8].copy_from_slice(&parent.to_le_bytes());
        }
        e[8..8 + name.len()].copy_from_slice(name);
        e
    };
    let table = |big: bool| {
        let mut t = Vec::new();
        t.extend(path_entry(&[0], layout.iso_root, 1, big));
        t.extend(path_entry(b"AUDIO_TS", layout.iso_audio, 1, big));
        t.extend(path_entry(b"VIDEO_TS", layout.iso_video, 1, big));
        t
    };
    let l_table = table(false);
    let m_table = table(true);
    sector(&mut image, ISO_START, &l_table);
    sector(&mut image, ISO_START + 1, &m_table);

    // ISO primary volume descriptor + terminator.
    {
        let mut b = vec![0u8; SECTOR];
        b[0] = 1;
        b[1..6].copy_from_slice(b"CD001");
        b[6] = 1;
        let mut system = [b' '; 32];
        system[..9].copy_from_slice(b"DVD FORGE");
        b[8..40].copy_from_slice(&system);
        let mut volume = [b' '; 32];
        let id = iso_volume_id(label);
        volume[..id.len()].copy_from_slice(id.as_bytes());
        b[40..72].copy_from_slice(&volume);
        both32(&mut b, 80, total);
        both16(&mut b, 120, 1);
        both16(&mut b, 124, 1);
        both16(&mut b, 128, SECTOR as u16);
        both32(&mut b, 132, l_table.len() as u32);
        put32(&mut b, 140, ISO_START);
        b[148..152].copy_from_slice(&(ISO_START + 1).to_be_bytes());
        let root = iso_record(&[0], layout.iso_root, SECTOR as u32, true, clock);
        b[156..156 + 34].copy_from_slice(&root[..34]);
        for field in [190usize, 318, 446, 574] {
            b[field..field + 128].fill(b' ');
        }
        b[574..574 + 9].copy_from_slice(b"DVD FORGE");
        for field in [702usize, 739, 776] {
            b[field..field + 37].fill(b' ');
        }
        clock.iso17(&mut b, 813);
        clock.iso17(&mut b, 830);
        b[847..863].fill(b'0');
        b[864..880].fill(b'0');
        b[881] = 1;
        sector(&mut image, 16, &b);
        let mut terminator = vec![0u8; SECTOR];
        terminator[0] = 255;
        terminator[1..6].copy_from_slice(b"CD001");
        terminator[6] = 1;
        sector(&mut image, 17, &terminator);
    }
    // UDF volume recognition sequence.
    for (lba, id) in [(18u32, b"BEA01"), (19, b"NSR02"), (20, b"TEA01")] {
        let mut b = vec![0u8; SECTOR];
        b[1..6].copy_from_slice(id);
        b[6] = 1;
        sector(&mut image, lba, &b);
    }
    sector(&mut image, MAIN_VDS, &vds(layout, label, clock, MAIN_VDS));
    sector(
        &mut image,
        RESERVE_VDS,
        &vds(layout, label, clock, RESERVE_VDS),
    );

    // Logical Volume Integrity Descriptor (closed) + terminator.
    let next_unique = 16 + 2 + layout.files.len() as u64 + 1;
    {
        let mut b = vec![0u8; SECTOR];
        clock.udf(&mut b, 16);
        put32(&mut b, 28, 1);
        put64(&mut b, 40, next_unique);
        put32(&mut b, 72, 1);
        put32(&mut b, 76, 46);
        put32(&mut b, 80, 0);
        put32(&mut b, 84, layout.partition_length);
        regid(&mut b, 88, IMPL_ID, &[]);
        put32(&mut b, 120, layout.files.len() as u32);
        put32(&mut b, 124, 3);
        put16(&mut b, 128, 0x0102);
        put16(&mut b, 130, 0x0102);
        put16(&mut b, 132, 0x0102);
        tag(&mut b[..134], TAG_LVID, LVID, 118);
        sector(&mut image, LVID, &b);
        let mut td = vec![0u8; SECTOR];
        tag(&mut td[..512], TAG_TD, LVID + 1, 496);
        sector(&mut image, LVID + 1, &td);
    }
    sector(&mut image, ANCHOR, &anchor(ANCHOR));

    // Partition contents (tag locations are partition-relative).
    let in_partition = |image: &mut Vec<u8>, lbn: u32, bytes: &[u8]| {
        let start = (p + lbn) as usize * SECTOR;
        image[start..start + bytes.len()].copy_from_slice(bytes);
    };
    {
        let mut b = vec![0u8; SECTOR];
        clock.udf(&mut b, 16);
        put16(&mut b, 28, 3);
        put16(&mut b, 30, 3);
        put32(&mut b, 32, 1);
        put32(&mut b, 36, 1);
        charspec(&mut b, 48);
        dstring(&mut b, 112, 128, label);
        charspec(&mut b, 240);
        dstring(&mut b, 304, 32, label);
        put32(&mut b, 400, SECTOR as u32);
        put32(&mut b, 404, layout.root_fe);
        regid(&mut b, 416, b"*OSTA UDF Compliant", &[0x02, 0x01, 0x00]);
        tag(&mut b[..512], TAG_FSD, 0, 496);
        in_partition(&mut image, 0, &b);
        let mut td = vec![0u8; SECTOR];
        tag(&mut td[..512], TAG_TD, 1, 496);
        in_partition(&mut image, 1, &td);
    }
    let unique = |index: usize| 16 + index as u32;
    let (root_blocks, root_len) = fid_blocks(
        &[
            (None, true, layout.root_fe, 0),
            (Some("AUDIO_TS".into()), true, layout.audio_fe, unique(1)),
            (Some("VIDEO_TS".into()), true, layout.video_fe, unique(0)),
        ],
        layout.root_fids,
    );
    in_partition(&mut image, layout.root_fids, &root_blocks);
    in_partition(
        &mut image,
        layout.root_fe,
        &file_entry(
            layout.root_fe,
            true,
            3,
            0,
            root_len,
            layout.root_fids,
            clock,
        ),
    );
    let mut video_entries = vec![(None, true, layout.root_fe, 0)];
    for (index, file) in layout.files.iter().enumerate() {
        video_entries.push((
            Some(file.name.clone()),
            false,
            layout.file_fes + index as u32,
            unique(2 + index),
        ));
    }
    let (video_blocks, video_len) = fid_blocks(&video_entries, layout.video_fids);
    if (video_blocks.len() / SECTOR) as u32 != layout.video_fid_sectors {
        return Err(IsoError::UnreadableIso(
            "plan de directorio inconsistente".into(),
        ));
    }
    in_partition(&mut image, layout.video_fids, &video_blocks);
    in_partition(
        &mut image,
        layout.video_fe,
        &file_entry(
            layout.video_fe,
            true,
            1,
            u64::from(unique(0)),
            video_len,
            layout.video_fids,
            clock,
        ),
    );
    let (audio_blocks, audio_len) =
        fid_blocks(&[(None, true, layout.root_fe, 0)], layout.audio_fids);
    in_partition(&mut image, layout.audio_fids, &audio_blocks);
    in_partition(
        &mut image,
        layout.audio_fe,
        &file_entry(
            layout.audio_fe,
            true,
            1,
            u64::from(unique(1)),
            audio_len,
            layout.audio_fids,
            clock,
        ),
    );
    for (index, (file, data)) in layout.files.iter().zip(&layout.data).enumerate() {
        let location = layout.file_fes + index as u32;
        in_partition(
            &mut image,
            location,
            &file_entry(
                location,
                false,
                1,
                u64::from(unique(2 + index)),
                file.size,
                *data,
                clock,
            ),
        );
    }
    Ok(image)
}

/// Writes a DVD-Video image of `source_dir` (which must contain VIDEO_TS) to
/// `iso_path`. The image is written to a temporary sibling and renamed only
/// when complete; `cancelled` is polled between payload chunks.
pub fn write_dvd_video_iso(
    source_dir: &Path,
    iso_path: &Path,
    label: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<IsoReport, IsoError> {
    if label.trim().is_empty() || label.len() > 32 || !label.is_ascii() {
        return Err(IsoError::InvalidLabel(label.to_string()));
    }
    let video_ts = source_dir.join("VIDEO_TS");
    if !video_ts.is_dir() {
        return Err(IsoError::MissingSource(video_ts.display().to_string()));
    }
    let clock = Clock::now();
    let files = collect_files(&video_ts)?;
    let layout = plan(files, &clock)?;
    let head = header(&layout, label, &clock)?;

    if let Some(parent) = iso_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temp = crate::TempIso::new(iso_path);
    {
        let file = File::create(temp.path())?;
        file.set_len(layout.total_sectors * SECTOR_BYTES)?;
        let mut out = BufWriter::with_capacity(IO_BUFFER, file);
        out.write_all(&head)?;
        let mut buffer = vec![0u8; IO_BUFFER];
        for (dvd_file, data) in layout.files.iter().zip(&layout.data) {
            let offset = (u64::from(layout.partition_start) + u64::from(*data)) * SECTOR_BYTES;
            out.seek(SeekFrom::Start(offset))?;
            let mut reader = BufReader::with_capacity(IO_BUFFER, File::open(&dvd_file.source)?);
            let mut copied = 0u64;
            loop {
                if cancelled() {
                    return Err(IsoError::UnreadableIso("generación cancelada".into()));
                }
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                out.write_all(&buffer[..read])?;
                copied += read as u64;
            }
            if copied != dvd_file.size {
                return Err(IsoError::UnreadableIso(format!(
                    "{} cambió de tamaño mientras se escribía el ISO",
                    dvd_file.name
                )));
            }
        }
        let last = sector32(layout.total_sectors - 1)?;
        out.seek(SeekFrom::Start(u64::from(last) * SECTOR_BYTES))?;
        out.write_all(&anchor(last))?;
        let file = out
            .into_inner()
            .map_err(|error| IsoError::Io(error.into_error()))?;
        file.sync_all()?;
    }
    let listing = layout
        .files
        .iter()
        .zip(&layout.data)
        .map(|(file, data)| {
            format!(
                "/VIDEO_TS/{} @{} {} bytes",
                file.name,
                u64::from(layout.partition_start) + u64::from(*data),
                file.size
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let has_video_ts_ifo = layout.files.iter().any(|file| file.name == "VIDEO_TS.IFO");
    let has_vts_01_ifo = layout.files.iter().any(|file| file.name == "VTS_01_0.IFO");
    temp.publish(iso_path)?;
    Ok(IsoReport {
        iso_path: iso_path.to_path_buf(),
        size_bytes: layout.total_sectors * SECTOR_BYTES,
        has_video_ts_ifo,
        has_vts_01_ifo,
        listing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_order_matches_dvd_video_layout() {
        let mut names = vec![
            "VTS_01_0.BUP",
            "VTS_01_1.VOB",
            "VIDEO_TS.BUP",
            "VTS_02_0.IFO",
            "VTS_01_0.IFO",
            "VIDEO_TS.VOB",
            "VTS_01_2.VOB",
            "VIDEO_TS.IFO",
            "VTS_01_0.VOB",
        ];
        names.sort_by_key(|name| order_key(name));
        assert_eq!(
            names,
            vec![
                "VIDEO_TS.IFO",
                "VIDEO_TS.VOB",
                "VIDEO_TS.BUP",
                "VTS_01_0.IFO",
                "VTS_01_0.VOB",
                "VTS_01_1.VOB",
                "VTS_01_2.VOB",
                "VTS_01_0.BUP",
                "VTS_02_0.IFO",
            ]
        );
        assert!(order_key("README.TXT").is_none());
        assert!(order_key("VTS_01_1.IFO").is_none());
    }

    #[test]
    fn crc_matches_ecma_167_example() {
        // ECMA-167 3/7.2.6 example: CRC of bytes 70 6A 77 is 0x3299.
        assert_eq!(crc16(&[0x70, 0x6A, 0x77]), 0x3299);
    }

    #[test]
    fn large_vobs_are_split_into_short_extents() {
        let clock = Clock::from_unix(0);
        let size = MAX_EXTENT + 4096;
        let fe = file_entry(10, false, 1, 20, size, 100, &clock);
        assert_eq!(u32::from_le_bytes(fe[172..176].try_into().unwrap()), 16);
        assert_eq!(
            u32::from_le_bytes(fe[176..180].try_into().unwrap()),
            MAX_EXTENT as u32
        );
        assert_eq!(u32::from_le_bytes(fe[184..188].try_into().unwrap()), 4096);
        let second = u32::from_le_bytes(fe[188..192].try_into().unwrap());
        assert_eq!(u64::from(second), 100 + sectors(MAX_EXTENT));
    }

    #[test]
    fn clock_converts_unix_time() {
        let clock = Clock::from_unix(1_700_000_000);
        assert_eq!((clock.year, clock.month, clock.day), (2023, 11, 14));
        assert_eq!((clock.hour, clock.minute, clock.second), (22, 13, 20));
    }
    #[test]
    fn udf_and_iso9660_views_agree_and_payload_is_contiguous() {
        let root = std::env::temp_dir().join(format!("kinodisc-native-iso-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let video_ts = root.join("src").join("VIDEO_TS");
        fs::create_dir_all(&video_ts).unwrap();
        let files: [(&str, usize); 7] = [
            ("VIDEO_TS.IFO", 12_288),
            ("VIDEO_TS.VOB", 4_096),
            ("VIDEO_TS.BUP", 12_288),
            ("VTS_01_0.IFO", 18_432),
            ("VTS_01_0.VOB", 2_048),
            ("VTS_01_1.VOB", 200_704),
            ("VTS_01_0.BUP", 18_432),
        ];
        for (index, (name, size)) in files.iter().enumerate() {
            let bytes: Vec<u8> = (0..*size).map(|i| (i as u8) ^ (index as u8 * 37)).collect();
            fs::write(video_ts.join(name), bytes).unwrap();
        }
        let iso = root.join("disc.iso");
        let report = write_dvd_video_iso(&root.join("src"), &iso, "TEST_DISC", &|| false).unwrap();
        assert!(report.has_video_ts_ifo && report.has_vts_01_ifo);
        let image = fs::read(&iso).unwrap();
        assert_eq!(image.len() as u64 % SECTOR_BYTES, 0);

        // ISO 9660 view: every file present, bytes match, canonical order,
        // contiguous (each file starts where the previous one ends).
        let index = libmkiso::read_iso_index(
            &mut std::io::Cursor::new(&image),
            libmkiso::IsoLimits::default(),
        )
        .unwrap();
        let mut expected_next: Option<u64> = None;
        for (name, size) in files {
            let position = index
                .entries
                .iter()
                .position(|entry| entry.name.ends_with(name))
                .unwrap_or_else(|| panic!("ISO 9660 sin {name}"));
            let offset = index.offsets[position];
            assert_eq!(index.entries[position].size, size as u64, "{name}");
            let original = fs::read(video_ts.join(name)).unwrap();
            assert_eq!(
                &image[offset as usize..offset as usize + size],
                &original[..],
                "{name}"
            );
            if let Some(next) = expected_next {
                assert_eq!(offset, next, "{name} no está contiguo");
            }
            expected_next = Some(offset + size as u64);
        }

        // UDF view (what DVD players read) lists the same files and sizes.
        let udf = libmkiso::UdfReader::open(&image, libmkiso::UdfLimits::default()).unwrap();
        for (name, size) in files {
            let entry = udf
                .entries()
                .iter()
                .find(|entry| entry.name.ends_with(name))
                .unwrap_or_else(|| panic!("UDF sin {name}"));
            assert_eq!(entry.size, size as u64, "{name}");
        }
        assert!(udf
            .entries()
            .iter()
            .any(|entry| entry.directory && entry.name.ends_with("AUDIO_TS")));
        let _ = fs::remove_dir_all(&root);
    }
}
