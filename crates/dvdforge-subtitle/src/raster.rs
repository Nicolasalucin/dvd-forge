use crate::cues::{CueStyle, SubtitleCue};
use crate::fonts::{FontProvider, ResolvedFont};
use crate::style::{SubtitleAlignment, SubtitleStyle};
use crate::SubtitleError;
use ab_glyph::{Font, PxScale, ScaleFont};

#[derive(Debug, Clone)]
pub struct RasterOptions<'a> {
    pub width: u32,
    pub height: u32,
    pub style: &'a SubtitleStyle,
}

#[derive(Debug, Clone)]
pub struct RasterCue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub fill: Vec<u8>,
    pub outline: Vec<u8>,
    pub shadow: Vec<u8>,
    pub force: bool,
    pub fill_color: [u8; 4],
    pub outline_color: [u8; 4],
    pub shadow_color: [u8; 4],
}

pub fn rasterize_cues(
    cues: &[SubtitleCue],
    options: &RasterOptions<'_>,
) -> Result<Vec<RasterCue>, SubtitleError> {
    let provider = FontProvider::new();
    let resolved = provider.resolve(options.style)?;
    let mut output = Vec::new();
    for cue in cues {
        if let Some(raster) = rasterize_cue(cue, options, &resolved)? {
            output.push(raster);
        }
    }
    Ok(output)
}

fn rasterize_cue(
    cue: &SubtitleCue,
    options: &RasterOptions<'_>,
    resolved: &ResolvedFont,
) -> Result<Option<RasterCue>, SubtitleError> {
    let width = options.width.max(1) as usize;
    let height = options.height.max(1) as usize;
    let font_size = cue
        .style
        .font_size_px
        .unwrap_or(options.height as f32 * options.style.font_size_percent / 100.0)
        .max(8.0);
    let scale = PxScale::from(font_size);
    let scaled = resolved.font.as_scaled(scale);
    let line_gap = scaled.line_gap().max(font_size * 0.15);
    let line_height = scaled.ascent() - scaled.descent() + line_gap;
    let safe_x = (options.width as f32 * options.style.safe_area_percent as f32 / 100.0).max(4.0);
    let safe_y = (options.height as f32 * options.style.safe_area_percent as f32 / 100.0).max(4.0);
    let max_width = (options.width as f32 - safe_x * 2.0).max(16.0);
    let max_lines = options.style.max_lines.max(1) as usize;

    let lines = wrap_text(&resolved.font, scale, &cue.text, max_width, max_lines);
    if lines.is_empty() {
        return Ok(None);
    }
    let total_height = line_height * lines.len() as f32;
    let alignment = cue.style.alignment.unwrap_or(options.style.alignment);
    let block_top = match alignment {
        SubtitleAlignment::TopCenter => safe_y,
        SubtitleAlignment::Center => ((options.height as f32 - total_height) / 2.0).max(safe_y),
        _ => (options.height as f32 - safe_y - total_height).max(safe_y),
    };

    let mut fill = vec![0u8; width * height];
    for (line_index, line) in lines.iter().enumerate() {
        let line_width = measure_line(&resolved.font, scale, line);
        let line_x = match alignment {
            SubtitleAlignment::BottomLeft => safe_x,
            SubtitleAlignment::BottomRight => options.width as f32 - safe_x - line_width,
            _ => (options.width as f32 - line_width) / 2.0,
        }
        .max(safe_x);
        let baseline = block_top + scaled.ascent() + line_index as f32 * line_height;
        draw_line(
            &resolved.font,
            scale,
            line,
            line_x,
            baseline,
            width,
            height,
            &mut fill,
        );
    }

    let outline_radius = (options.height as f32 * options.style.outline_percent / 100.0)
        .round()
        .clamp(0.0, 12.0) as usize;
    let outline = dilate(&fill, width, height, outline_radius);
    let shadow_offset = (options.height as f32 * options.style.shadow_offset_percent / 100.0)
        .round()
        .clamp(0.0, 16.0) as isize;
    let shadow = offset_mask(&outline, width, height, shadow_offset, shadow_offset);

    let (min_x, min_y, max_x, max_y) = bounds(&fill, &outline, &shadow, width, height);
    if max_x <= min_x || max_y <= min_y {
        return Ok(None);
    }
    let pad = 2usize;
    let min_x = min_x.saturating_sub(pad);
    let min_y = min_y.saturating_sub(pad);
    let max_x = (max_x + pad).min(width);
    let max_y = (max_y + pad).min(height);
    let crop_width = max_x - min_x;
    let crop_height = max_y - min_y;
    let crop = |source: &[u8]| {
        let mut result = vec![0u8; crop_width * crop_height];
        for row in 0..crop_height {
            let source_start = (min_y + row) * width + min_x;
            let target_start = row * crop_width;
            result[target_start..target_start + crop_width]
                .copy_from_slice(&source[source_start..source_start + crop_width]);
        }
        result
    };
    Ok(Some(RasterCue {
        start_ms: cue.start_ms,
        end_ms: cue.end_ms,
        x: min_x as u32,
        y: min_y as u32,
        width: crop_width as u32,
        height: crop_height as u32,
        fill: crop(&fill),
        outline: crop(&outline),
        shadow: crop(&shadow),
        force: cue.forced,
        fill_color: cue.style.fill_color.unwrap_or(options.style.fill_color),
        outline_color: cue
            .style
            .outline_color
            .unwrap_or(options.style.outline_color),
        shadow_color: options.style.shadow_color,
    }))
}

