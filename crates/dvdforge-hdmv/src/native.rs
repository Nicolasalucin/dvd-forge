//! Native Blu-ray (BDMV) navigation structures.
//!
//! These builders emit the canonical on-disc layouts used by tsMuxeR for a
//! single-movie Blu-ray folder. They are deliberately independent from the
//! in-memory tsMuxeR patch helpers so the native FFmpeg fallback can produce a
//! complete BDMV tree without reading media payloads.

use crate::bdmv::{
    BdmvError, M2TS_PACKET_SIZE, PMT_STREAM_TYPE_IG, PMT_STREAM_TYPE_PGS, TS_PACKET_SIZE,
};
use crate::opcodes::{play_playlist_mark, HdmvCommand};

/// BDAV clips start at 600 seconds. Playlist and mark times are 45 kHz.
pub const BDAV_START_45K: u32 = 27_000_000;
pub const CLPI_HEADER_LEN: usize = 56;
pub const MPLS_HEADER_LEN: usize = 58;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStream {
    pub pid: u16,
    pub stream_type: u8,
    pub language: Option<String>,
}

impl NativeStream {
    pub const fn video(pid: u16) -> Self {
        Self {
            pid,
            stream_type: 0x1B,
            language: None,
        }
    }

    pub const fn audio(pid: u16, language: Option<String>) -> Self {
        Self {
            pid,
            stream_type: 0x81,
            language,
        }
    }

    pub const fn pgs(pid: u16, language: Option<String>) -> Self {
        Self {
            pid,
            stream_type: PMT_STREAM_TYPE_PGS,
            language,
        }
    }

    pub const fn ig(pid: u16, language: Option<String>) -> Self {
        Self {
            pid,
            stream_type: PMT_STREAM_TYPE_IG,
            language,
        }
    }

    fn coding_type(&self) -> u8 {
        match self.stream_type {
            0x02 | 0x1B | 0x24 => 0x1B,
            0x03 | 0x04 | 0x0F | 0x81 | 0x82 | 0x86 => 0x81,
            PMT_STREAM_TYPE_IG => PMT_STREAM_TYPE_IG,
            _ => PMT_STREAM_TYPE_PGS,
        }
    }

    fn language_bytes(&self) -> [u8; 3] {
        let language = self.language.as_deref().unwrap_or("und");
        let bytes = language.as_bytes();
        [
            bytes.first().copied().unwrap_or(b'u'),
            bytes.get(1).copied().unwrap_or(b'n'),
            bytes.get(2).copied().unwrap_or(b'd'),
        ]
    }

