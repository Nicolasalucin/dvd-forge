//! PES packetization for the HDMV Interactive Graphics stream.

use crate::segments::Segment;

/// Blu-ray Interactive Graphics PID.
pub const IG_PID: u16 = 0x1400;
/// M2TS stream type for HDMV Interactive Graphics.
pub const IG_STREAM_TYPE: u8 = 0x91;
/// PES private_stream_1, used by Blu-ray graphics streams.
pub const IG_STREAM_ID: u8 = 0xBD;

const MAX_PES_PAYLOAD: usize = 65_000;

/// Wraps one or more IG segments in PES packets. Large payloads are split at
/// PES boundaries while preserving the segment byte stream.
pub fn pes_packet(payload: &[u8], pts_90k: u64) -> Vec<u8> {
    if payload.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for chunk in payload.chunks(MAX_PES_PAYLOAD) {
        let packet_length = 8 + chunk.len();
        out.extend_from_slice(&[0x00, 0x00, 0x01, IG_STREAM_ID]);
        out.extend_from_slice(&(packet_length.min(0xFFFF) as u16).to_be_bytes());
        out.push(0x80); // marker bits
        out.push(0x80); // PTS only
        out.push(5); // PTS is five bytes
        out.extend_from_slice(&encode_pts(pts_90k));
        out.extend_from_slice(chunk);
    }
    out
}

pub fn encode_segments(segments: &[Segment]) -> Vec<u8> {
    let mut payload = Vec::new();
    for segment in segments {
        payload.extend_from_slice(&segment.encode());
    }
    payload
}

fn encode_pts(pts_90k: u64) -> [u8; 5] {
    let pts = pts_90k & 0x1_FFFF_FFFF;
    [
        0x21 | (((pts >> 30) as u8 & 0x07) << 1),
        ((pts >> 22) & 0xFF) as u8,
        (((pts >> 15) as u8 & 0x7F) << 1) | 0x01,
        ((pts >> 7) & 0xFF) as u8,
        (((pts as u8) & 0x7F) << 1) | 0x01,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pes_header_uses_private_stream_one_and_pts() {
        let packet = pes_packet(&[0xAA, 0xBB], 90_000);
        assert_eq!(&packet[..4], &[0, 0, 1, 0xBD]);
        assert_eq!(u16::from_be_bytes([packet[4], packet[5]]) as usize, 10);
        assert_eq!(packet[6], 0x80);
        assert_eq!(packet[7], 0x80);
        assert_eq!(packet[8], 5);
        assert_eq!(&packet[14..], &[0xAA, 0xBB]);
    }

    #[test]
    fn large_payloads_are_split_into_multiple_pes_packets() {
        let payload = vec![0u8; MAX_PES_PAYLOAD + 10];
        let packet = pes_packet(&payload, 0);
        assert!(packet.len() > payload.len());
        assert_eq!(&packet[..4], &[0, 0, 1, 0xBD]);
        let first_payload = 14;
        assert_eq!(
            u16::from_be_bytes([packet[4], packet[5]]) as usize,
            8 + MAX_PES_PAYLOAD
        );
        assert_eq!(&packet[first_payload..first_payload + 4], &[0, 0, 0, 0]);
    }
}
