//! Native subtitle parsing, layout, rasterization and Blu-ray/DVD encoding.
//!
//! This crate is independent from FFmpeg for authoring: text is parsed and
//! rasterized in Rust, then emitted as HDMV PGS display sets or as four-color
//! PNG assets for Spumux.

pub mod cues;
pub mod fonts;
pub mod pgs;
pub mod raster;
pub mod spumux;
pub mod style;

pub use cues::{CueStyle, ParsedSubtitles, SubtitleCue, SubtitleFormat, SubtitleWarning};
pub use fonts::{FontProvider, ResolvedFont};
pub use pgs::render_pgs;
pub use raster::{rasterize_cues, RasterCue, RasterOptions};
pub use spumux::{render_spumux_assets, SpumuxAssets};
pub use style::{parse_color, SubtitleAlignment, SubtitleStyle};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SubtitleError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("formato de subtítulo no soportado: {0}")]
    UnsupportedFormat(String),
    #[error("subtítulo inválido: {0}")]
    Invalid(String),
    #[error("no se pudo cargar la fuente: {0}")]
    Font(String),
    #[error("no se pudo rasterizar el subtítulo: {0}")]
    Raster(String),
}

pub fn parse_path(path: &std::path::Path) -> Result<ParsedSubtitles, SubtitleError> {
    let format = SubtitleFormat::from_path(path)
        .ok_or_else(|| SubtitleError::UnsupportedFormat(path.display().to_string()))?;
    let text = std::fs::read_to_string(path)?;
    cues::parse_text(format, &text)
}