fn wrap_text(
    font: &ab_glyph::FontArc,
    scale: PxScale,
    text: &str,
    max_width: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in text.lines() {
        let paragraph = paragraph.trim();
        if paragraph.is_empty() {
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure_line(font, scale, &candidate) > max_width {
                result.push(std::mem::take(&mut current));
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }
    if result.len() > max_lines {
        result.truncate(max_lines);
    }
    result
}

fn measure_line(font: &ab_glyph::FontArc, scale: PxScale, line: &str) -> f32 {
    let scaled = font.as_scaled(scale);
    line.chars()
        .map(|ch| scaled.h_advance(font.glyph_id(ch)))
        .sum()
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    font: &ab_glyph::FontArc,
    scale: PxScale,
    line: &str,
    mut x: f32,
    baseline: f32,
    width: usize,
    height: usize,
    output: &mut [u8],
) {
    let scaled = font.as_scaled(scale);
    for ch in line.chars() {
        let glyph_id = font.glyph_id(ch);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(x, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as isize + gx as isize;
                let py = bounds.min.y as isize + gy as isize;
                if px < 0 || py < 0 || px >= width as isize || py >= height as isize {
                    return;
                }
                let index = py as usize * width + px as usize;
                let value = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                output[index] = output[index].max(value);
            });
        }
        x += scaled.h_advance(glyph_id);
    }
}

fn dilate(source: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 {
        return source.to_vec();
    }
    let mut horizontal = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius + 1).min(width);
            let mut value = 0;
            for candidate in start..end {
                value = value.max(source[y * width + candidate]);
            }
            horizontal[y * width + x] = value;
        }
    }
    let mut output = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let start = y.saturating_sub(radius);
            let end = (y + radius + 1).min(height);
            let mut value = 0;
            for candidate in start..end {
                value = value.max(horizontal[candidate * width + x]);
            }
            output[y * width + x] = value;
        }
    }
    output
}

fn offset_mask(source: &[u8], width: usize, height: usize, dx: isize, dy: isize) -> Vec<u8> {
    let mut output = vec![0u8; source.len()];
    for y in 0..height {
        for x in 0..width {
            let source_x = x as isize - dx;
            let source_y = y as isize - dy;
            if source_x < 0
                || source_y < 0
                || source_x >= width as isize
                || source_y >= height as isize
            {
                continue;
            }
            output[y * width + x] = source[source_y as usize * width + source_x as usize];
        }
    }
    output
}

fn bounds(
    fill: &[u8],
    outline: &[u8],
    shadow: &[u8],
    width: usize,
    height: usize,
) -> (usize, usize, usize, usize) {
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if fill[index] == 0 && outline[index] == 0 && shadow[index] == 0 {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + 1);
            max_y = max_y.max(y + 1);
        }
    }
    (min_x, min_y, max_x, max_y)
}

#[allow(dead_code)]
fn _style(_: &CueStyle) {}
