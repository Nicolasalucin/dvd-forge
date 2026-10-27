//! PES packetization for the HDMV Interactive Graphics stream.

use crate::segments::{Segment, SegmentError};
use thiserror::Error;

/// Blu-ray Interactive Graphics PID.
pub const IG_PID: u16 = 0x1400;
/// M2TS stream type for HDMV Interactive Graphics.
pub const IG_STREAM_TYPE: u8 = 0x91;
/// PES private_stream_1, used by Blu-ray graphics streams.
pub const IG_STREAM_ID: u8 = 0xBD;

/// PES header bytes after the 16-bit packet length: marker/flags/header length
/// (3) plus a five-byte PTS and a five-byte DTS.
const PES_HEADER_AFTER_LENGTH: usize = 3 + 10;
const MAX_PES_PAYLOAD: usize = 0xFFFF - PES_HEADER_AFTER_LENGTH;
const MAX_TIMESTAMP: u64 = 0x1_FFFF_FFFF;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PesError {
    #[error("el segmento {index} no se pudo codificar: {source}")]
    Segment {
        index: usize,
        #[source]
        source: SegmentError,
    },
    #[error(
        "el segmento {index} ocupa {length} bytes y no cabe en un PES de 16 bits ({max} bytes)"
    )]
    PayloadTooLarge {
        index: usize,
        length: usize,
        max: usize,
    },
    #[error("el PTS {0} supera los 33 bits admitidos por MPEG-2")]
    TimestampOverflow(u64),
}

/// Wraps each IG segment in its own PES packet with PTS and DTS.
///
/// Blu-ray graphics decoders treat every PES as one segment, so a segment is
/// never split across PES packets and several segments are never packed into
/// the same PES. Oversized segments are rejected instead of being truncated.
pub fn pes_packets_from_segments(
    segments: &[Segment],
    pts_90k: u64,
) -> Result<Vec<Vec<u8>>, PesError> {
    if pts_90k > MAX_TIMESTAMP {
        return Err(PesError::TimestampOverflow(pts_90k));
    }
    let mut output = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let payload = segment
            .encode()
            .map_err(|source| PesError::Segment { index, source })?;
        if payload.len() > MAX_PES_PAYLOAD {
            return Err(PesError::PayloadTooLarge {
                index,
                length: payload.len(),
                max: MAX_PES_PAYLOAD,
            });
        }
        output.push(pes_packet(&payload, pts_90k));
    }
    Ok(output)
}

fn pes_packet(payload: &[u8], pts_90k: u64) -> Vec<u8> {
    let packet_length = PES_HEADER_AFTER_LENGTH + payload.len();
    let mut out = Vec::with_capacity(6 + packet_length);
    out.extend_from_slice(&[0x00, 0x00, 0x01, IG_STREAM_ID]);
    out.extend_from_slice(&(packet_length as u16).to_be_bytes());
    out.push(0x80); // marker bits
    out.push(0xC0); // PTS and DTS present
    out.push(10); // PTS + DTS are ten bytes
    push_timestamp(&mut out, 0x20, pts_90k); // '0010' PTS
    push_timestamp(&mut out, 0x10, pts_90k); // '0001' DTS
    out.extend_from_slice(payload);
    out
}

fn push_timestamp(out: &mut Vec<u8>, prefix: u8, timestamp: u64) {
    let ts = timestamp & MAX_TIMESTAMP;
    out.push(prefix | (((ts >> 29) as u8) & 0x0E) | 0x01);
    out.push((ts >> 22) as u8);
    out.push((((ts >> 14) as u8) & 0xFE) | 0x01);
    out.push((ts >> 7) as u8);
    out.push(((ts as u8) << 1) | 0x01);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segments::{SEGMENT_END, SEGMENT_PDS};

    fn decode_timestamp(bytes: &[u8]) -> u64 {
        (u64::from((bytes[0] >> 1) & 0x07) << 30)
            | (u64::from(bytes[1]) << 22)
            | (u64::from((bytes[2] >> 1) & 0x7F) << 15)
            | (u64::from(bytes[3]) << 7)
            | u64::from(bytes[4] >> 1)
    }

    #[test]
    fn pes_header_uses_private_stream_one_and_pts_dts() {
        let segments = [Segment::new(SEGMENT_PDS, vec![0xAA, 0xBB])];
        let packets = pes_packets_from_segments(&segments, 90_000).expect("pes");
        let packet = &packets[0];
        assert_eq!(&packet[..4], &[0, 0, 1, 0xBD]);
        assert_eq!(
            u16::from_be_bytes([packet[4], packet[5]]) as usize,
            PES_HEADER_AFTER_LENGTH + 5
        );
        assert_eq!(packet[6], 0x80);
        assert_eq!(packet[7], 0xC0);
        assert_eq!(packet[8], 10);
        assert_eq!(decode_timestamp(&packet[9..14]), 90_000);
        assert_eq!(decode_timestamp(&packet[14..19]), 90_000);
        assert_eq!(&packet[19..], &[SEGMENT_PDS, 0, 2, 0xAA, 0xBB]);
    }

    #[test]
    fn keeps_one_pes_per_segment() {
        let segments = [
            Segment::new(SEGMENT_PDS, vec![1, 2, 3]),
            Segment::new(SEGMENT_END, Vec::new()),
        ];
        let packets = pes_packets_from_segments(&segments, 0).expect("pes");
        assert_eq!(packets.len(), 2);
        assert_eq!(&packets[0][19..], &[SEGMENT_PDS, 0, 3, 1, 2, 3]);
        assert_eq!(&packets[1][19..], &[0x80, 0, 0]);
    }

    #[test]
    fn rejects_oversized_segments_instead_of_truncating() {
        let segments = [Segment::new(SEGMENT_PDS, vec![0; 0xFFFF])];
        let error = pes_packets_from_segments(&segments, 0).expect_err("oversized");
        assert!(matches!(error, PesError::PayloadTooLarge { .. }));
    }

    #[test]
    fn rejects_timestamps_above_33_bits() {
        let error =
            pes_packets_from_segments(&[], MAX_TIMESTAMP + 1).expect_err("timestamp overflow");
        assert_eq!(error, PesError::TimestampOverflow(MAX_TIMESTAMP + 1));
    }
}