    fn attribute_bytes(&self) -> [u8; 4] {
        let language = self.language_bytes();
        match self.coding_type() {
            0x1B => [0x31, 0x30, 0x00, 0x00],
            0x81 => [0x10, language[0], language[1], language[2]],
            PMT_STREAM_TYPE_IG => [language[0], language[1], language[2], 0x00],
            _ => [language[0], language[1], language[2], 0x00],
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativePlaylist {
    pub clip_id: u16,
    pub in_time: u32,
    pub out_time: u32,
    pub streams: Vec<NativeStream>,
    /// 45 kHz timestamps. Empty for menu playlists.
    pub marks: Vec<u32>,
    pub still_time: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct NativeClip {
    pub clip_id: u16,
    pub source_packets: u32,
    pub streams: Vec<NativeStream>,
}

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn clip_id_bytes(clip_id: u16) -> [u8; 5] {
    let mut bytes = [b'0'; 5];
    let text = format!("{clip_id:05}");
    bytes.copy_from_slice(&text.as_bytes()[..5]);
    bytes
}

/// Builds a 120-byte INDX0200 first-play/top-menu index.
pub fn build_index_bdmv(first_play: u16, top_menu: u16) -> Vec<u8> {
    let mut index = vec![0u8; 120];
    index[..8].copy_from_slice(b"INDX0200");
    put32(&mut index, 8, 78);
    put32(&mut index, 78, 38);
    let first = 82usize;
    index[first] = 0x40;
    put16(&mut index, first + 6, first_play);
    let top = 94usize;
    index[top] = 0x40;
    put16(&mut index, top + 6, top_menu);
    put16(&mut index, 106, 1);
    let title = 108usize;
    index[title] = 0x40;
    put16(&mut index, title + 6, 0);
    index
}

/// Builds a MovieObject.bdmv with a single 12-byte-command object.
pub fn build_movie_object(commands: &[HdmvCommand]) -> Result<Vec<u8>, BdmvError> {
    let command_count = u16::try_from(commands.len())
        .map_err(|_| BdmvError::InvalidMovieObject("too many commands"))?;
    let mut movie_object = vec![0u8; 50];
    movie_object[..8].copy_from_slice(b"MOBJ0200");
    let data_len = 6u32
        .checked_add(4)
        .and_then(|value| value.checked_add(u32::from(command_count) * 12))
        .ok_or(BdmvError::InvalidMovieObject("data length overflow"))?;
    put32(&mut movie_object, 40, data_len);
    put16(&mut movie_object, 48, 1);
    movie_object.extend_from_slice(&0x8000u16.to_be_bytes());
    movie_object.extend_from_slice(&command_count.to_be_bytes());
    for command in commands {
        movie_object.extend_from_slice(&command.to_bytes());
    }
    Ok(movie_object)
}

/// A one-object movie table whose object starts at playmark GPR0. Chapter
/// buttons set GPR0 and jump to title 1, so PlayPL_PM preserves seeking.
pub fn build_movie_object_with_gpr0_playmark(playlist: u16) -> Result<Vec<u8>, BdmvError> {
    build_movie_object(&[play_playlist_mark(u32::from(playlist), 0)])
}

fn build_stn(streams: &[NativeStream]) -> Result<Vec<u8>, BdmvError> {
    let mut video = 0u8;
    let mut audio = 0u8;
    let mut pg = 0u8;
    let mut ig = 0u8;
    for stream in streams {
        match stream.coding_type() {
            0x1B => video = video.saturating_add(1),
            0x81 => audio = audio.saturating_add(1),
            PMT_STREAM_TYPE_IG => ig = ig.saturating_add(1),
            _ => pg = pg.saturating_add(1),
        }
    }
    let payload_len = 2usize
        .checked_add(4)
        .and_then(|value| value.checked_add(8))
        .and_then(|value| value.checked_add(streams.len().checked_mul(16)?))
        .ok_or(BdmvError::InvalidMpls("STN length overflow"))?;
    let stn_len =
        u16::try_from(payload_len).map_err(|_| BdmvError::InvalidMpls("STN too large"))?;
    let mut stn = Vec::with_capacity(2 + payload_len);
    stn.extend_from_slice(&stn_len.to_be_bytes());
    stn.extend_from_slice(&[0, 0]);
    stn.extend_from_slice(&[video, audio, pg, ig]);
    stn.extend_from_slice(&[0; 8]);
    for stream in streams {
        stn.push(9); // stream entry length
        stn.push(0x01);
        stn.extend_from_slice(&stream.pid.to_be_bytes());
        stn.extend_from_slice(&[0; 6]);
        stn.push(5); // coding-info length
        let attributes = stream.attribute_bytes();
        stn.push(stream.coding_type());
        stn.extend_from_slice(&attributes);
    }
    Ok(stn)
}

/// Builds an MPLS0200 movie playlist with an optional chapter mark table.
pub fn build_playlist_mpls(spec: &NativePlaylist) -> Result<Vec<u8>, BdmvError> {
    if spec.out_time <= spec.in_time {
        return Err(BdmvError::InvalidMpls("OUT_time must follow IN_time"));
    }
    let stn = build_stn(&spec.streams)?;
    let playitem_len = 32usize
        .checked_add(stn.len())
        .ok_or(BdmvError::InvalidMpls("playitem overflow"))?;
    let playitem_len =
        u16::try_from(playitem_len).map_err(|_| BdmvError::InvalidMpls("playitem too large"))?;
    let list_len = 6usize
        .checked_add(2)
        .and_then(|value| value.checked_add(usize::from(playitem_len)))
        .ok_or(BdmvError::InvalidMpls("playlist overflow"))?;
    let list_len =
        u32::try_from(list_len).map_err(|_| BdmvError::InvalidMpls("playlist too large"))?;
    let mark_pos = MPLS_HEADER_LEN
        .checked_add(4)
        .and_then(|value| value.checked_add(usize::try_from(list_len).ok()?))
        .ok_or(BdmvError::InvalidMpls("mark position overflow"))?;

    let mut mpls = vec![0u8; MPLS_HEADER_LEN];
    mpls[..8].copy_from_slice(b"MPLS0200");
    put32(&mut mpls, 8, MPLS_HEADER_LEN as u32);
    put32(
        &mut mpls,
        12,
        u32::try_from(mark_pos).map_err(|_| BdmvError::InvalidMpls("mark position"))?,
    );
    let mut playlist = Vec::with_capacity(4 + list_len as usize);
    playlist.extend_from_slice(&list_len.to_be_bytes());
    playlist.extend_from_slice(&[0, 0]);
    playlist.extend_from_slice(&1u16.to_be_bytes());
    playlist.extend_from_slice(&0u16.to_be_bytes());
    playlist.extend_from_slice(&playitem_len.to_be_bytes());
    playlist.extend_from_slice(&clip_id_bytes(spec.clip_id));
    playlist.extend_from_slice(b"M2TS");
    playlist.extend_from_slice(&[0, 1, 0]);
    playlist.extend_from_slice(&spec.in_time.to_be_bytes());
    playlist.extend_from_slice(&spec.out_time.to_be_bytes());
    playlist.extend_from_slice(&[0; 9]);
    playlist.push(spec.still_time.map_or(0, |_| 0x01));
    playlist.extend_from_slice(&spec.still_time.unwrap_or(0).to_be_bytes());
    playlist.extend_from_slice(&stn);
    mpls.extend_from_slice(&playlist);

    if spec.marks.is_empty() {
        mpls.extend_from_slice(&0u32.to_be_bytes());
    } else {
        let count = u16::try_from(spec.marks.len())
            .map_err(|_| BdmvError::InvalidMpls("too many marks"))?;
        let length = 8u32
            .checked_add(u32::from(count) * 12)
            .ok_or(BdmvError::InvalidMpls("mark table overflow"))?;
        mpls.extend_from_slice(&length.to_be_bytes());
        mpls.extend_from_slice(&count.to_be_bytes());
        mpls.extend_from_slice(&[0, 1]);
        for mark in &spec.marks {
            mpls.extend_from_slice(&[0, 1, 0, 0, 1]);
            mpls.extend_from_slice(&mark.to_be_bytes());
            mpls.extend_from_slice(&[0xff, 0xff, 0, 0]);
        }
        mpls.extend_from_slice(&[0, 0, 0, 0]);
    }
    Ok(mpls)
}

fn build_sequence_info(streams: &[NativeStream]) -> Result<Vec<u8>, BdmvError> {
    let count =
        u8::try_from(streams.len()).map_err(|_| BdmvError::InvalidClpi("too many streams"))?;
    let length = 10usize
        .checked_add(
            streams
                .len()
                .checked_mul(24)
                .ok_or(BdmvError::InvalidClpi("sequence overflow"))?,
        )
        .ok_or(BdmvError::InvalidClpi("sequence overflow"))?;
    let length = u32::try_from(length).map_err(|_| BdmvError::InvalidClpi("sequence too large"))?;
    let mut sequence = Vec::with_capacity(10 + streams.len() * 24);
    sequence.extend_from_slice(&length.to_be_bytes());
    sequence.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
    for (index, stream) in streams.iter().enumerate() {
        if index == 0 {
            sequence.extend_from_slice(&[1, 0, count, 0]);
        } else {
            sequence.extend_from_slice(&[0, 0, 0, 0]);
        }
        sequence.extend_from_slice(&stream.pid.to_be_bytes());
        sequence.push(0x15);
        sequence.push(stream.coding_type());
        sequence.extend_from_slice(&stream.attribute_bytes());
        sequence.extend_from_slice(&[0x30; 12]);
    }
    Ok(sequence)
}

/// Builds a canonical HDMV0200 clip-info file with empty CPI and clip-mark
/// tables (the same layout tsMuxeR emits for short clips).
pub fn build_clpi(spec: &NativeClip) -> Result<Vec<u8>, BdmvError> {
    let sequence = build_sequence_info(&spec.streams)?;
    let sequence_start = 248usize;
    let program_start = align8(
        sequence_start
            .checked_add(sequence.len())
            .ok_or(BdmvError::InvalidClpi("sequence position overflow"))?,
    );
    let cpi_start = program_start
        .checked_add(36)
        .ok_or(BdmvError::InvalidClpi("program position overflow"))?;
    let mark_start = cpi_start
        .checked_add(4)
        .ok_or(BdmvError::InvalidClpi("CPI position overflow"))?;
    let total = mark_start
        .checked_add(4)
        .ok_or(BdmvError::InvalidClpi("clip length overflow"))?;
    let mut clpi = vec![0u8; total];
    clpi[..8].copy_from_slice(b"HDMV0200");
    put32(&mut clpi, 8, 0xdc);
    put32(&mut clpi, 12, sequence_start as u32);
    put32(&mut clpi, 16, program_start as u32);
    put32(&mut clpi, 20, cpi_start as u32);
    put32(&mut clpi, 24, mark_start as u32);
    clpi[40..56].copy_from_slice(&[
        0x00, 0x00, 0x00, 0xb0, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5b, 0x8d,
        0x80,
    ]);
    put32(&mut clpi, 56, spec.source_packets);
    clpi[sequence_start..sequence_start + sequence.len()].copy_from_slice(&sequence);
    // Canonical ProgramInfo block emitted by tsMuxeR for a primary video stream.
    let program: [u8; 36] = [
        0x00, 0x00, 0x00, 0x20, 0x00, 0x01, 0x00, 0x01, 0x10, 0x11, 0x00, 0x04, 0x00, 0x04, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x66, 0x00, 0x00,
        0x00, 0x04, 0x17, 0xf8, 0x00, 0x04,
    ];
    clpi[program_start..program_start + 36].copy_from_slice(&program);
    Ok(clpi)
}

fn align8(value: usize) -> usize {
    value.div_ceil(8) * 8
}

fn pid(packet: &[u8]) -> u16 {
    (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16
}

fn payload_start(packet: &[u8]) -> Option<usize> {
    if packet.len() < 5 || packet[0] != 0x47 || packet[3] & 0x10 == 0 {
        return None;
    }
    if packet[3] & 0x20 != 0 {
        let offset = 5 + packet[4] as usize;
        (offset < TS_PACKET_SIZE).then_some(offset)
    } else {
        Some(4)
    }
}

/// Parses the PAT/PMT stream descriptors from a bounded M2TS prefix. No media
/// payload is read; the caller only needs the first few kilobytes.
pub fn parse_pmt_from_m2ts_prefix(prefix: &[u8]) -> Result<Vec<NativeStream>, BdmvError> {
    let pmt_pid = find_pmt_pid(prefix)?;
    let mut streams = Vec::new();
    for packet in prefix.as_chunks::<M2TS_PACKET_SIZE>().0 {
        let ts = &packet[4..];
        if pid(ts) != pmt_pid || ts[1] & 0x40 == 0 {
            continue;
        }
        let Some(start) = payload_start(ts) else {
            continue;
        };
        let pointer = ts[start] as usize;
        let section_start = start + 1 + pointer;
        if section_start + 12 > TS_PACKET_SIZE {
            return Err(BdmvError::FragmentedPmt);
        }
        let section = &ts[section_start..];
        if section[0] != 0x02 {
            continue;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = 3 + section_length;
        if end > section.len() || end < 16 {
            return Err(BdmvError::FragmentedPmt);
        }
        let program_info_length = (((section[10] & 0x0F) as usize) << 8) | section[11] as usize;
        let mut offset = 12 + program_info_length;
        while offset + 5 <= end - 4 {
            let stream_type = section[offset];
            let stream_pid =
                (((section[offset + 1] & 0x1F) as u16) << 8) | section[offset + 2] as u16;
            let info_length =
                (((section[offset + 3] & 0x0F) as usize) << 8) | section[offset + 4] as usize;
            let info = section
                .get(offset + 5..offset + 5 + info_length)
                .unwrap_or(&[]);
            let mut language = None;
            let mut cursor = 0usize;
            while cursor + 2 <= info.len() {
                let tag = info[cursor];
                let length = info[cursor + 1] as usize;
                let body = info.get(cursor + 2..cursor + 2 + length).unwrap_or(&[]);
                if tag == 0x0A && body.len() >= 3 {
                    language = Some(String::from_utf8_lossy(&body[..3]).to_string());
                }
                cursor += 2 + length;
            }
            streams.push(NativeStream {
                pid: stream_pid,
                stream_type,
                language,
            });
            offset += 5 + info_length;
        }
        return Ok(streams);
    }
    Err(BdmvError::MissingPmt)
}

fn find_pmt_pid(prefix: &[u8]) -> Result<u16, BdmvError> {
    for packet in prefix.as_chunks::<M2TS_PACKET_SIZE>().0 {
        let ts = &packet[4..];
        if pid(ts) != 0 || ts[1] & 0x40 == 0 {
            continue;
        }
        let Some(start) = payload_start(ts) else {
            continue;
        };
        let pointer = ts[start] as usize;
        let Some(section) = ts.get(start + 1 + pointer..) else {
            continue;
        };
        if section.first().copied() != Some(0x00) || section.len() < 12 {
            continue;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = 3 + section_length;
        if end > section.len() || end < 12 {
            continue;
        }
        let mut offset = 8usize;
        while offset + 4 <= end - 4 {
            let program = u16::from_be_bytes([section[offset], section[offset + 1]]);
            let program_pid =
                (((section[offset + 2] & 0x1F) as u16) << 8) | section[offset + 3] as u16;
            if program != 0 {
                return Ok(program_pid);
            }
            offset += 4;
        }
    }
    Err(BdmvError::MissingPmt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::jump_object;

    #[test]
    fn blu_ray_packet_counts_are_checked_against_u32_on_32_bit() {
        let fifty_gib = 50u64 * 1024 * 1024 * 1024;
        let packets = fifty_gib / 192;
        assert!(u32::try_from(packets).is_ok());
        assert!(u32::try_from(u64::from(u32::MAX) + 1).is_err());
        assert!(u32::try_from(fifty_gib).is_err());
    }

    #[test]
    fn index_round_trips_through_the_existing_patch_helper() {
        let index = build_index_bdmv(3, 3);
        assert_eq!(&index[..8], b"INDX0200");
        let patched = crate::bdmv::set_index_menu_object(&index, 7).expect("index patch");
        assert_eq!(u16::from_be_bytes([patched[88], patched[89]]), 7);
        assert_eq!(u16::from_be_bytes([patched[100], patched[101]]), 7);
    }

    #[test]
    fn movie_object_starts_at_gpr0_mark_and_accepts_menu_append() {
        let object = build_movie_object_with_gpr0_playmark(0).expect("movie object");
        assert_eq!(crate::bdmv::movie_object_count(&object).unwrap(), 1);
        let (appended, index) = crate::bdmv::append_menu_movie_object(&object, 1).unwrap();
        assert_eq!(index, 1);
        assert_eq!(crate::bdmv::movie_object_count(&appended).unwrap(), 2);
        let command = jump_object(1);
        assert_eq!(command.to_bytes()[0], 0x21);
    }

    #[test]
    fn playlist_registers_pgs_and_chapter_marks() {
        let spec = NativePlaylist {
            clip_id: 0,
            in_time: BDAV_START_45K,
            out_time: BDAV_START_45K + 90_000,
            streams: vec![
                NativeStream::video(0x1011),
                NativeStream::audio(0x1100, Some("eng".into())),
                NativeStream::pgs(0x1200, Some("spa".into())),
            ],
            marks: vec![
                BDAV_START_45K,
                BDAV_START_45K + 45_000,
                BDAV_START_45K + 90_000,
            ],
            still_time: None,
        };
        let mpls = build_playlist_mpls(&spec).expect("mpls");
        assert_eq!(&mpls[..8], b"MPLS0200");
        assert_eq!(&mpls[70..75], b"00000");
        assert_eq!(&mpls[75..79], b"M2TS");
        let playitem = 58 + 10;
        let stn = playitem + 2 + 32;
        let counts = stn + 4;
        assert_eq!(&mpls[counts..counts + 4], &[1, 1, 1, 0]);
        let pg_entry = stn + 48;
        assert_eq!(
            u16::from_be_bytes([mpls[pg_entry + 2], mpls[pg_entry + 3]]),
            0x1200
        );
        let mark_start = u32::from_be_bytes([mpls[12], mpls[13], mpls[14], mpls[15]]) as usize;
        assert_eq!(
            u32::from_be_bytes(mpls[mark_start..mark_start + 4].try_into().unwrap()),
            44
        );
    }

    #[test]
    fn clpi_has_canonical_size_stream_blocks_and_source_packet_count() {
        let clip = NativeClip {
            clip_id: 0,
            source_packets: 704,
            streams: vec![
                NativeStream::video(0x1011),
                NativeStream::audio(0x1100, Some("eng".into())),
                NativeStream::pgs(0x1200, Some("spa".into())),
            ],
        };
        let clpi = build_clpi(&clip).expect("clpi");
        assert_eq!(&clpi[..8], b"HDMV0200");
        assert_eq!(clpi.len(), 380);
        assert_eq!(u32::from_be_bytes(clpi[56..60].try_into().unwrap()), 704);
        assert_eq!(u32::from_be_bytes(clpi[12..16].try_into().unwrap()), 248);
        assert_eq!(u32::from_be_bytes(clpi[16..20].try_into().unwrap()), 336);
        assert_eq!(u32::from_be_bytes(clpi[20..24].try_into().unwrap()), 372);
        let sequence = &clpi[248..336];
        assert_eq!(u32::from_be_bytes(sequence[..4].try_into().unwrap()), 82);
        assert_eq!(&sequence[14..16], &[0x10, 0x11]);
        assert_eq!(sequence[65], 0x90, "PGS coding type");
        assert_eq!(&sequence[66..69], b"spa");
    }
}
