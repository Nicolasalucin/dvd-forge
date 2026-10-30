//! Native BDMV authoring helpers for HDMV interactive graphics menus.
//!
//! tsMuxeR 2.7 can multiplex a Blu-ray movie but it cannot carry a native
//! `S_HDMV/IGS` track: it only knows the presentation-graphics label and drops
//! IG payloads. This module keeps the movie mux untouched and attaches a
//! dedicated menu clip built from a short video track plus a dummy PGS track:
//!
//! 1. tsMuxeR reserves a PGS stream (PID `0x1200`) in the menu clip.
//! 2. The first PGS transport-packet slots are replaced with the compiled IG
//!    PES packets on PID `0x1400`.
//! 3. PMT, MPLS and CLPI entries are retagged from PG (`0x90`) to IG (`0x91`).
//! 4. A MovieObject plays the new menu playlist and index.bdmv points First
//!    Playback and Top Menu to it.
//!
//! The result is a standards-shaped HDMV menu that libbluray recognizes and
//! decodes as an interactive graphics plane.

use crate::opcodes::{jump_object, play_playlist};
use crate::pes::IG_PID;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use thiserror::Error;

pub const PGS_PID: u16 = 0x1200;
pub const PMT_STREAM_TYPE_PGS: u8 = 0x90;
pub const PMT_STREAM_TYPE_IG: u8 = 0x91;
pub const M2TS_PACKET_SIZE: usize = 192;
pub const TS_PACKET_SIZE: usize = 188;
pub const STILL_TIME_INFINITE: u16 = 0xFFFF;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BdmvError {
    #[error("el M2TS del menú está vacío o no está alineado a 192 bytes")]
    InvalidM2ts,
    #[error("error de E/S: {0}")]
    Io(String),
    #[error("el M2TS supera el contador de paquetes de 32 bits")]
    PacketCountOverflow,
    #[error("no se encontró el PAT/PMT del menú")]
    MissingPmt,
    #[error("la sección PMT está fragmentada; no se puede parchear en sitio")]
    FragmentedPmt,
    #[error("el menú no contiene una pista PGS reservada")]
    MissingPgsTrack,
    #[error("el manifiesto MPLS no es válido: {0}")]
    InvalidMpls(&'static str),
    #[error("el manifiesto CLPI no es válido: {0}")]
    InvalidClpi(&'static str),
    #[error("MovieObject.bdmv no es válido: {0}")]
    InvalidMovieObject(&'static str),
    #[error("index.bdmv no es válido: {0}")]
    InvalidIndex(&'static str),
    #[error("el flujo IG no cabe en los huecos PGS reservados ({needed} > {available})")]
    NotEnoughPgsSlots { needed: usize, available: usize },
    #[error(
        "el segmento PGS {kind:#04x} ocupa {length} bytes y no cabe en el campo de longitud de 16 bits"
    )]
    PgsSegmentTooLarge { kind: u8, length: usize },
}

impl From<std::io::Error> for BdmvError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Supplies the number of transport packets needed by a set of PES packets.
/// Each PES starts a new transport packet and the final chunk uses an
/// adaptation field so the decoder sees exactly the declared PES length.
pub fn pes_transport_packet_count(pes_packets: &[Vec<u8>]) -> usize {
    pes_packets
        .iter()
        .map(|packet| packet.len().div_ceil(184).max(1))
        .sum()
}

/// Wraps each PES packet in TS packets with a valid adaptation field on the
/// final chunk. Every PES starts with `payload_unit_start_indicator` set.
pub fn packetize_pes(pes_packets: &[Vec<u8>], pid: u16) -> Vec<[u8; TS_PACKET_SIZE]> {
    let mut output = Vec::with_capacity(pes_transport_packet_count(pes_packets));
    let mut continuity = 0u8;
    for pes in pes_packets {
        if pes.is_empty() {
            continue;
        }
        let chunk_count = pes.len().div_ceil(184).max(1);
        for (index, chunk) in pes.chunks(184).enumerate() {
            let mut packet = [0xFFu8; TS_PACKET_SIZE];
            packet[0] = 0x47;
            let pusi = if index == 0 { 0x40 } else { 0x00 };
            packet[1] = pusi | ((pid >> 8) as u8 & 0x1F);
            packet[2] = (pid & 0xFF) as u8;
            if chunk.len() == 184 {
                packet[3] = 0x10 | (continuity & 0x0F);
                packet[4..].copy_from_slice(chunk);
            } else {
                let adaptation_len = (TS_PACKET_SIZE - 5 - chunk.len()) as u8;
                packet[3] = 0x30 | (continuity & 0x0F);
                packet[4] = adaptation_len;
                if adaptation_len > 0 {
                    packet[5] = 0;
                    for byte in &mut packet[6..5 + adaptation_len as usize] {
                        *byte = 0xFF;
                    }
                }
                let start = 5 + adaptation_len as usize;
                packet[start..start + chunk.len()].copy_from_slice(chunk);
            }
            continuity = continuity.wrapping_add(1);
            output.push(packet);
            if index + 1 == chunk_count {
                debug_assert!(chunk.len() <= 184);
            }
        }
    }
    output
}

/// Replaces the reserved PGS transport packets in a menu M2TS with native IG
/// packets and patches its PMT.
///
/// The replacement is strictly one transport packet for another: the original
/// arrival timestamp and the total packet count are preserved, so the SPN table
/// and CLPI EP_map generated by tsMuxeR remain valid. Unused reserved slots are
/// filled with null packets instead of being removed.
pub fn inject_ig_into_menu_m2ts(
    m2ts: &[u8],
    ig_pes_packets: &[Vec<u8>],
    ig_pid: u16,
) -> Result<Vec<u8>, BdmvError> {
    if m2ts.is_empty() || !m2ts.len().is_multiple_of(M2TS_PACKET_SIZE) {
        return Err(BdmvError::InvalidM2ts);
    }
    if !m2ts
        .as_chunks::<M2TS_PACKET_SIZE>()
        .0
        .iter()
        .all(|packet| packet[4] == 0x47)
    {
        return Err(BdmvError::InvalidM2ts);
    }

    let pmt_pid = find_pmt_pid(m2ts)?;
    let pgs_pid = find_pgs_pid(m2ts, pmt_pid)?;
    let mut patched = patch_pmt(m2ts, pmt_pid, pgs_pid, ig_pid)?;

    let ig_packets = packetize_pes(ig_pes_packets, ig_pid);
    if ig_packets.is_empty() {
        return Ok(patched);
    }
    let available = patched
        .as_chunks::<M2TS_PACKET_SIZE>()
        .0
        .iter()
        .filter(|packet| ts_pid(&packet[4..]) == pgs_pid)
        .count();
    if ig_packets.len() > available {
        return Err(BdmvError::NotEnoughPgsSlots {
            needed: ig_packets.len(),
            available,
        });
    }

    // Overwrite the reserved slots in place (ATS bytes untouched): no third copy.
    let null = null_ts_packet();
    let mut next = 0usize;
    for packet in patched.as_chunks_mut::<M2TS_PACKET_SIZE>().0 {
        if ts_pid(&packet[4..]) != pgs_pid {
            continue;
        }
        match ig_packets.get(next) {
            Some(ig) => {
                packet[4..].copy_from_slice(ig);
                next += 1;
            }
            None => packet[4..].copy_from_slice(&null),
        }
    }
    debug_assert_eq!(next, ig_packets.len());
    Ok(patched)
}

fn ts_pid(packet: &[u8]) -> u16 {
    (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16
}

fn ts_payload(packet: &[u8]) -> Option<&[u8]> {
    if packet.len() < 5 || packet[0] != 0x47 {
        return None;
    }
    if packet[3] & 0x10 == 0 {
        return None;
    }
    let offset = if packet[3] & 0x20 != 0 {
        5 + packet[4] as usize
    } else {
        4
    };
    packet.get(offset..TS_PACKET_SIZE)
}

fn null_ts_packet() -> [u8; TS_PACKET_SIZE] {
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = 0x1F;
    packet[2] = 0xFF;
    packet[3] = 0x10;
    packet
}

fn find_pmt_pid(m2ts: &[u8]) -> Result<u16, BdmvError> {
    for packet in m2ts.as_chunks::<M2TS_PACKET_SIZE>().0 {
        let ts = &packet[4..];
        if ts_pid(ts) != 0 || ts[1] & 0x40 == 0 {
            continue;
        }
        let Some(payload) = ts_payload(ts) else {
            continue;
        };
        let pointer = *payload.first().unwrap_or(&0) as usize;
        let Some(section) = payload.get(1 + pointer..) else {
            continue;
        };
        if section.first().copied() != Some(0x00) || section.len() < 12 {
            continue;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = 3 + section_length;
        if section.len() < end || end < 12 {
            continue;
        }
        let mut offset = 8usize;
        while offset + 4 <= end - 4 {
            let program = u16::from_be_bytes([section[offset], section[offset + 1]]);
            let pid = (((section[offset + 2] & 0x1F) as u16) << 8) | section[offset + 3] as u16;
            if program != 0 {
                return Ok(pid);
            }
            offset += 4;
        }
    }
    Err(BdmvError::MissingPmt)
}

fn find_pgs_pid(m2ts: &[u8], pmt_pid: u16) -> Result<u16, BdmvError> {
    for packet in m2ts.as_chunks::<M2TS_PACKET_SIZE>().0 {
        let ts = &packet[4..];
        if ts_pid(ts) != pmt_pid || ts[1] & 0x40 == 0 {
            continue;
        }
        let Some(payload) = ts_payload(ts) else {
            continue;
        };
        let pointer = *payload.first().unwrap_or(&0) as usize;
        let Some(section) = payload.get(1 + pointer..) else {
            continue;
        };
        if section.first().copied() != Some(0x02) {
            continue;
        }
        let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
        let end = 3 + section_length;
        if section.len() < end || end < 16 {
            continue;
        }
        let program_info_length = (((section[10] & 0x0F) as usize) << 8) | section[11] as usize;
        let mut offset = 12 + program_info_length;
        while offset + 5 <= end - 4 {
            let stream_type = section[offset];
            let pid = (((section[offset + 1] & 0x1F) as u16) << 8) | section[offset + 2] as u16;
            let info_length =
                (((section[offset + 3] & 0x0F) as usize) << 8) | section[offset + 4] as usize;
            if stream_type == PMT_STREAM_TYPE_PGS {
                return Ok(pid);
            }
            offset += 5 + info_length;
        }
    }
    Err(BdmvError::MissingPgsTrack)
}

fn patch_pmt(m2ts: &[u8], pmt_pid: u16, pgs_pid: u16, ig_pid: u16) -> Result<Vec<u8>, BdmvError> {
    if m2ts.is_empty() || !m2ts.len().is_multiple_of(M2TS_PACKET_SIZE) {
        return Err(BdmvError::InvalidM2ts);
    }
    let mut output = m2ts.to_vec();
    let mut patched = 0usize;
    for packet in output.as_chunks_mut::<M2TS_PACKET_SIZE>().0 {
        if patch_pmt_packet(packet, pmt_pid, pgs_pid, ig_pid)? {
            patched += 1;
        }
    }
    if patched == 0 {
        return Err(BdmvError::MissingPmt);
    }
    Ok(output)
}

/// Patches one 192-byte M2TS packet if it carries the PMT. Returns whether the
/// packet was a PMT that had its reserved PG stream retagged to IG.
fn patch_pmt_packet(
    packet: &mut [u8; M2TS_PACKET_SIZE],
    pmt_pid: u16,
    pgs_pid: u16,
    ig_pid: u16,
) -> Result<bool, BdmvError> {
    let ts = &mut packet[4..];
    if ts_pid(ts) != pmt_pid || ts[1] & 0x40 == 0 {
        return Ok(false);
    }
    let Some(payload_start) = ts_payload_start(ts) else {
        return Ok(false);
    };
    let pointer = ts[payload_start] as usize;
    let section_start = payload_start + 1 + pointer;
    if section_start + 12 > TS_PACKET_SIZE {
        return Ok(false);
    }
    let section = &mut ts[section_start..TS_PACKET_SIZE];
    if section[0] != 0x02 {
        return Ok(false);
    }
    let section_length = (((section[1] & 0x0F) as usize) << 8) | section[2] as usize;
    let end = 3 + section_length;
    if end > section.len() || end < 16 {
        return Err(BdmvError::FragmentedPmt);
    }
    if section[6] != 0 || section[7] != 0 {
        return Err(BdmvError::FragmentedPmt);
    }
    let program_info_length = (((section[10] & 0x0F) as usize) << 8) | section[11] as usize;
    let mut offset = 12 + program_info_length;
    while offset + 5 <= end - 4 {
        let info_length =
            (((section[offset + 3] & 0x0F) as usize) << 8) | section[offset + 4] as usize;
        let pid = (((section[offset + 1] & 0x1F) as u16) << 8) | section[offset + 2] as u16;
        if section[offset] == PMT_STREAM_TYPE_PGS && pid == pgs_pid {
            section[offset] = PMT_STREAM_TYPE_IG;
            section[offset + 1] = 0xE0 | ((ig_pid >> 8) as u8 & 0x1F);
            section[offset + 2] = (ig_pid & 0xFF) as u8;
            let crc = mpeg_crc32(&section[..end - 4]);
            section[end - 4..end].copy_from_slice(&crc.to_be_bytes());
            return Ok(true);
        }
        offset += 5 + info_length;
    }
    Err(BdmvError::MissingPgsTrack)
}

/// Bounded streaming form of [`inject_ig_into_menu_m2ts`].
///
/// The reader is scanned only up to 1 MiB to locate the PAT/PMT; the rewrite
/// then processes 192-byte packets through one reusable 192 KiB buffer. The
/// returned value is the number of source packets in the output.
pub fn inject_ig_into_menu_m2ts_stream<R: Read + Seek, W: Write>(
    reader: &mut R,
    writer: &mut W,
    ig_pes_packets: &[Vec<u8>],
    ig_pid: u16,
) -> Result<u32, BdmvError> {
    // Keep the prefix buffer an exact multiple of the 192-byte M2TS packet.
    // A raw 1 MiB buffer is not aligned (1,048,576 / 192 leaves a remainder)
    // and would make every menu larger than 1 MiB fail the alignment check.
    const PREFIX_SCAN_BYTES: usize = M2TS_PACKET_SIZE * (1024 * 1024 / M2TS_PACKET_SIZE);
    const PACKET_BUFFER_BYTES: usize = 192 * 1024;

    let mut prefix = vec![0u8; PREFIX_SCAN_BYTES];
    let mut prefix_len = 0usize;
    while prefix_len < prefix.len() {
        let read = reader.read(&mut prefix[prefix_len..])?;
        if read == 0 {
            break;
        }
        prefix_len += read;
    }
    prefix.truncate(prefix_len);
    if prefix.is_empty() || !prefix.len().is_multiple_of(M2TS_PACKET_SIZE) {
        return Err(BdmvError::InvalidM2ts);
    }
    let pmt_pid = find_pmt_pid(&prefix)?;
    let pgs_pid = find_pgs_pid(&prefix, pmt_pid)?;
    let ig_packets = packetize_pes(ig_pes_packets, ig_pid);
    let null = null_ts_packet();
    let mut next_ig = 0usize;
    let mut available = 0usize;
    let mut packets = 0u64;
    let mut patched_pmt = false;

    let mut write_packet = |packet: &[u8], writer: &mut W| -> Result<(), BdmvError> {
        let mut output = [0u8; M2TS_PACKET_SIZE];
        output.copy_from_slice(packet);
        if patch_pmt_packet(&mut output, pmt_pid, pgs_pid, ig_pid)? {
            patched_pmt = true;
        }
        if ts_pid(&output[4..]) == pgs_pid {
            available += 1;
            if let Some(ig) = ig_packets.get(next_ig) {
                output[4..].copy_from_slice(ig);
                next_ig += 1;
            } else {
                output[4..].copy_from_slice(&null);
            }
        }
        writer.write_all(&output)?;
        packets = packets
            .checked_add(1)
            .ok_or(BdmvError::PacketCountOverflow)?;
        Ok(())
    };

    for packet in prefix.as_chunks::<M2TS_PACKET_SIZE>().0 {
        write_packet(packet, writer)?;
    }
    reader.seek(SeekFrom::Start(prefix.len() as u64))?;
    let mut buffer = vec![0u8; PACKET_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if !read.is_multiple_of(M2TS_PACKET_SIZE) {
            return Err(BdmvError::InvalidM2ts);
        }
        for packet in buffer[..read].as_chunks::<M2TS_PACKET_SIZE>().0 {
            write_packet(packet, writer)?;
        }
    }
    if !patched_pmt {
        return Err(BdmvError::MissingPmt);
    }
    if next_ig < ig_packets.len() {
        return Err(BdmvError::NotEnoughPgsSlots {
            needed: ig_packets.len(),
            available,
        });
    }
    u32::try_from(packets).map_err(|_| BdmvError::PacketCountOverflow)
}

/// File-to-file wrapper around [`inject_ig_into_menu_m2ts_stream`]. The caller
/// is responsible for removing a partial output after an error.
pub fn inject_ig_into_menu_m2ts_file(
    input: &Path,
    output: &Path,
    ig_pes_packets: &[Vec<u8>],
    ig_pid: u16,
) -> Result<u32, BdmvError> {
    let mut reader = BufReader::new(File::open(input)?);
    let mut writer = BufWriter::new(File::create(output)?);
    let packets =
        inject_ig_into_menu_m2ts_stream(&mut reader, &mut writer, ig_pes_packets, ig_pid)?;
    writer.flush()?;
    Ok(packets)
}

fn ts_payload_start(ts: &[u8]) -> Option<usize> {
    if ts.len() < 5 || ts[0] != 0x47 || ts[3] & 0x10 == 0 {
        return None;
    }
    if ts[3] & 0x20 != 0 {
        let offset = 5 + ts[4] as usize;
        (offset < TS_PACKET_SIZE).then_some(offset)
    } else {
        Some(4)
    }
}

fn mpeg_crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= (*byte as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Rewrites a tsMuxeR menu playlist: clip id, still mode and the reserved PG
/// stream entry (counts, PID and coding type).
pub fn patch_menu_mpls(
    mpls: &[u8],
    clip_id: u16,
    still_time: Option<u16>,
) -> Result<Vec<u8>, BdmvError> {
    if mpls.len() < 40 || (&mpls[..8] != b"MPLS0200" && &mpls[..8] != b"MPLS0100") {
        return Err(BdmvError::InvalidMpls("firma MPLS0100/0200 ausente"));
    }
    let list_pos = read_u32(mpls, 8).ok_or(BdmvError::InvalidMpls("list_pos"))? as usize;
    let list_len =
        read_u32(mpls, list_pos).ok_or(BdmvError::InvalidMpls("playlist length"))? as usize;
    if list_pos + 4 + list_len > mpls.len() {
        return Err(BdmvError::InvalidMpls("playlist fuera de rango"));
    }
    let num_playitems =
        read_u16(mpls, list_pos + 6).ok_or(BdmvError::InvalidMpls("num_playitems"))?;
    if num_playitems != 1 {
        return Err(BdmvError::InvalidMpls(
            "el menú debe tener exactamente un playitem",
        ));
    }
    let offset = list_pos + 10;
    {
        let playitem_len =
            read_u16(mpls, offset).ok_or(BdmvError::InvalidMpls("playitem length"))? as usize;
        if playitem_len < 34 || offset + 2 + playitem_len > mpls.len() {
            return Err(BdmvError::InvalidMpls("playitem fuera de rango"));
        }
        let mut output = mpls.to_vec();
        let clip_start = offset + 2;
        output[clip_start..clip_start + 5].copy_from_slice(format!("{clip_id:05}").as_bytes());

        let q = clip_start;
        let still_mode = q + 29;
        let still_time_offset = q + 30;
        if still_time_offset + 2 > output.len() {
            return Err(BdmvError::InvalidMpls("still fuera de rango"));
        }
        match still_time {
            Some(seconds) => {
                output[still_mode] = 0x01;
                output[still_time_offset..still_time_offset + 2]
                    .copy_from_slice(&seconds.to_be_bytes());
            }
            None => {
                output[still_mode] = 0x00;
                output[still_time_offset..still_time_offset + 2].copy_from_slice(&[0, 0]);
            }
        }

        let stn = q + 32;
        let stn_len = read_u16(&output, stn).ok_or(BdmvError::InvalidMpls("STN length"))? as usize;
        if stn + 2 + stn_len > output.len() || stn_len < 14 {
            return Err(BdmvError::InvalidMpls("STN fuera de rango"));
        }
        let payload = stn + 2;
        let counts = payload + 2;
        let num_video = output[counts] as usize;
        let num_audio = output[counts + 1] as usize;
        let num_pg = output[counts + 2] as usize;
        let num_ig = output[counts + 3] as usize;
        if num_pg == 0 || num_ig != 0 {
            return Err(BdmvError::InvalidMpls("se esperaba una pista PG reservada"));
        }
        output[counts + 2] = 0;
        output[counts + 3] = 1;

        let mut cursor = counts + 8 + 4;
        let mut patched = false;
        for _ in 0..num_video + num_audio {
            cursor = skip_mpls_stream(&output, cursor)?;
        }
        for _ in 0..num_pg {
            let stream_len = *output
                .get(cursor)
                .ok_or(BdmvError::InvalidMpls("stream PG"))? as usize;
            let stream_start = cursor + 1;
            let attr_start = stream_start + stream_len;
            let attr_len = *output
                .get(attr_start)
                .ok_or(BdmvError::InvalidMpls("atributos PG"))? as usize;
            let coding_start = attr_start + 1;
            if coding_start >= output.len() {
                return Err(BdmvError::InvalidMpls("coding type PG"));
            }
            if output[stream_start] == 0x01 {
                // Plain stream (no sub-path): type + PID.
                let pid =
                    read_u16(&output, stream_start + 1).ok_or(BdmvError::InvalidMpls("PID PG"))?;
                if pid == PGS_PID {
                    output[stream_start + 1] = ((IG_PID >> 8) & 0xFF) as u8;
                    output[stream_start + 2] = (IG_PID & 0xFF) as u8;
                }
            }
            if output[coding_start] == PMT_STREAM_TYPE_PGS {
                output[coding_start] = PMT_STREAM_TYPE_IG;
                patched = true;
            }
            cursor = coding_start + attr_len;
        }
        if !patched {
            return Err(BdmvError::InvalidMpls("pista PG no encontrada"));
        }
        Ok(output)
    }
}

fn skip_mpls_stream(bytes: &[u8], cursor: usize) -> Result<usize, BdmvError> {
    let stream_len = *bytes.get(cursor).ok_or(BdmvError::InvalidMpls("stream"))? as usize;
    let stream_start = cursor + 1;
    let attr_start = stream_start + stream_len;
    let attr_len = *bytes
        .get(attr_start)
        .ok_or(BdmvError::InvalidMpls("atributos de stream"))? as usize;
    Ok(attr_start + 1 + attr_len)
}

/// Retags the reserved PG entry in a menu CLPI and updates the source packet
/// count after the IG packets have been injected.
pub fn patch_menu_clpi(clpi: &[u8], source_packets: u32) -> Result<Vec<u8>, BdmvError> {
    if clpi.len() < 60 || (&clpi[..8] != b"HDMV0200" && &clpi[..8] != b"HDMV0100") {
        return Err(BdmvError::InvalidClpi("firma HDMV0100/0200 ausente"));
    }
    let mut output = clpi.to_vec();
    output[56..60].copy_from_slice(&source_packets.to_be_bytes());

    let program_info_start =
        read_u32(clpi, 12).ok_or(BdmvError::InvalidClpi("program_info_start"))? as usize;
    if program_info_start + 6 > clpi.len() {
        return Err(BdmvError::InvalidClpi("program info fuera de rango"));
    }
    let program_info_length = read_u32(clpi, program_info_start)
        .ok_or(BdmvError::InvalidClpi("program length"))? as usize;
    if program_info_start + 4 + program_info_length > clpi.len() {
        return Err(BdmvError::InvalidClpi("program info truncado"));
    }
    let num_programs = clpi[program_info_start + 5] as usize;
    let mut offset = program_info_start + 6;
    let mut patched = false;
    for _ in 0..num_programs {
        if offset + 8 > clpi.len() {
            return Err(BdmvError::InvalidClpi("programa truncado"));
        }
        let num_streams = clpi[offset + 6] as usize;
        offset += 8;
        for _ in 0..num_streams {
            if offset + 3 > clpi.len() {
                return Err(BdmvError::InvalidClpi("stream truncado"));
            }
            let pid = read_u16(clpi, offset).ok_or(BdmvError::InvalidClpi("PID"))?;
            let attr_len = clpi[offset + 2] as usize;
            if offset + 3 + attr_len > clpi.len() {
                return Err(BdmvError::InvalidClpi("atributos truncados"));
            }
            if pid == PGS_PID {
                let coding = output[offset + 3];
                output[offset..offset + 2].copy_from_slice(&IG_PID.to_be_bytes());
                if coding == PMT_STREAM_TYPE_PGS {
                    output[offset + 3] = PMT_STREAM_TYPE_IG;
                    patched = true;
                }
            }
            offset += 3 + attr_len;
        }
    }
    if !patched {
        return Err(BdmvError::InvalidClpi("pista PG no encontrada"));
    }
    Ok(output)
}

/// Returns the number of MovieObjects in a MovieObject.bdmv file.
pub fn movie_object_count(movie_object: &[u8]) -> Result<u16, BdmvError> {
    if movie_object.len() < 50
        || (&movie_object[..8] != b"MOBJ0200" && &movie_object[..8] != b"MOBJ0100")
    {
        return Err(BdmvError::InvalidMovieObject("firma MOBJ0100/0200 ausente"));
    }
    read_u16(movie_object, 48).ok_or(BdmvError::InvalidMovieObject("num_objects"))
}

/// Appends a MovieObject that plays `playlist` and returns its object index.
pub fn append_menu_movie_object(
    movie_object: &[u8],
    playlist: u16,
) -> Result<(Vec<u8>, u16), BdmvError> {
    if movie_object.len() < 50
        || (&movie_object[..8] != b"MOBJ0200" && &movie_object[..8] != b"MOBJ0100")
    {
        return Err(BdmvError::InvalidMovieObject("firma MOBJ0100/0200 ausente"));
    }
    let data_len =
        read_u32(movie_object, 40).ok_or(BdmvError::InvalidMovieObject("data_len"))? as usize;
    let num_objects =
        read_u16(movie_object, 48).ok_or(BdmvError::InvalidMovieObject("num_objects"))?;
    if 44 + data_len != movie_object.len() {
        return Err(BdmvError::InvalidMovieObject("data_len incoherente"));
    }
    let mut output = movie_object.to_vec();
    output.extend_from_slice(&0x8000u16.to_be_bytes());
    // Two commands: play the menu playlist, then jump back to the top menu.
    // The second command turns the finite menu clip into a loop without using
    // still mode, so remote input stays live on all decoders.
    output.extend_from_slice(&2u16.to_be_bytes());
    output.extend_from_slice(&play_playlist(playlist as u32).to_bytes());
    // Loop this menu object after the finite menu clip ends.
    output.extend_from_slice(&jump_object(num_objects as u32).to_bytes());
    let new_len = data_len + 28;
    output[40..44].copy_from_slice(&(new_len as u32).to_be_bytes());
    output[48..50].copy_from_slice(&(num_objects + 1).to_be_bytes());
    Ok((output, num_objects))
}

/// Points First Playback and Top Menu to `object`.
pub fn set_index_menu_object(index: &[u8], object: u16) -> Result<Vec<u8>, BdmvError> {
    if index.len() < 40 || (&index[..8] != b"INDX0200" && &index[..8] != b"INDX0100") {
        return Err(BdmvError::InvalidIndex("firma INDX0100/0200 ausente"));
    }
    let index_start = read_u32(index, 8).ok_or(BdmvError::InvalidIndex("index_start"))? as usize;
    if index_start + 4 > index.len() {
        return Err(BdmvError::InvalidIndex("index_start fuera de rango"));
    }
    let index_len =
        read_u32(index, index_start).ok_or(BdmvError::InvalidIndex("index_len"))? as usize;
    if index_start + 4 + index_len > index.len() || index_len < 26 {
        return Err(BdmvError::InvalidIndex("index_len incoherente"));
    }
    let num_titles =
        read_u16(index, index_start + 4 + 24).ok_or(BdmvError::InvalidIndex("num_titles"))?;
    if index_len < 26 + num_titles as usize * 12 {
        return Err(BdmvError::InvalidIndex("tabla de títulos truncada"));
    }
    let mut output = index.to_vec();
    for entry in [index_start + 4, index_start + 16] {
        let object_type = output[entry] >> 6;
        if object_type != 1 {
            return Err(BdmvError::InvalidIndex("First Play/Top Menu no es HDMV"));
        }
        output[entry + 6..entry + 8].copy_from_slice(&object.to_be_bytes());
    }
    Ok(output)
}

/// Number of transport packets one dummy display set occupies once tsMuxeR
/// wraps its five PGS segments in a single PES. Used to reserve exactly as many
/// slots as the native IG stream needs.
pub fn pgs_transport_packet_count(width: u16, height: u16) -> usize {
    let width = usize::from(width.max(1));
    let height = usize::from(height.max(2));
    // PG + PTS + DTS + kind + length = 13 bytes per segment header.
    let palette = 2 + 256 * 4;
    let window = 10;
    let presentation = 25;
    // Worst-case RLE object: every row needs a 2-byte length plus up to one
    // run byte per pixel. This is deliberately conservative; under-reserving
    // PGS slots makes tsMuxeR drop or truncate the interactive stream.
    let object = 11 + height * (2 + width);
    let payload = 5 * 13 + palette + window + presentation + object;
    // PES start code + length + flags + header length + PTS + DTS = 19 bytes.
    (19 + payload).div_ceil(184).max(1)
}

/// Builds a dummy PGS stream with enough display sets to reserve at least
/// `needed_slots` transport packets. One extra display set absorbs small
/// differences between our estimate and the muxer's PES packing.
pub fn dummy_pgs_for_slots(
    width: u16,
    height: u16,
    needed_slots: usize,
) -> Result<Vec<u8>, BdmvError> {
    // The dummy object is a solid RLE fill, so `pgs_transport_packet_count`
    // (which deliberately uses a worst-case per-pixel RLE estimate) would
    // under-reserve display sets. Measure one real display set instead.
    let one_set = dummy_pgs(width, height, 1)?;
    let per_set = one_set.len().div_ceil(184).max(1);
    let sets = needed_slots
        .div_ceil(per_set)
        .saturating_add(1)
        .clamp(1, usize::from(u16::MAX));
    dummy_pgs(width, height, sets as u16)
}

/// Builds a raw PGS stream used only to reserve transport slots. Each display
/// set is a full-screen solid object; the payload is discarded after the IG
/// packets replace the reserved slots.
pub fn dummy_pgs(width: u16, height: u16, display_sets: u16) -> Result<Vec<u8>, BdmvError> {
    let mut output = Vec::new();
    let width = width.max(2);
    let height = height.max(2);
    for set in 0..display_sets.max(1) {
        // tsMuxeR 2.7 adds the 600 s BDAV origin to the SUP timestamps. Start
        // the dummy track at zero so its PES land at 54 000 000 (the PTS used
        // by the IG packets) and keep every reserved display set inside the
        // first few milliseconds of the clip. Spacing them by 0.5 s makes the
        // IG composition arrive long after the PTS of its ICS.
        let pts = u32::from(set) * 90;
        // tsMuxeR 2.7's SUP probe requires the compact 4-byte palette form
        // used by its own PGS samples. This track is discarded after muxing,
        // so compatibility with the muxer matters more than palette fidelity.
        let mut palette = Vec::with_capacity(2 + 256 * 4);
        palette.extend_from_slice(&[0, 0]);
        for index in 0..=255u8 {
            palette.extend_from_slice(&[
                if index == 0 { 16 } else { 235 },
                128,
                128,
                if index == 0 { 0 } else { 255 },
            ]);
        }
        push_pgs_segment(&mut output, 0x14, pts, &palette)?;

        let mut window = Vec::with_capacity(10);
        window.push(1); // one window
        window.push(0); // window id
        window.extend_from_slice(&0u16.to_be_bytes());
        window.extend_from_slice(&0u16.to_be_bytes());
        window.extend_from_slice(&width.to_be_bytes());
        window.extend_from_slice(&height.to_be_bytes());
        push_pgs_segment(&mut output, 0x17, pts, &window)?;

        let mut presentation = Vec::with_capacity(25);
        presentation.extend_from_slice(&width.to_be_bytes());
        presentation.extend_from_slice(&height.to_be_bytes());
        presentation.push(0x10); // 23.976 fps code
        presentation.extend_from_slice(&set.to_be_bytes());
        presentation.push(0x80); // epoch start
        presentation.push(0); // no palette update
        presentation.push(0); // palette id
        presentation.push(1); // one object reference
        presentation.extend_from_slice(&1u16.to_be_bytes());
        presentation.push(0); // window id
        presentation.push(0); // no crop, not forced
        presentation.extend_from_slice(&0u16.to_be_bytes());
        presentation.extend_from_slice(&0u16.to_be_bytes());
        push_pgs_segment(&mut output, 0x16, pts, &presentation)?;

        let mut rle = Vec::with_capacity(height as usize * 6);
        for _ in 0..height {
            rle.push(0x00);
            rle.push(0xC0 | ((width >> 8) as u8 & 0x3F));
            rle.push((width & 0xFF) as u8);
            rle.push(1);
            rle.extend_from_slice(&[0x00, 0x00]);
        }
        let object_data_len = 4 + rle.len();
        let mut object = Vec::with_capacity(11 + rle.len());
        object.extend_from_slice(&1u16.to_be_bytes());
        object.push(0);
        object.push(0xC0);
        object.extend_from_slice(&(object_data_len as u32).to_be_bytes()[1..]);
        object.extend_from_slice(&width.to_be_bytes());
        object.extend_from_slice(&height.to_be_bytes());
        object.extend_from_slice(&rle);
        push_pgs_segment(&mut output, 0x15, pts, &object)?;
        push_pgs_segment(&mut output, 0x80, pts, &[])?;
    }
    Ok(output)
}

fn push_pgs_segment(
    output: &mut Vec<u8>,
    kind: u8,
    pts: u32,
    payload: &[u8],
) -> Result<(), BdmvError> {
    let length = u16::try_from(payload.len()).map_err(|_| BdmvError::PgsSegmentTooLarge {
        kind,
        length: payload.len(),
    })?;
    output.extend_from_slice(b"PG");
    output.extend_from_slice(&pts.to_be_bytes());
    output.extend_from_slice(&pts.to_be_bytes());
    output.push(kind);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(payload);
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{
        compile_ig_pes_packets, HdmvButton, HdmvButtonState, HdmvMenu, RgbaBitmap,
    };
    use crate::opcodes::jump_title;

    fn menu() -> HdmvMenu {
        let solid = |w, h, rgba| RgbaBitmap::solid(w, h, rgba).expect("bitmap");
        HdmvMenu {
            width: 320,
            height: 180,
            frame_rate_code: 4,
            background: Some(solid(320, 180, [8, 12, 24, 255])),
            buttons: vec![HdmvButton {
                id: 1,
                x: 20,
                y: 20,
                up: None,
                down: None,
                left: None,
                right: None,
                commands: vec![jump_title(1)],
                state: HdmvButtonState {
                    normal: solid(120, 40, [20, 40, 70, 255]),
                    selected: solid(120, 40, [255, 255, 255, 255]),
                    activated: solid(120, 40, [34, 211, 238, 255]),
                },
            }],
            default_selected: Some(1),
        }
    }

    fn crc32_mpeg(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in bytes {
            crc ^= (*byte as u32) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04C1_1DB7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    fn pgs_display_set_pts(bytes: &[u8]) -> Vec<u32> {
        let mut pts = Vec::new();
        let mut offset = 0usize;
        while offset + 13 <= bytes.len() {
            assert_eq!(&bytes[offset..offset + 2], b"PG");
            let timestamp = u32::from_be_bytes([
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
            ]);
            if pts.last().copied() != Some(timestamp) {
                pts.push(timestamp);
            }
            let length = u16::from_be_bytes([bytes[offset + 11], bytes[offset + 12]]) as usize;
            offset += 13 + length;
        }
        assert_eq!(offset, bytes.len());
        pts
    }

    fn synthetic_m2ts(pgs_packets: usize) -> Vec<u8> {
        let ats: Vec<u32> = (0..pgs_packets)
            .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
            .collect();
        synthetic_m2ts_with_slot_ats(&ats)
    }

    fn synthetic_m2ts_with_slot_ats(slot_ats: &[u32]) -> Vec<u8> {
        let mut output = Vec::new();
        let pat = [
            0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE1, 0x00,
        ];
        let mut pat_section = pat.to_vec();
        pat_section.extend_from_slice(&crc32_mpeg(&pat).to_be_bytes());
        let mut pat_packet = [0xFFu8; 188];
        pat_packet[0] = 0x47;
        pat_packet[1] = 0x40;
        pat_packet[2] = 0x00;
        pat_packet[3] = 0x10;
        pat_packet[4] = 0x00;
        pat_packet[5..5 + pat_section.len()].copy_from_slice(&pat_section);
        output.extend_from_slice(&[0, 0, 0, 0]);
        output.extend_from_slice(&pat_packet);

        let mut pmt = vec![
            0x02, 0xB0, 0x17, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xF0, 0x11, 0xF0, 0x00, 0x1B, 0xF0,
            0x11, 0xF0, 0x00, 0x90, 0xF2, 0x00, 0xF0, 0x00,
        ];
        let crc = crc32_mpeg(&pmt);
        pmt.extend_from_slice(&crc.to_be_bytes());
        let mut pmt_packet = [0xFFu8; 188];
        pmt_packet[0] = 0x47;
        pmt_packet[1] = 0x41;
        pmt_packet[2] = 0x00;
        pmt_packet[3] = 0x10;
        pmt_packet[4] = 0x00;
        pmt_packet[5..5 + pmt.len()].copy_from_slice(&pmt);
        output.extend_from_slice(&[0, 0, 0, 1]);
        output.extend_from_slice(&pmt_packet);

        for ats in slot_ats {
            let mut pgs = [0xFFu8; 188];
            pgs[0] = 0x47;
            pgs[1] = 0x12;
            pgs[2] = 0x00;
            pgs[3] = 0x10;
            pgs[4] = 0x00;
            output.extend_from_slice(&ats.to_be_bytes());
            output.extend_from_slice(&pgs);
        }
        output
    }

    #[test]
    fn packetizes_each_pes_with_its_own_start_and_adapted_tail() {
        let packets = vec![vec![0xAB; 200], vec![0xCD; 20]];
        let ts = packetize_pes(&packets, IG_PID);
        assert_eq!(ts.len(), 3);
        assert!(ts[0][1] & 0x40 != 0);
        assert_eq!(ts[1][1] & 0x40, 0);
        assert!(ts[2][1] & 0x40 != 0);
        assert_eq!(ts[1][4], 183 - 16);
        assert_eq!(ts[2][4], 183 - 20);
        for packet in &ts {
            assert_eq!(packet[0], 0x47);
            let pid = (((packet[1] & 0x1F) as u16) << 8) | packet[2] as u16;
            assert_eq!(pid, IG_PID);
        }
    }

    #[test]
    fn streaming_injector_matches_the_in_memory_implementation() {
        use std::io::Cursor;
        const ICS_PTS: u32 = 54_000_000;
        let pes = compile_ig_pes_packets(&menu(), u64::from(ICS_PTS)).expect("pes");
        let needed = pes_transport_packet_count(&pes);
        let m2ts = synthetic_m2ts(needed + 3);
        let expected = inject_ig_into_menu_m2ts(&m2ts, &pes, IG_PID).expect("in-memory inject");
        let mut reader = Cursor::new(m2ts.clone());
        let mut output = Vec::new();
        let packets = inject_ig_into_menu_m2ts_stream(&mut reader, &mut output, &pes, IG_PID)
            .expect("streaming inject");
        assert_eq!(output, expected);
        assert_eq!(packets as usize, m2ts.len() / M2TS_PACKET_SIZE);
        assert!(packets > 0);
    }

    #[test]
    fn injects_ig_and_retags_the_reserved_pgs_stream() {
        let pes = compile_ig_pes_packets(&menu(), 54_000_000).expect("pes");
        let needed = pes_transport_packet_count(&pes);
        let m2ts = synthetic_m2ts(needed);
        let output = inject_ig_into_menu_m2ts(&m2ts, &pes, IG_PID).expect("inject");
        // One reserved transport packet is replaced by exactly one IG packet, so
        // the SPN table and the CLPI EP_map stay valid.
        assert_eq!(output.len(), m2ts.len());
        assert!(output.as_chunks::<192>().0.iter().any(|packet| {
            let pid = (((packet[5] & 0x1F) as u16) << 8) | packet[6] as u16;
            pid == IG_PID
        }));
        let pids: Vec<u16> = output
            .as_chunks::<192>()
            .0
            .iter()
            .map(|packet| (((packet[5] & 0x1F) as u16) << 8) | packet[6] as u16)
            .collect();
        assert!(!pids.contains(&PGS_PID), "pids: {pids:04X?}");
    }

    #[test]
    fn reassembles_the_ig_pes_stream_from_injected_transport_packets() {
        let pes = compile_ig_pes_packets(&menu(), 54_000_000).expect("pes");
        let needed = pes_transport_packet_count(&pes);
        let output =
            inject_ig_into_menu_m2ts(&synthetic_m2ts(needed), &pes, IG_PID).expect("inject");
        let mut reassembled = Vec::new();
        for packet in output.as_chunks::<192>().0 {
            let ts = &packet[4..];
            let pid = (((ts[1] & 0x1F) as u16) << 8) | ts[2] as u16;
            if pid != IG_PID {
                continue;
            }
            let adaptation = (ts[3] >> 4) & 0x03;
            let offset = if adaptation == 3 {
                5 + ts[4] as usize
            } else {
                4
            };
            reassembled.extend_from_slice(&ts[offset..]);
        }
        assert_eq!(reassembled, pes.concat());
        assert_eq!(&reassembled[..4], &[0x00, 0x00, 0x01, 0xBD]);
    }

    #[test]
    fn rejects_ig_streams_that_do_not_fit_the_reserved_slots() {
        let pes = compile_ig_pes_packets(&menu(), 54_000_000).expect("pes");
        let needed = pes_transport_packet_count(&pes);
        let m2ts = synthetic_m2ts(needed - 1);
        let error = inject_ig_into_menu_m2ts(&m2ts, &pes, IG_PID).expect_err("slots");
        assert_eq!(
            error,
            BdmvError::NotEnoughPgsSlots {
                needed,
                available: needed - 1,
            }
        );
    }

    #[test]
    fn appends_a_play_playlist_movie_object() {
        let mut movie_object = vec![0u8; 50];
        movie_object[..8].copy_from_slice(b"MOBJ0200");
        movie_object[40..44].copy_from_slice(&6u32.to_be_bytes());
        movie_object[48..50].copy_from_slice(&0u16.to_be_bytes());
        let (patched, index) = append_menu_movie_object(&movie_object, 1).expect("mobj");
        assert_eq!(index, 0);
        assert_eq!(&patched[..8], b"MOBJ0200");
        assert_eq!(patched.len(), 78);
        assert_eq!(
            u32::from_be_bytes([patched[40], patched[41], patched[42], patched[43]]),
            34
        );
        assert_eq!(u16::from_be_bytes([patched[48], patched[49]]), 1);
        assert_eq!(u16::from_be_bytes([patched[52], patched[53]]), 2);
        assert_eq!(&patched[54..66], &play_playlist(1).to_bytes());
        assert_eq!(&patched[66..78], &jump_object(0).to_bytes());
        let (patched_again, index) = append_menu_movie_object(&patched, 2).expect("mobj 2");
        assert_eq!(index, 1);
        assert_eq!(
            u16::from_be_bytes([patched_again[48], patched_again[49]]),
            2
        );
    }

    #[test]
    fn points_first_play_and_top_menu_to_the_new_object() {
        let mut index = vec![0u8; 90];
        index[..8].copy_from_slice(b"INDX0200");
        index[8..12].copy_from_slice(&40u32.to_be_bytes());
        index[40..44].copy_from_slice(&38u32.to_be_bytes());
        for entry in [44, 56] {
            index[entry] = 0x40;
            index[entry + 6..entry + 8].copy_from_slice(&1u16.to_be_bytes());
        }
        index[68..70].copy_from_slice(&1u16.to_be_bytes());
        index[70] = 0x40;
        let patched = set_index_menu_object(&index, 3).expect("index");
        assert_eq!(u16::from_be_bytes([patched[50], patched[51]]), 3);
        assert_eq!(u16::from_be_bytes([patched[62], patched[63]]), 3);
    }

    #[test]
    fn retags_mpls_pg_entry_and_can_disable_still_mode() {
        let mut mpls = vec![0u8; 40];
        mpls[..8].copy_from_slice(b"MPLS0200");
        mpls[8..12].copy_from_slice(&40u32.to_be_bytes());
        let mut playlist = vec![0u8; 10];
        playlist[6..8].copy_from_slice(&1u16.to_be_bytes()); // one playitem
        playlist[8..10].copy_from_slice(&0u16.to_be_bytes()); // no subpaths
        let mut playitem = vec![0u8; 2];
        playitem.extend_from_slice(b"00000");
        playitem.extend_from_slice(b"M2TS");
        playitem.push(0);
        playitem.push(0x10);
        playitem.push(0);
        playitem.extend_from_slice(&27_000_000u32.to_be_bytes());
        playitem.extend_from_slice(&29_700_000u32.to_be_bytes());
        playitem.extend_from_slice(&[0u8; 8]);
        playitem.push(0);
        playitem.push(0);
        playitem.extend_from_slice(&[0, 0]);
        let mut stn = vec![0u8; 2];
        stn.extend_from_slice(&[0, 0]);
        stn.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0]);
        stn.extend_from_slice(&[0u8; 4]);
        let mut video = vec![9, 1];
        video.extend_from_slice(&0x1011u16.to_be_bytes());
        video.extend_from_slice(&[0u8; 6]);
        video.extend_from_slice(&[5, 0x1B, 0, 0, 0, 0]);
        stn.extend_from_slice(&video);
        let mut pg = vec![9, 1];
        pg.extend_from_slice(&PGS_PID.to_be_bytes());
        pg.extend_from_slice(&[0u8; 6]);
        pg.extend_from_slice(&[5, PMT_STREAM_TYPE_PGS, b'u', b'n', b'd', 0]);
        stn.extend_from_slice(&pg);
        let stn_len = (stn.len() - 2) as u16;
        stn[..2].copy_from_slice(&stn_len.to_be_bytes());
        playitem.extend_from_slice(&stn);
        let playitem_len = (playitem.len() - 2) as u16;
        playitem[..2].copy_from_slice(&playitem_len.to_be_bytes());
        playlist.extend_from_slice(&playitem);
        let playlist_len = (playlist.len() - 4) as u32;
        playlist[..4].copy_from_slice(&playlist_len.to_be_bytes());
        mpls.extend_from_slice(&playlist);

        let patched = patch_menu_mpls(&mpls, 1, None).expect("mpls");
        let playitem_start = 40 + 10;
        assert_eq!(&patched[playitem_start + 2..playitem_start + 7], b"00001");
        let counts = playitem_start + 2 + 32 + 2 + 2;
        assert_eq!(patched[counts + 2], 0);
        assert_eq!(patched[counts + 3], 1);
        assert!(patched
            .windows(2)
            .any(|window| window == IG_PID.to_be_bytes()));
        assert!(patched.contains(&PMT_STREAM_TYPE_IG));
        assert_eq!(patched[playitem_start + 31], 0);
    }

    #[test]
    fn retags_clpi_pg_entry_and_source_packets() {
        let mut clpi = vec![0u8; 60];
        clpi[..8].copy_from_slice(b"HDMV0200");
        clpi[12..16].copy_from_slice(&60u32.to_be_bytes());
        let mut program = vec![0u8; 4];
        program.push(0);
        program.push(1);
        program.extend_from_slice(&0u32.to_be_bytes());
        program.extend_from_slice(&0x0100u16.to_be_bytes());
        program.push(2);
        program.push(0);
        program.extend_from_slice(&0x1011u16.to_be_bytes());
        program.push(21);
        program.push(0x1B);
        program.extend_from_slice(&[0u8; 20]);
        program.extend_from_slice(&PGS_PID.to_be_bytes());
        program.push(21);
        program.push(PMT_STREAM_TYPE_PGS);
        program.extend_from_slice(b"und");
        program.extend_from_slice(&[0u8; 17]);
        let program_len = (program.len() - 4) as u32;
        program[..4].copy_from_slice(&program_len.to_be_bytes());
        clpi.extend_from_slice(&program);

        let patched = patch_menu_clpi(&clpi, 1234).expect("clpi");
        assert_eq!(
            u32::from_be_bytes([patched[56], patched[57], patched[58], patched[59]]),
            1234
        );
        assert!(patched
            .windows(2)
            .any(|window| window == IG_PID.to_be_bytes()));
        assert!(patched.contains(&PMT_STREAM_TYPE_IG));
    }

    #[test]
    fn dummy_pgs_contains_full_display_sets() {
        let bytes = dummy_pgs(320, 180, 2).expect("dummy pgs");
        assert!(bytes.starts_with(b"PG"));
        let mut offset = 0;
        let mut segments = 0;
        while offset + 13 <= bytes.len() {
            assert_eq!(&bytes[offset..offset + 2], b"PG");
            let length = u16::from_be_bytes([bytes[offset + 11], bytes[offset + 12]]) as usize;
            offset += 13 + length;
            segments += 1;
        }
        assert_eq!(offset, bytes.len());
        assert_eq!(segments, 10);
    }

    #[test]
    fn dummy_pgs_groups_display_sets_at_the_start_of_the_clip() {
        let bytes = dummy_pgs(320, 180, 25).expect("dummy pgs");
        let pts = pgs_display_set_pts(&bytes);
        assert_eq!(pts.len(), 25);
        assert_eq!(pts[0], 0);
        assert_eq!(pts[24], 24 * 90);
        assert!(
            pts[24] - pts[0] < 90_000,
            "los display sets deben caber en un segundo"
        );
    }

    #[test]
    fn injected_ig_slots_arrive_before_the_ics_presentation_time() {
        const ICS_PTS: u32 = 54_000_000;
        let pes = compile_ig_pes_packets(&menu(), u64::from(ICS_PTS)).expect("pes");
        let needed = pes_transport_packet_count(&pes);
        let per_set = pgs_transport_packet_count(320, 180);
        let sets = needed.div_ceil(per_set).saturating_add(1);
        let dummy = dummy_pgs(320, 180, u16::try_from(sets).expect("sets")).expect("dummy pgs");
        let display_pts = pgs_display_set_pts(&dummy);
        assert!(display_pts.len() >= sets);

        // tsMuxeR places the reserved PGS PES near the start of the clip (about
        // 3 s of ATS lead for this menu). Model that lead: with the old 0.5 s
        // spacing, or with the old 54 000 000 SUP base, the later slots cross
        // the ICS PTS while the grouped slots starting at zero stay below it.
        let lead_27mhz = 81_000_000u64;
        let mut slot_ats = Vec::with_capacity(needed);
        for pts in &display_pts {
            let ats = ((lead_27mhz + u64::from(*pts) * 300) & 0x3FFF_FFFF) as u32;
            for _ in 0..per_set {
                slot_ats.push(ats);
            }
        }
        slot_ats.truncate(needed);
        assert_eq!(slot_ats.len(), needed);

        let m2ts = synthetic_m2ts_with_slot_ats(&slot_ats);
        let output = inject_ig_into_menu_m2ts(&m2ts, &pes, IG_PID).expect("inject");
        let ics_ats = ((u64::from(ICS_PTS) * 300) & 0x3FFF_FFFF) as u32;
        let ig_ats: Vec<u32> = output
            .as_chunks::<192>()
            .0
            .iter()
            .filter(|packet| (((packet[5] & 0x1F) as u16) << 8 | packet[6] as u16) == IG_PID)
            .map(|packet| u32::from_be_bytes([packet[0], packet[1], packet[2], packet[3]]))
            .collect();
        assert_eq!(ig_ats.len(), needed);
        assert!(
            ig_ats.iter().all(|ats| *ats < ics_ats),
            "ATS de las ranuras IG: {ig_ats:?}, PTS del ICS: {ics_ats}"
        );
    }

    #[test]
    fn dummy_pgs_for_slots_scales_display_sets_with_the_estimate() {
        let one_set = dummy_pgs(1920, 1080, 1).expect("one dummy set");
        let per_set = one_set.len().div_ceil(184).max(1);
        let needed = per_set * 3;
        let bytes = dummy_pgs_for_slots(1920, 1080, needed).expect("dummy pgs");
        let mut offset = 0;
        let mut segments = 0;
        while offset + 13 <= bytes.len() {
            assert_eq!(&bytes[offset..offset + 2], b"PG");
            let length = u16::from_be_bytes([bytes[offset + 11], bytes[offset + 12]]) as usize;
            offset += 13 + length;
            segments += 1;
        }
        assert_eq!(offset, bytes.len());
        assert_eq!(segments, 5 * (needed.div_ceil(per_set) + 1));
    }

    #[test]
    fn pgs_slot_estimate_accounts_for_the_object_height_and_width() {
        assert!(pgs_transport_packet_count(1920, 1080) > pgs_transport_packet_count(1920, 480));
        assert!(pgs_transport_packet_count(1920, 1080) > pgs_transport_packet_count(720, 1080));
        assert!(pgs_transport_packet_count(1920, 0) >= 1);
        assert_ne!(
            pgs_transport_packet_count(640, 480),
            pgs_transport_packet_count(1920, 480)
        );
    }
}
