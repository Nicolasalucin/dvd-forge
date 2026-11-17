use crate::cues::SubtitleCue;
use crate::raster::{rasterize_cues, RasterCue, RasterOptions};
use crate::SubtitleError;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SpumuxAssets {
    pub xml: String,
    pub png_paths: Vec<PathBuf>,
}

/// Writes four-color PNGs and a Spumux XML document for DVD subpictures.
/// Spumux owns the MPEG-PS packetization and produces the final SPU stream.
pub fn render_spumux_assets(
    cues: &[SubtitleCue],
    options: &RasterOptions<'_>,
    directory: &Path,
    stem: &str,
) -> Result<SpumuxAssets, SubtitleError> {
    std::fs::create_dir_all(directory)?;
    let raster = rasterize_cues(cues, options)?;
    let format = if options.height == 576 { "PAL" } else { "NTSC" };
    let mut xml = format!("<subpictures format=\"{format}\">\n  <stream>\n");
    let mut png_paths = Vec::new();
    for (index, cue) in raster.iter().enumerate() {
        let path = directory.join(format!("{stem}.{index:04}.png"));
        write_four_color_png(cue, &path)?;
        let escaped = xml_escape(&path.display().to_string());
        writeln!(
            xml,
            "    <spu start=\"{}\" end=\"{}\" image=\"{}\" xoffset=\"{}\" yoffset=\"{}\" force=\"{}\" />",
            timestamp(cue.start_ms),
            timestamp(cue.end_ms),
            escaped,
            cue.x,
            cue.y,
            if cue.force { "yes" } else { "no" }
        )
        .map_err(|error| SubtitleError::Raster(error.to_string()))?;
        png_paths.push(path);
    }
    xml.push_str("  </stream>\n</subpictures>\n");
    Ok(SpumuxAssets { xml, png_paths })
}

fn write_four_color_png(cue: &RasterCue, path: &Path) -> Result<(), SubtitleError> {
    let width = cue.width.max(1);
    let height = cue.height.max(1);
    let mut image = image::RgbaImage::new(width, height);
    let bayer = [
        [0u8, 8, 2, 10],
        [12, 4, 14, 6],
        [3, 11, 1, 9],
        [15, 7, 13, 5],
    ];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let threshold = bayer[(y % 4) as usize][(x % 4) as usize] * 16 + 8;
            let fill = cue.fill[index];
            let outline = cue.outline[index];
            let shadow = cue.shadow[index];
            let pixel = if fill > threshold {
                cue.fill_color
            } else if outline > threshold {
                cue.outline_color
            } else if shadow > threshold {
                cue.shadow_color
            } else {
                [0, 0, 0, 0]
            };
            image.put_pixel(x, y, image::Rgba(pixel));
        }
    }
    image
        .save(path)
        .map_err(|error| SubtitleError::Raster(format!("PNG {}: {error}", path.display())))
}

fn timestamp(milliseconds: u64) -> String {
    let hours = milliseconds / 3_600_000;
    let minutes = (milliseconds / 60_000) % 60;
    let seconds = (milliseconds / 1_000) % 60;
    let millis = milliseconds % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
