//! Per-episode picture fixes ("Fix the video"): rotation, mirror, crop,
//! brightness/contrast/saturation, noise reduction, deinterlacing and a
//! blurred background for vertical (phone) videos. All neutral by default,
//! so projects saved before 1.1 open unchanged and encode exactly as before.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoFilters {
    #[serde(default)]
    pub rotate: Rotation,
    #[serde(default)]
    pub flip_horizontal: bool,
    /// Pixels removed from each side of the source (before rotation).
    #[serde(default)]
    pub crop: Option<CropMargins>,
    /// -100..100 (0 = unchanged).
    #[serde(default)]
    pub brightness: i8,
    /// -100..100 (0 = unchanged).
    #[serde(default)]
    pub contrast: i8,
    /// -100..100 (0 = unchanged; -100 = black and white).
    #[serde(default)]
    pub saturation: i8,
    #[serde(default)]
    pub denoise: Denoise,
    #[serde(default)]
    pub deinterlace: Deinterlace,
    /// What fills the sides of a video narrower than the screen.
    #[serde(default)]
    pub side_fill: SideFill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Rotation {
    #[default]
    None,
    Clockwise,
    Half,
    CounterClockwise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CropMargins {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Denoise {
    #[default]
    Off,
    Light,
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Deinterlace {
    /// Only when an interlaced source goes to a progressive disc.
    #[default]
    Auto,
    /// Always (combing visible even on an interlaced disc).
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SideFill {
    /// Black bars (the classic letterbox/pillarbox).
    #[default]
    Bars,
    /// The same video, enlarged and blurred, behind it (phone videos).
    Blur,
}

impl VideoFilters {
    /// True when the filters change nothing (the 1.0 behaviour). `Auto`
    /// deinterlacing counts as neutral: 1.0 already kept interlaced sources
    /// interlaced on interlaced discs.
    pub fn is_neutral(&self) -> bool {
        self.rotate == Rotation::None
            && !self.flip_horizontal
            && self.crop.is_none_or(|crop| crop == CropMargins::default())
            && self.brightness == 0
            && self.contrast == 0
            && self.saturation == 0
            && self.denoise == Denoise::Off
            && self.deinterlace == Deinterlace::Auto
            && self.side_fill == SideFill::Bars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_projects_have_neutral_filters() {
        let filters: VideoFilters = serde_json::from_str("{}").expect("empty");
        assert!(filters.is_neutral());
        let rotated: VideoFilters =
            serde_json::from_str(r#"{"rotate":"clockwise"}"#).expect("rotate");
        assert!(!rotated.is_neutral());
    }
}
