//! Minimal Blu-ray M2TS wrapper for a standalone HDMV IG stream.
//!
//! tsMuxeR 2.7 only exposes `S_HDMV/PGS`, so the normal mux path uses that
//! compatibility label. This writer produces the canonical IG transport
//! stream (stream type `0x91`, PID `0x1400`) for dedicated muxers and for
//! debugging with `libbluray`.

use crate::pes::IG_PID;

pub const TS_PACKET_SIZE: usize = 188;
pub const M2TS_PACKET_SIZE: usize = 192;
const PMT_PID: u16 = 0x1000;
const PCR_PID: u16 = IG_PID;

pub fn m2ts_from_pes_packets(packets: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut continuity = 0u8;
    write_section_packet(&mut out, 0, &pat_section(), &mut continuity);
    write_section_packet(&mut out, PMT_PID, &pmt_section(), &mut continuity);
    let transport = crate::bdmv::packetize_pes(packets, IG_PID);
    for (index, packet) in transport.iter().enumerate() {
        let arrival = (index as u32) << 8;
        out.extend_from_slice(&arrival.to_be_bytes());
        out.extend_from_slice(packet);
    }
    out
}

fn write_section_packet(out: &mut Vec<u8>, pid: u16, section: &[u8], continuity: &mut u8) {
    let mut packet = [0xFFu8; TS_PACKET_SIZE];
    packet[0] = 0x47;
    packet[1] = 0x40 | ((pid >> 8) as u8 & 0x1F);
    packet[2] = (pid & 0xFF) as u8;
    packet[3] = 0x10 | (*continuity & 0x0F);
    let pointer = 0u8;
    packet[4] = pointer;
    packet[5..5 + section.len()].copy_from_slice(section);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&packet);
}

fn pat_section() -> Vec<u8> {
    let mut section = vec![
        0x00, // table_id
        0xB0,
        0x0D, // section_syntax_indicator + section_length 13
        0x00,
        0x01, // transport_stream_id
        0xC1, // version 0, current
        0x00,
        0x00, // section_number, last_section_number
        0x00,
        0x01, // program_number
        0xE0 | ((PMT_PID >> 8) as u8 & 0x1F),
        (PMT_PID & 0xFF) as u8,
    ];
    append_crc(&mut section);
    section
}

fn pmt_section() -> Vec<u8> {
    let mut section = vec![
        0x02, // table_id
        0xB0,
        0x12, // section_length 18
        0x00,
        0x01, // program_number
        0xC1, // version 0, current
        0x00,
        0x00, // section_number, last_section_number
        0xE0 | ((PCR_PID >> 8) as u8 & 0x1F),
        (PCR_PID & 0xFF) as u8,
        0xF0,
        0x00, // program_info_length
        0x91, // HDMV Interactive Graphics
        0xE0 | ((IG_PID >> 8) as u8 & 0x1F),
        (IG_PID & 0xFF) as u8,
        0xF0,
        0x00, // ES_info_length
    ];
    append_crc(&mut section);
    section
}

fn append_crc(section: &mut Vec<u8>) {
    let crc = mpeg_crc32(section);
    section.extend_from_slice(&crc.to_be_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{compile_ig_m2ts, HdmvButton, HdmvButtonState, HdmvMenu, RgbaBitmap};
    use crate::opcodes::jump_title;

    fn menu() -> HdmvMenu {
        let make = |w, h, rgba| RgbaBitmap::solid(w, h, rgba).expect("bitmap");
        HdmvMenu {
            width: 640,
            height: 360,
            frame_rate_code: 4,
            background: Some(make(640, 360, [8, 12, 24, 255])),
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
                    normal: make(200, 60, [20, 40, 70, 255]),
                    selected: make(200, 60, [255, 255, 255, 255]),
                    activated: make(200, 60, [34, 211, 238, 255]),
                },
            }],
            default_selected: Some(1),
        }
    }

    #[test]
    fn writes_aligned_m2ts_with_ig_pid_and_stream_type() {
        let bytes = compile_ig_m2ts(&menu(), 0).expect("m2ts");
        assert!(bytes.len() >= M2TS_PACKET_SIZE);
        assert_eq!(bytes.len() % M2TS_PACKET_SIZE, 0);
        for offset in (0..bytes.len()).step_by(M2TS_PACKET_SIZE) {
            assert_eq!(bytes[offset + 4], 0x47);
        }
        assert!(bytes
            .as_chunks::<M2TS_PACKET_SIZE>()
            .0
            .iter()
            .any(|packet| (packet[5] & 0x1F) == 0x14 && packet[6] == 0x00));
        assert!(bytes.windows(2).any(|window| window == [0x91, 0xF4]));
    }
}
