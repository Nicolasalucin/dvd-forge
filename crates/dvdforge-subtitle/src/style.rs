use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubtitleAlignment {
    #[default]
    BottomCenter,
    BottomLeft,
    BottomRight,
    TopCenter,
    Center,
}

#[derive(Debug, Clone)]
pub struct SubtitleStyle {
    pub font_family: String,
    pub font_path: Option<PathBuf>,
    pub font_weight: u16,
    pub italic: bool,
    /// Font size as a percentage of the target height.
    pub font_size_percent: f32,
    pub outline_percent: f32,
    pub shadow_offset_percent: f32,
    pub fill_color: [u8; 4],
    pub outline_color: [u8; 4],
    pub shadow_color: [u8; 4],
    pub alignment: SubtitleAlignment,
    pub safe_area_percent: u8,
    pub max_lines: u8,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_family: "sans-serif".to_string(),
            font_path: None,
            font_weight: 400,
            italic: false,
            font_size_percent: 4.5,
            outline_percent: 0.18,
            shadow_offset_percent: 0.12,
            fill_color: [255, 255, 255, 255],
            outline_color: [0, 0, 0, 255],
            shadow_color: [0, 0, 0, 160],
            alignment: SubtitleAlignment::BottomCenter,
            safe_area_percent: 5,
            max_lines: 3,
        }
    }
}

/// Parses `#RRGGBB`, `#RRGGBBAA` or ASS `&HAABBGGRR`/`&HBBGGRR`.
pub fn parse_color(value: &str) -> Option<[u8; 4]> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let digits: Vec<u8> = hex
            .chars()
            .map(|ch| ch.to_digit(16).map(|digit| digit as u8))
            .collect::<Option<Vec<_>>>()?;
        return match digits.len() {
            6 => Some([
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                255,
            ]),
            8 => Some([
                digits[0] * 16 + digits[1],
                digits[2] * 16 + digits[3],
                digits[4] * 16 + digits[5],
                digits[6] * 16 + digits[7],
            ]),
            _ => None,
        };
    }
    let hex = value
        .strip_prefix("&H")
        .or_else(|| value.strip_prefix("&h"))
        .unwrap_or(value);
    if hex.len() == 6 {
        let number = u32::from_str_radix(hex, 16).ok()?;
        return Some([
            (number & 0xFF) as u8,
            ((number >> 8) & 0xFF) as u8,
            ((number >> 16) & 0xFF) as u8,
            255,
        ]);
    }
    if hex.len() == 8 {
        let number = u32::from_str_radix(hex, 16).ok()?;
        // ASS alpha is inverted: 0x00 is fully opaque.
        let alpha = 255 - ((number >> 24) & 0xFF) as u8;
        return Some([
            (number & 0xFF) as u8,
            ((number >> 8) & 0xFF) as u8,
            ((number >> 16) & 0xFF) as u8,
            alpha,
        ]);
    }
    None
}
