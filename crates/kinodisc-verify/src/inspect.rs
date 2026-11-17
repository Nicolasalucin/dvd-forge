//! Disc inspector: what is on a DVD-Video or Blu-ray/AVCHD image or folder
//! (titles, durations, chapters, audio and subtitle streams, sizes), and an
//! importer that turns an unencrypted disc into plain video files that
//! Kinodisc can use as episodes.
//!
//! Everything read here is untrusted: all offsets are bounds-checked and a
//! malformed structure yields "unknown" values, never a panic. Encrypted
//! discs (CSS scrambling bits, AACS) are reported and never imported.

use libmkiso::{UdfEntryKind, UdfReader};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const SECTOR: u64 = 2048;
const MAX_IFO: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum InspectedKind {
    DvdVideo,
    Bdmv,
    Data,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InspectedStream {
    /// "ac3", "dts", "lpcm", "mpeg", "subtitle"...
    pub codec: String,
    pub language: String,
    pub channels: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct InspectedTitle {
    /// 1-based title (DVD) or playlist number (BDMV).
    pub number: u32,
    pub duration_ms: f64,
    pub chapters: u32,
    /// Chapter start times relative to the title, in ms (first is 0).
    pub chapter_starts_ms: Vec<f64>,
    pub bytes: f64,
    pub audio: Vec<InspectedStream>,
    pub subtitles: Vec<InspectedStream>,
    /// DVD: title set number; BDMV: main clip number.
    pub group: u32,
    /// BDMV: same clip and duration as an earlier playlist.
    pub duplicate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscInspection {
    pub kind: InspectedKind,
    pub volume_label: String,
    pub total_bytes: f64,
    pub file_count: u32,
    pub encrypted: bool,
    /// "NTSC 16:9", "PAL 4:3", "1080p 23.976"... when known.
    pub video: String,
    pub titles: Vec<InspectedTitle>,
    pub notes: Vec<String>,
}

/// A disc as a set of files: a UDF image or a folder holding VIDEO_TS/BDMV.
pub enum DiscSource {
    Image(Box<UdfReader<'static>>, String),
    Folder(PathBuf),
}

struct FileInfo {
    path: String,
    size: u64,
}

impl DiscSource {
    /// Opens an `.iso` image or a folder (the folder itself, its VIDEO_TS
    /// or BDMV parent, or a folder containing them).
    pub fn open(path: &Path) -> Result<Self, String> {
        if path.is_file() {
            let source = libmkiso::FileSource::open(path)
                .map_err(|error| format!("no se pudo abrir {}: {error}", path.display()))?;
            let reader = UdfReader::open_source(source, libmkiso::UdfLimits::default()).map_err(
                |error| format!("la imagen no tiene un sistema de archivos UDF válido: {error}"),
            )?;
            let label = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Ok(Self::Image(Box::new(reader), label));
        }
        if !path.is_dir() {
            return Err(format!("{} no existe.", path.display()));
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_uppercase())
            .unwrap_or_default();
        let root = if name == "VIDEO_TS" || name == "BDMV" {
            path.parent().unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        };
        Ok(Self::Folder(root))
    }

    fn files(&self) -> Vec<FileInfo> {
        match self {
            Self::Image(reader, _) => reader
                .entries()
                .iter()
                .filter(|entry| {
                    matches!(entry.kind, UdfEntryKind::File)
                        && !entry.directory
                        && entry.stream.is_none()
                })
                .map(|entry| FileInfo {
                    path: crate::normalize(&entry.name),
                    size: entry.size,
                })
                .collect(),
            Self::Folder(root) => {
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(dir) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let Ok(kind) = entry.file_type() else {
                            continue;
                        };
                        let path = entry.path();
                        if kind.is_dir() {
                            if out.len() < 100_000 {
                                stack.push(path);
                            }
                        } else if kind.is_file() {
                            let relative = path
                                .strip_prefix(root)
                                .map(|value| value.to_string_lossy().replace('\\', "/"))
                                .unwrap_or_default();
                            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                            out.push(FileInfo {
                                path: relative,
                                size,
                            });
                        }
                    }
                }
                out
            }
        }
    }

    /// Case-insensitive lookup of a disc path ("VIDEO_TS/VIDEO_TS.IFO").
    fn find(&self, files: &[FileInfo], wanted: &str) -> Option<(String, u64)> {
        files
            .iter()
            .find(|file| file.path.eq_ignore_ascii_case(wanted))
            .map(|file| (file.path.clone(), file.size))
    }

    fn read_range(&self, path: &str, offset: u64, length: usize) -> Result<Vec<u8>, String> {
        match self {
            Self::Image(reader, _) => {
                let index = reader
                    .entries()
                    .iter()
                    .position(|entry| crate::normalize(&entry.name) == path)
                    .ok_or_else(|| format!("no existe {path}"))?;
                let mut buffer = vec![0u8; length];
                let mut filled = 0;
                while filled < length {
                    let read = reader
                        .read_at(index, offset + filled as u64, &mut buffer[filled..])
                        .map_err(|error| format!("no se pudo leer {path}: {error}"))?;
                    if read == 0 {
                        break;
                    }
                    filled += read;
                }
                buffer.truncate(filled);
                Ok(buffer)
            }
            Self::Folder(root) => {
                let full = root.join(path);
                let mut file = std::fs::File::open(&full)
                    .map_err(|error| format!("no se pudo abrir {}: {error}", full.display()))?;
                file.seek(SeekFrom::Start(offset))
                    .map_err(|error| error.to_string())?;
                let mut buffer = Vec::with_capacity(length);
                file.take(length as u64)
                    .read_to_end(&mut buffer)
                    .map_err(|error| error.to_string())?;
                Ok(buffer)
            }
        }
    }

    fn read_small(&self, path: &str, size: u64) -> Result<Vec<u8>, String> {
        if size > MAX_IFO {
            return Err(format!(
                "{path} es demasiado grande para un archivo de control"
            ));
        }
        self.read_range(path, 0, size as usize)
    }
}

fn be16(bytes: &[u8], offset: usize) -> u16 {
    crate::be16(bytes, offset).unwrap_or(0)
}

fn be32(bytes: &[u8], offset: usize) -> u32 {
    crate::be32(bytes, offset).unwrap_or(0)
}

fn bcd(value: u8) -> u64 {
    u64::from(value >> 4) * 10 + u64::from(value & 0x0F)
}

/// DVD playback time (BCD hh:mm:ss:ff + frame-rate bits) in ms.
fn dvd_time_ms(bytes: &[u8]) -> u64 {
    let Some(time) = bytes.get(..4) else { return 0 };
    let fps = match time[3] >> 6 {
        1 => 25.0,
        3 => 29.97,
        _ => 30.0,
    };
    let frames = bcd(time[3] & 0x3F) as f64;
    ((bcd(time[0]) * 3600 + bcd(time[1]) * 60 + bcd(time[2])) as f64 * 1000.0
        + frames * 1000.0 / fps)
        .round() as u64
}

fn language(bytes: &[u8]) -> String {
    match bytes.get(..2) {
        Some([a, b]) if a.is_ascii_alphabetic() && b.is_ascii_alphabetic() => {
            format!("{}{}", *a as char, *b as char).to_ascii_lowercase()
        }
        _ => String::new(),
    }
}

/// Inspects an image or folder.
pub fn inspect(path: &Path) -> Result<DiscInspection, String> {
    let source = DiscSource::open(path)?;
    inspect_source(&source)
}

pub fn inspect_source(source: &DiscSource) -> Result<DiscInspection, String> {
    let files = source.files();
    let total: u64 = files.iter().map(|file| file.size).sum();
    let mut inspection = DiscInspection {
        kind: InspectedKind::Data,
        volume_label: match source {
            DiscSource::Image(_, label) => label.clone(),
            DiscSource::Folder(root) => root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        total_bytes: total as f64,
        file_count: files.len() as u32,
        encrypted: false,
        video: String::new(),
        titles: Vec::new(),
        notes: Vec::new(),
    };
    if source.find(&files, "VIDEO_TS/VIDEO_TS.IFO").is_some() {
        inspection.kind = InspectedKind::DvdVideo;
        inspect_dvd(source, &files, &mut inspection);
    } else if source.find(&files, "BDMV/index.bdmv").is_some() {
        inspection.kind = InspectedKind::Bdmv;
        inspect_bdmv(source, &files, &mut inspection);
    } else {
        inspection
            .notes
            .push("No contiene VIDEO_TS ni BDMV: es un disco de datos.".to_string());
    }
    Ok(inspection)
}

struct DvdTitlePlan {
    number: u32,
    vts: u32,
    /// Sector ranges (inclusive) relative to the start of VTS_xx_1.VOB.
    sectors: Vec<(u64, u64)>,
}

fn dvd_vob_files(files: &[FileInfo], vts: u32) -> Vec<(String, u64)> {
    let mut parts: Vec<(u32, String, u64)> = files
        .iter()
        .filter_map(|file| {
            let upper = file.path.to_ascii_uppercase();
            let name = upper.strip_prefix("VIDEO_TS/")?;
            let rest = name.strip_prefix(&format!("VTS_{vts:02}_"))?;
            let part: u32 = rest.strip_suffix(".VOB")?.parse().ok()?;
            (part >= 1).then(|| (part, file.path.clone(), file.size))
        })
        .collect();
    parts.sort_by_key(|(part, _, _)| *part);
    parts
        .into_iter()
        .map(|(_, path, size)| (path, size))
        .collect()
}

fn inspect_dvd(source: &DiscSource, files: &[FileInfo], out: &mut DiscInspection) {
    let (titles, notes) = dvd_titles(source, files, Some(out));
    out.notes.extend(notes);
    let _ = titles;
}

/// Parses the DVD title table; fills `out` (when given) and returns the
/// sector plan used by the importer.
fn dvd_titles(
    source: &DiscSource,
    files: &[FileInfo],
    mut out: Option<&mut DiscInspection>,
) -> (Vec<DvdTitlePlan>, Vec<String>) {
    let mut notes = Vec::new();
    let mut plans = Vec::new();
    let Some((vmg_path, vmg_size)) = source.find(files, "VIDEO_TS/VIDEO_TS.IFO") else {
        return (plans, notes);
    };
    let Ok(vmg) = source.read_small(&vmg_path, vmg_size) else {
        notes.push("No se pudo leer VIDEO_TS.IFO.".to_string());
        return (plans, notes);
    };
    if vmg.get(..12) != Some(b"DVDVIDEO-VMG".as_slice()) {
        notes.push("VIDEO_TS.IFO no tiene la cabecera DVDVIDEO-VMG.".to_string());
        return (plans, notes);
    }
    let tt_srpt = (be32(&vmg, 0xC4) as usize).saturating_mul(SECTOR as usize);
    let title_count = be16(&vmg, tt_srpt) as usize;
    let mut vts_cache: std::collections::HashMap<u32, Vec<u8>> = std::collections::HashMap::new();
    for index in 0..title_count.min(99) {
        let entry = tt_srpt + 8 + index * 12;
        if entry + 12 > vmg.len() {
            break;
        }
        let ptts = be16(&vmg, entry + 2) as u32;
        let vts = u32::from(vmg[entry + 6]);
        let vts_title = vmg[entry + 7];
        if vts == 0 {
            continue;
        }
        let vts_ifo = vts_cache.entry(vts).or_insert_with(|| {
            source
                .find(files, &format!("VIDEO_TS/VTS_{vts:02}_0.IFO"))
                .and_then(|(path, size)| source.read_small(&path, size).ok())
                .unwrap_or_default()
        });
        if vts_ifo.get(..12) != Some(b"DVDVIDEO-VTS".as_slice()) {
            notes.push(format!("Falta o está dañado VTS_{vts:02}_0.IFO."));
            continue;
        }
        let pgcit = (be32(vts_ifo, 0xCC) as usize).saturating_mul(SECTOR as usize);
        let pgc_count = be16(vts_ifo, pgcit) as usize;
        let mut chosen = None;
        for pgc_index in 0..pgc_count.min(999) {
            let search = pgcit + 8 + pgc_index * 8;
            let Some(&category) = vts_ifo.get(search) else {
                break;
            };
            if category & 0x80 != 0 && category & 0x7F == vts_title {
                chosen = Some(pgcit.saturating_add(be32(vts_ifo, search + 4) as usize));
                break;
            }
        }
        let Some(pgc) = chosen.filter(|pgc| pgc.saturating_add(0xEC) <= vts_ifo.len()) else {
            notes.push(format!("No se encontró la PGC del título {}.", index + 1));
            continue;
        };
        let duration = dvd_time_ms(&vts_ifo[pgc + 4..]);
        let program_count = vts_ifo[pgc + 2] as usize;
        let cell_count = vts_ifo[pgc + 3] as usize;
        let program_map = pgc + be16(vts_ifo, pgc + 0xE6) as usize;
        let cell_table = pgc + be16(vts_ifo, pgc + 0xE8) as usize;
        let mut cell_starts = Vec::new();
        let mut elapsed = 0u64;
        let mut sectors = Vec::new();
        for cell in 0..cell_count {
            let at = cell_table + cell * 0x18;
            let Some(entry) = vts_ifo.get(at..at + 0x18) else {
                break;
            };
            cell_starts.push(elapsed);
            elapsed += dvd_time_ms(&entry[4..]);
            let first = u64::from(be32(entry, 8));
            let last = u64::from(be32(entry, 0x14));
            // Angle cells other than the first block member are skipped.
            let block_member = (entry[0] >> 4) & 0x03 >= 2;
            if last >= first && !block_member {
                sectors.push((first, last));
            }
        }
        let chapter_starts: Vec<f64> = (0..program_count)
            .filter_map(|program| vts_ifo.get(program_map + program).copied())
            .filter_map(|entry_cell| {
                cell_starts
                    .get((entry_cell as usize).checked_sub(1)?)
                    .copied()
            })
            .map(|ms| ms as f64)
            .collect();
        let bytes: u64 = sectors.iter().map(|(a, b)| (b - a + 1) * SECTOR).sum();
        if let Some(out) = out.as_deref_mut() {
            if out.video.is_empty() {
                let attributes = be16(vts_ifo, 0x200);
                let standard = if (attributes >> 12) & 0x03 == 1 {
                    "PAL"
                } else {
                    "NTSC"
                };
                let aspect = if (attributes >> 10) & 0x03 == 3 {
                    "16:9"
                } else {
                    "4:3"
                };
                out.video = format!("{standard} {aspect}");
            }
            let audio_count = (be16(vts_ifo, 0x202) as usize).min(8);
            let audio = (0..audio_count)
                .filter_map(|stream| vts_ifo.get(0x204 + stream * 8..0x204 + stream * 8 + 8))
                .map(|attr| InspectedStream {
                    codec: match attr[0] >> 5 {
                        0 => "ac3",
                        2 | 3 => "mpeg",
                        4 => "lpcm",
                        6 => "dts",
                        _ => "audio",
                    }
                    .to_string(),
                    language: language(&attr[2..]),
                    channels: (attr[1] & 0x07) + 1,
                })
                .collect();
            let sub_count = (be16(vts_ifo, 0x254) as usize).min(32);
            let subtitles = (0..sub_count)
                .filter_map(|stream| vts_ifo.get(0x256 + stream * 6..0x256 + stream * 6 + 6))
                .map(|attr| InspectedStream {
                    codec: "subtitle".to_string(),
                    language: language(&attr[2..]),
                    channels: 0,
                })
                .collect();
            out.titles.push(InspectedTitle {
                number: index as u32 + 1,
                duration_ms: duration as f64,
                chapters: ptts,
                chapter_starts_ms: chapter_starts,
                bytes: bytes as f64,
                audio,
                subtitles,
                group: vts,
                duplicate: false,
            });
        }
        plans.push(DvdTitlePlan {
            number: index as u32 + 1,
            vts,
            sectors,
        });
    }
    // CSS: scrambled PES packets in the first title set's VOBs.
    if let Some(out) = out {
        if let Some((vob, _)) = dvd_vob_files(files, 1).first() {
            if let Ok(head) = source.read_range(vob, 0, 64 * SECTOR as usize) {
                out.encrypted = mpeg_ps_scrambled(&head);
            }
        }
        if out.encrypted {
            notes.push(
                "El disco está cifrado (CSS): se puede inspeccionar, pero no importar.".to_string(),
            );
        }
    }
    (plans, notes)
}

/// True when a PES packet in the MPEG program stream has its scrambling
/// control bits set (CSS-protected content).
pub fn mpeg_ps_scrambled(data: &[u8]) -> bool {
    for pack in data.chunks(SECTOR as usize) {
        if pack.len() < 24 || pack[..4] != [0, 0, 1, 0xBA] {
            continue;
        }
        let stuffing = (pack[13] & 0x07) as usize;
        let pes = 14 + stuffing;
        let Some(header) = pack.get(pes..pes + 7) else {
            continue;
        };
        let stream = header[3];
        if header[..3] == [0, 0, 1]
            && (stream == 0xE0 || stream == 0xBD || (0xC0..=0xDF).contains(&stream))
            && header[6] & 0x30 != 0
        {
            return true;
        }
    }
    false
}

struct BdPlaylist {
    number: u32,
    clip: u32,
    duration_ms: u64,
    chapter_starts_ms: Vec<f64>,
    audio: u8,
    pg: u8,
    ig: u8,
}

fn parse_mpls(bytes: &[u8], number: u32) -> Option<BdPlaylist> {
    if bytes.get(..4) != Some(b"MPLS".as_slice()) {
        return None;
    }
    let list = be32(bytes, 8) as usize;
    let marks = be32(bytes, 12) as usize;
    let items = be16(bytes, list + 6) as usize;
    let mut cursor = list + 10;
    let mut duration = 0u64;
    let mut clip = None;
    let mut first_in = 0u32;
    let mut counts = (0u8, 0u8, 0u8);
    for item in 0..items.min(999) {
        let length = be16(bytes, cursor) as usize;
        let start = cursor + 2;
        let name = bytes.get(start..start + 5)?;
        let in_time = be32(bytes, start + 12);
        let out_time = be32(bytes, start + 16);
        if item == 0 {
            clip = std::str::from_utf8(name).ok()?.parse::<u32>().ok();
            first_in = in_time;
            let stn = start + 32;
            if let Some(numbers) = bytes.get(stn + 4..stn + 8) {
                counts = (numbers[1], numbers[2], numbers[3]);
            }
        }
        duration += u64::from(out_time.saturating_sub(in_time));
        cursor = start + length;
    }
    let mut chapter_starts = Vec::new();
    let count = be16(bytes, marks + 4) as usize;
    for index in 0..count.min(9999) {
        let entry = marks + 6 + index * 14;
        let Some(fields) = bytes.get(entry..entry + 14) else {
            break;
        };
        if fields[1] == 1 && be16(fields, 2) == 0 {
            let time = be32(fields, 4);
            chapter_starts.push(f64::from(time.saturating_sub(first_in)) / 45.0);
        }
    }
    Some(BdPlaylist {
        number,
        clip: clip?,
        duration_ms: duration / 45,
        chapter_starts_ms: chapter_starts,
        audio: counts.0,
        pg: counts.1,
        ig: counts.2,
    })
}

fn bd_playlists(source: &DiscSource, files: &[FileInfo]) -> Vec<BdPlaylist> {
    let mut list: Vec<BdPlaylist> = files
        .iter()
        .filter_map(|file| {
            let upper = file.path.to_ascii_uppercase();
            let name = upper
                .strip_prefix("BDMV/PLAYLIST/")?
                .strip_suffix(".MPLS")?;
            let number: u32 = name.parse().ok()?;
            let bytes = source.read_small(&file.path, file.size).ok()?;
            parse_mpls(&bytes, number)
        })
        .collect();
    list.sort_by_key(|playlist| playlist.number);
    list
}

fn inspect_bdmv(source: &DiscSource, files: &[FileInfo], out: &mut DiscInspection) {
    let aacs = files.iter().any(|file| {
        let upper = file.path.to_ascii_uppercase();
        upper.starts_with("AACS/") || upper.starts_with("BDMV/AACS/")
    });
    let mut scrambled = false;
    if let Some(stream) = files
        .iter()
        .find(|file| file.path.to_ascii_uppercase().starts_with("BDMV/STREAM/") && file.size >= 192)
    {
        if let Ok(head) = source.read_range(&stream.path, 0, 192 * 32) {
            // copy_permission_indicator (top bits of TP_extra_header).
            scrambled = head.chunks(192).any(|packet| packet[0] >> 6 != 0);
        }
    }
    out.encrypted = aacs || scrambled;
    if out.encrypted {
        out.notes.push(
            "El disco está cifrado (AACS): se puede inspeccionar, pero no importar.".to_string(),
        );
    }
    let mut seen: Vec<(u32, u64)> = Vec::new();
    for playlist in bd_playlists(source, files) {
        // Menus (interactive graphics without chapters) and very short
        // clips are not titles.
        if playlist.duration_ms < 5_000
            || (playlist.ig > 0 && playlist.chapter_starts_ms.is_empty())
        {
            continue;
        }
        let duplicate = seen.contains(&(playlist.clip, playlist.duration_ms));
        seen.push((playlist.clip, playlist.duration_ms));
        let bytes = source
            .find(files, &format!("BDMV/STREAM/{:05}.m2ts", playlist.clip))
            .map(|(_, size)| size)
            .unwrap_or(0);
        out.titles.push(InspectedTitle {
            number: playlist.number,
            duration_ms: playlist.duration_ms as f64,
            chapters: playlist.chapter_starts_ms.len() as u32,
            chapter_starts_ms: playlist.chapter_starts_ms.clone(),
            bytes: bytes as f64,
            audio: (0..playlist.audio)
                .map(|_| InspectedStream {
                    codec: "audio".to_string(),
                    language: String::new(),
                    channels: 0,
                })
                .collect(),
            subtitles: (0..playlist.pg)
                .map(|_| InspectedStream {
                    codec: "subtitle".to_string(),
                    language: String::new(),
                    channels: 0,
                })
                .collect(),
            group: playlist.clip,
            duplicate,
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTitle {
    pub path: String,
    pub title: String,
    pub chapter_starts_ms: Vec<f64>,
}

/// Writes one video file per title into `dest` (DVD: the title's sectors
/// cut from its VOB set; BDMV: a copy of the main clip of each distinct
/// playlist). Encrypted discs are refused.
pub fn import_titles(
    path: &Path,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<ImportedTitle>, String> {
    let source = DiscSource::open(path)?;
    let report = inspect_source(&source)?;
    if report.encrypted {
        return Err("El disco está cifrado; Kinodisc no importa discos protegidos.".to_string());
    }
    std::fs::create_dir_all(dest).map_err(|error| error.to_string())?;
    let files = source.files();
    let mut imported = Vec::new();
    match report.kind {
        InspectedKind::DvdVideo => {
            let (plans, _) = dvd_titles(&source, &files, None);
            for (plan, info) in plans.iter().zip(report.titles.iter()) {
                if plan.sectors.is_empty() || info.duration_ms < 1_000.0 {
                    continue;
                }
                let vobs = dvd_vob_files(&files, plan.vts);
                let target = dest.join(format!("Titulo {:02}.vob", plan.number));
                copy_sectors(&source, &vobs, &plan.sectors, &target, cancelled)?;
                imported.push(ImportedTitle {
                    path: target.display().to_string(),
                    title: format!("Título {}", plan.number),
                    chapter_starts_ms: info.chapter_starts_ms.clone(),
                });
            }
        }
        InspectedKind::Bdmv => {
            for info in report.titles.iter().filter(|title| !title.duplicate) {
                let Some((clip, size)) =
                    source.find(&files, &format!("BDMV/STREAM/{:05}.m2ts", info.group))
                else {
                    continue;
                };
                let target = dest.join(format!("Titulo {:05}.m2ts", info.number));
                copy_sectors(
                    &source,
                    &[(clip, size)],
                    &[(0, size.div_ceil(SECTOR).saturating_sub(1))],
                    &target,
                    cancelled,
                )?;
                // Keep the exact size (the last sector may be partial).
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = file.set_len(size);
                }
                imported.push(ImportedTitle {
                    path: target.display().to_string(),
                    title: format!("Título {}", info.number),
                    chapter_starts_ms: info.chapter_starts_ms.clone(),
                });
            }
        }
        InspectedKind::Data => {
            return Err("No hay vídeo de DVD ni de Blu-ray que importar.".to_string())
        }
    }
    if imported.is_empty() {
        return Err("No se encontró ningún título que importar.".to_string());
    }
    Ok(imported)
}

/// Copies inclusive sector ranges of a multi-file VOB set (treated as one
/// continuous stream) into `target`.
fn copy_sectors(
    source: &DiscSource,
    parts: &[(String, u64)],
    ranges: &[(u64, u64)],
    target: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), String> {
    let partial = target.with_extension("kinodisc-part");
    let mut out = std::io::BufWriter::new(
        std::fs::File::create(&partial).map_err(|error| error.to_string())?,
    );
    let total: u64 = parts.iter().map(|(_, size)| size).sum();
    let chunk = 4 << 20;
    let result = (|| {
        for &(first, last) in ranges {
            let mut offset = first.saturating_mul(SECTOR);
            let end = last.saturating_add(1).saturating_mul(SECTOR).min(total);
            while offset < end {
                if cancelled() {
                    return Err("cancelado".to_string());
                }
                // Locate the part holding `offset`.
                let mut base = 0u64;
                let mut found = None;
                for (path, size) in parts {
                    if offset < base + size {
                        found = Some((path, offset - base, base + size));
                        break;
                    }
                    base += size;
                }
                let Some((path, local, part_end)) = found else {
                    break;
                };
                let length = (end.min(part_end) - offset).min(chunk) as usize;
                let data = source.read_range(path, local, length)?;
                if data.is_empty() {
                    break;
                }
                out.write_all(&data).map_err(|error| error.to_string())?;
                offset += data.len() as u64;
            }
        }
        out.flush().map_err(|error| error.to_string())
    })();
    drop(out);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(error);
    }
    let _ = std::fs::remove_file(target);
    std::fs::rename(&partial, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dvd_times_decode_bcd_and_frame_rate() {
        // 01:02:03, 12 frames at 25 fps (0x40 | 0x12).
        assert_eq!(dvd_time_ms(&[0x01, 0x02, 0x03, 0x52]), 3_723_480);
        // 29.97 fps flag (0xC0).
        assert_eq!(dvd_time_ms(&[0x00, 0x00, 0x01, 0xC0]), 1_000);
        assert_eq!(dvd_time_ms(&[1, 2]), 0);
    }

    #[test]
    fn scrambled_pes_is_detected() {
        let mut pack = vec![0u8; 2048];
        pack[..4].copy_from_slice(&[0, 0, 1, 0xBA]);
        pack[13] = 0xF8; // no stuffing
        pack[14..18].copy_from_slice(&[0, 0, 1, 0xE0]);
        pack[20] = 0x80; // '10' marker, not scrambled
        assert!(!mpeg_ps_scrambled(&pack));
        pack[20] = 0x80 | 0x10;
        assert!(mpeg_ps_scrambled(&pack));
        assert!(!mpeg_ps_scrambled(&[0u8; 100]));
    }

    #[test]
    fn malformed_playlists_do_not_panic() {
        for length in [0usize, 4, 20, 64, 200] {
            let mut bytes = vec![0xFFu8; length];
            if length >= 4 {
                bytes[..4].copy_from_slice(b"MPLS");
            }
            let _ = parse_mpls(&bytes, 1);
        }
    }
}
