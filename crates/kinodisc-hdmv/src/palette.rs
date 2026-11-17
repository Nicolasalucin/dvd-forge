//! Palette conversion from RGBA to the Y/Cr/Cb/alpha entries used by the
//! Palette Definition Segment.

use std::collections::HashMap;
use thiserror::Error;

pub const MAX_PALETTE_ENTRIES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub index: u8,
    pub y: u8,
    pub cr: u8,
    pub cb: u8,
    pub alpha: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub entries: Vec<PaletteEntry>,
    lookup: HashMap<[u8; 4], u8>,
    quantized: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PaletteError {
    #[error("la paleta no contiene colores")]
    Empty,
    #[error("la paleta supera las 256 entradas")]
    TooManyColors,
}

impl Palette {
    /// Builds a palette from RGBA pixels. If there are more than 255 distinct
    /// colors the image is reduced to a 3/3/1-bit RGB cube plus transparency,
    /// which keeps the palette under the 256-entry limit.
    pub fn from_pixels<'a>(
        images: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<Self, PaletteError> {
        let mut unique = Vec::<[u8; 4]>::new();
        let mut seen = HashMap::<[u8; 4], usize>::new();
        for pixels in images {
            if !pixels.len().is_multiple_of(4) {
                continue;
            }
            for chunk in pixels.as_chunks::<4>().0 {
                let rgba = [chunk[0], chunk[1], chunk[2], chunk[3]];
                if seen.contains_key(&rgba) {
                    continue;
                }
                seen.insert(rgba, unique.len());
                unique.push(rgba);
            }
        }
        if unique.is_empty() {
            return Err(PaletteError::Empty);
        }

        let quantize = unique.len() > MAX_PALETTE_ENTRIES - 1;
        let mut colors: Vec<[u8; 4]> = if quantize {
            unique.iter().copied().map(quantize_rgba).collect()
        } else {
            unique
        };
        colors.sort_unstable_by_key(|rgba| (rgba[3] == 0, *rgba));
        colors.dedup();

        let mut entries = Vec::with_capacity(colors.len() + 1);
        let mut lookup = HashMap::with_capacity(colors.len() + 1);
        // Index 0 is always the transparent entry used by RLE zero runs.
        entries.push(PaletteEntry {
            index: 0,
            y: 0,
            cr: 0,
            cb: 0,
            alpha: 0,
        });
        lookup.insert([0, 0, 0, 0], 0);
        for rgba in colors {
            if rgba == [0, 0, 0, 0] {
                continue;
            }
            if entries.len() >= MAX_PALETTE_ENTRIES {
                return Err(PaletteError::TooManyColors);
            }
            let index = entries.len() as u8;
            let [y, cr, cb, alpha] = rgba_to_ycbcr(rgba);
            entries.push(PaletteEntry {
                index,
                y,
                cr,
                cb,
                alpha,
            });
            lookup.insert(rgba, index);
        }
        Ok(Self {
            entries,
            lookup,
            quantized: quantize,
        })
    }

    pub fn index_pixels(&self, pixels: &[u8]) -> Vec<u8> {
        let mut indexed = Vec::with_capacity(pixels.len() / 4);
        for chunk in pixels.as_chunks::<4>().0 {
            let rgba = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let key = if self.quantized {
                quantize_rgba(rgba)
            } else {
                rgba
            };
            indexed.push(
                self.lookup
                    .get(&key)
                    .copied()
                    .or_else(|| self.lookup.get(&[0, 0, 0, 0]).copied())
                    .unwrap_or(0),
            );
        }
        indexed
    }

    pub fn is_quantized(&self) -> bool {
        self.quantized
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn rgba_to_ycbcr([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    let r = r as f32;
    let g = g as f32;
    let b = b as f32;
    let y = 16.0 + 0.257 * r + 0.504 * g + 0.098 * b;
    let cb = 128.0 - 0.148 * r - 0.291 * g + 0.439 * b;
    let cr = 128.0 + 0.439 * r - 0.368 * g - 0.071 * b;
    [
        y.round().clamp(16.0, 235.0) as u8,
        cr.round().clamp(16.0, 240.0) as u8,
        cb.round().clamp(16.0, 240.0) as u8,
        a,
    ]
}

fn quantize_rgba([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    // Near-transparent pixels share index 0.
    if a < 16 {
        return [0, 0, 0, 0];
    }
    // Opaque pixels: 6x6x6 cube (216 entries); 3/3/1 bits left blue with two levels.
    if a >= 240 {
        return [cube6(r), cube6(g), cube6(b), 255];
    }
    // Translucent pixels (button highlights, anti-aliased edges) keep four
    // alpha levels with 1-bit RGB: 32 entries. 216 + 32 + index 0 <= 256.
    let bit = |value: u8| if value >= 128 { 255 } else { 0 };
    [bit(r), bit(g), bit(b), (a & 0xC0) | 0x20]
}

/// Nearest of 0, 51, 102, 153, 204, 255.
fn cube6(value: u8) -> u8 {
    ((u16::from(value) + 25) / 51 * 51) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_transparent_index_and_converts_to_ycbcr() {
        let black = [0, 0, 0, 0];
        let white = [255, 255, 255, 255];
        let mut pixels = Vec::new();
        pixels.extend_from_slice(&black);
        pixels.extend_from_slice(&white);
        let palette = Palette::from_pixels([pixels.as_slice()]).expect("palette");
        assert_eq!(palette.entries[0].alpha, 0);
        assert_eq!(palette.index_pixels(&pixels), vec![0, 1]);
        assert!(palette.entries[1].y > 200);
    }

    #[test]
    fn quantizes_images_with_more_than_255_colors() {
        let mut pixels = Vec::new();
        for index in 0..300u32 {
            pixels.extend_from_slice(&[
                (index * 7) as u8,
                (index * 13) as u8,
                (index * 29) as u8,
                255,
            ]);
        }
        let palette = Palette::from_pixels([pixels.as_slice()]).expect("palette");
        assert!(palette.is_quantized());
        assert!(palette.len() <= MAX_PALETTE_ENTRIES);
    }

    #[test]
    fn quantization_keeps_translucent_highlights_visible() {
        let mut pixels = Vec::new();
        for index in 0..300u32 {
            pixels.extend_from_slice(&[
                (index * 7) as u8,
                (index * 13) as u8,
                (index * 29) as u8,
                255,
            ]);
        }
        let highlight = [255u8, 255, 255, 76];
        pixels.extend_from_slice(&highlight);
        let palette = Palette::from_pixels([pixels.as_slice()]).expect("palette");
        assert!(palette.is_quantized());
        let index = palette.index_pixels(&highlight)[0];
        assert_ne!(index, 0);
        assert!(palette.entries[usize::from(index)].alpha > 0);
    }
}
