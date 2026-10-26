//! Blu-ray run-length encoding for Object Definition Segments.
//!
//! The grammar is the one implemented by libbluray's `_decode_rle`:
//!
//! * `color != 0` (8 bits): one pixel of that palette index.
//! * `0x00` then `0` and a length: a run of transparent index 0.
//! * `0x00` then `1` and a length then a color byte: a run of that color.
//!
//! The bit after the marker selects 6-bit or 14-bit lengths.

use thiserror::Error;

const MAX_RUN: usize = (1 << 14) - 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RleError {
    #[error("la secuencia RLE termina antes de completar {0} píxeles")]
    Truncated(usize),
    #[error("la secuencia RLE contiene una longitud cero")]
    ZeroLength,
    #[error("la secuencia RLE produce más píxeles de los esperados")]
    Overflow,
}

pub fn encode_rle(pixels: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter::default();
    let mut index = 0usize;
    while index < pixels.len() {
        let color = pixels[index];
        let mut run = 1usize;
        while index + run < pixels.len() && pixels[index + run] == color && run < MAX_RUN {
            run += 1;
        }
        index += run;

        if color != 0 && run == 1 {
            writer.write_bits(color as u32, 8);
            continue;
        }

        writer.write_bits(0, 8);
        writer.write_bit(color != 0);
        if run <= 63 {
            writer.write_bit(false);
            writer.write_bits(run as u32, 6);
        } else {
            writer.write_bit(true);
            writer.write_bits(run as u32, 14);
        }
        if color != 0 {
            writer.write_bits(color as u32, 8);
        }
    }
    writer.finish()
}

/// Inverse used by unit tests and diagnostics.
pub fn decode_rle(bytes: &[u8], pixel_count: usize) -> Result<Vec<u8>, RleError> {
    let mut reader = BitReader::new(bytes);
    let mut pixels = Vec::with_capacity(pixel_count);
    while pixels.len() < pixel_count {
        let color = reader
            .read_bits(8)
            .ok_or(RleError::Truncated(pixel_count))? as u8;
        if color != 0 {
            pixels.push(color);
            continue;
        }
        let colored = reader.read_bit().ok_or(RleError::Truncated(pixel_count))?;
        let long = reader.read_bit().ok_or(RleError::Truncated(pixel_count))?;
        let len = reader
            .read_bits(if long { 14 } else { 6 })
            .ok_or(RleError::Truncated(pixel_count))? as usize;
        if len == 0 {
            return Err(RleError::ZeroLength);
        }
        let color = if colored {
            reader
                .read_bits(8)
                .ok_or(RleError::Truncated(pixel_count))? as u8
        } else {
            0
        };
        if pixels.len() + len > pixel_count {
            return Err(RleError::Overflow);
        }
        pixels.extend(std::iter::repeat_n(color, len));
    }
    Ok(pixels)
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    current: u8,
    used: u8,
}

impl BitWriter {
    fn write_bit(&mut self, bit: bool) {
        self.current = (self.current << 1) | u8::from(bit);
        self.used += 1;
        if self.used == 8 {
            self.bytes.push(self.current);
            self.current = 0;
            self.used = 0;
        }
    }

    fn write_bits(&mut self, value: u32, count: u8) {
        for shift in (0..count).rev() {
            self.write_bit(((value >> shift) & 1) != 0);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            self.current <<= 8 - self.used;
            self.bytes.push(self.current);
        }
        self.bytes
    }
}

struct BitReader<'a> {
    bytes: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, bit: 0 }
    }

    fn read_bit(&mut self) -> Option<bool> {
        let byte = self.bytes.get(self.bit / 8)?;
        let shift = 7 - (self.bit % 8);
        self.bit += 1;
        Some(((byte >> shift) & 1) != 0)
    }

    fn read_bits(&mut self, count: u8) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_single_pixels_and_short_runs() {
        let pixels = [0, 0, 0, 1, 2, 2, 2, 2, 0, 5, 0, 0];
        let encoded = encode_rle(&pixels);
        assert_eq!(decode_rle(&encoded, pixels.len()), Ok(pixels.to_vec()));
    }

    #[test]
    fn uses_long_runs_for_more_than_63_pixels() {
        let mut pixels = vec![7u8; 200];
        pixels.extend(std::iter::repeat_n(0u8, 100));
        let encoded = encode_rle(&pixels);
        assert!(encoded.len() < pixels.len() / 2);
        assert_eq!(decode_rle(&encoded, pixels.len()), Ok(pixels));
    }

    #[test]
    fn splits_runs_longer_than_14_bits() {
        let pixels = vec![3u8; 20_000];
        let encoded = encode_rle(&pixels);
        assert_eq!(decode_rle(&encoded, pixels.len()), Ok(pixels));
    }

    #[test]
    fn rejects_a_truncated_stream() {
        assert_eq!(decode_rle(&[], 3), Err(RleError::Truncated(3)));
    }
}
