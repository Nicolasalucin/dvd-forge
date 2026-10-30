use crate::cues::SubtitleCue;
use crate::raster::{rasterize_cues, RasterCue, RasterOptions};
use crate::SubtitleError;
use dvdforge_hdmv::rle::encode_rle_lines;

const PCS: u8 = 0x16;
const WDS: u8 = 0x17;
const PDS: u8 = 0x14;
const ODS: u8 = 0x15;
const END: u8 = 0x80;

/// Renders cues into a raw PGS `.sup` stream (13-byte PG segment headers).
pub fn render_pgs(
    cues: &[SubtitleCue],
    options: &RasterOptions<'_>,
) -> Result<Vec<u8>, SubtitleError> {
    let raster = rasterize_cues(cues, options)?;
    let mut output = Vec::new();
    let mut composition = 0u16;
    let mut first = true;
    for (index, cue) in raster.iter().enumerate() {
        let pts = u32::try_from(cue.start_ms.saturating_mul(90))
            .map_err(|_| SubtitleError::Raster("PTS fuera de rango".to_string()))?;
        write_display_set(
            &mut output,
            cue,
            options.width,
            options.height,
            pts,
            composition,
            first,
        )?;
        composition = composition.wrapping_add(1);
        first = false;
        let next_start = raster
            .get(index + 1)
            .map(|next| next.start_ms)
            .unwrap_or(u64::MAX);
        if cue.end_ms <= next_start {
            let clear_pts = u32::try_from(cue.end_ms.saturating_mul(90))
                .map_err(|_| SubtitleError::Raster("PTS fuera de rango".to_string()))?;
            write_clear_set(
                &mut output,
                options.width,
                options.height,
                clear_pts,
                composition,
            )?;
            composition = composition.wrapping_add(1);
        }
    }
    Ok(output)
}

fn write_display_set(
    output: &mut Vec<u8>,
    cue: &RasterCue,
    video_width: u32,
    video_height: u32,
    pts: u32,
    composition: u16,
    epoch_start: bool,
) -> Result<(), SubtitleError> {
    let palette = palette_for(cue);
    push_segment(
        output,
        PCS,
        pts,
        &build_pcs(video_width, video_height, composition, cue, epoch_start)?,
    )?;
    push_segment(output, WDS, pts, &build_wds(cue))?;
    push_segment(output, PDS, pts, &build_pds(&palette))?;
    push_segment(output, ODS, pts, &build_ods(cue)?)?;
    push_segment(output, END, pts, &[])?;
    Ok(())
}

fn write_clear_set(
    output: &mut Vec<u8>,
    video_width: u32,
    video_height: u32,
    pts: u32,
    composition: u16,
) -> Result<(), SubtitleError> {
    let mut payload = Vec::with_capacity(11);
    payload.extend_from_slice(&(video_width as u16).to_be_bytes());
    payload.extend_from_slice(&(video_height as u16).to_be_bytes());
    payload.push(0x10);
    payload.extend_from_slice(&composition.to_be_bytes());
    payload.push(0x00);
    payload.push(0);
    payload.push(0);
    payload.push(0);
    push_segment(output, PCS, pts, &payload)?;
    push_segment(output, END, pts, &[])?;
    Ok(())
}

fn build_pcs(
    video_width: u32,
    video_height: u32,
    composition: u16,
    cue: &RasterCue,
    epoch_start: bool,
) -> Result<Vec<u8>, SubtitleError> {
    let mut payload = Vec::with_capacity(11 + 8);
    payload.extend_from_slice(&(video_width as u16).to_be_bytes());
    payload.extend_from_slice(&(video_height as u16).to_be_bytes());
    payload.push(0x10);
    payload.extend_from_slice(&composition.to_be_bytes());
    payload.push(if epoch_start { 0x80 } else { 0x00 });
    payload.push(0);
    payload.push(0);
    payload.push(1);
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.push(0);
    payload.push(0);
    payload.extend_from_slice(&(cue.x as u16).to_be_bytes());
    payload.extend_from_slice(&(cue.y as u16).to_be_bytes());
    Ok(payload)
}

fn build_wds(cue: &RasterCue) -> Vec<u8> {
    let mut payload = Vec::with_capacity(10);
    payload.push(1);
    payload.push(0);
    payload.extend_from_slice(&(cue.x as u16).to_be_bytes());
    payload.extend_from_slice(&(cue.y as u16).to_be_bytes());
    payload.extend_from_slice(&(cue.width as u16).to_be_bytes());
    payload.extend_from_slice(&(cue.height as u16).to_be_bytes());
    payload
}

fn build_pds(palette: &[[u8; 5]]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + palette.len() * 5);
    payload.push(0);
    payload.push(0);
    for (index, entry) in palette.iter().enumerate() {
        payload.push(index as u8);
        payload.extend_from_slice(&entry[1..]);
    }
    payload
}

fn build_ods(cue: &RasterCue) -> Result<Vec<u8>, SubtitleError> {
    let pixels = cue_indices(cue);
    let rle = encode_rle_lines(&pixels, cue.width as usize);
    let data_length = 4usize
        .checked_add(rle.len())
        .ok_or_else(|| SubtitleError::Raster("ODS demasiado grande".to_string()))?;
    if data_length > 0xFF_FFFF {
        return Err(SubtitleError::Raster("ODS supera 24 bits".to_string()));
    }
    let mut payload = Vec::with_capacity(11 + rle.len());
    payload.extend_from_slice(&1u16.to_be_bytes());
    payload.push(0);
    payload.push(0xC0);
    payload.extend_from_slice(&(data_length as u32).to_be_bytes()[1..]);
    payload.extend_from_slice(&(cue.width as u16).to_be_bytes());
    payload.extend_from_slice(&(cue.height as u16).to_be_bytes());
    payload.extend_from_slice(&rle);
    Ok(payload)
}

fn cue_indices(cue: &RasterCue) -> Vec<u8> {
    let count = (cue.width * cue.height) as usize;
    let mut pixels = vec![0u8; count];
    for (index, pixel) in pixels.iter_mut().enumerate() {
        let fill = cue.fill[index];
        let outline = cue.outline[index];
        let shadow = cue.shadow[index];
        *pixel = if fill > 0 {
            1 + ((u16::from(fill) * 15 / 255) as u8).min(14)
        } else if outline > 0 {
            16 + ((u16::from(outline) * 8 / 255) as u8).min(7)
        } else if shadow > 0 {
            24 + ((u16::from(shadow) * 4 / 255) as u8).min(3)
        } else {
            0
        };
    }
    pixels
}

fn palette_for(cue: &RasterCue) -> Vec<[u8; 5]> {
    let mut palette = Vec::with_capacity(28);
    palette.push([0, 16, 128, 128, 0]);
    for level in 0..15u8 {
        palette.push(color_entry(
            cue.fill_color,
            ((level as u16 + 1) * 255 / 15) as u8,
        ));
    }
    for level in 0..8u8 {
        palette.push(color_entry(
            cue.outline_color,
            ((level as u16 + 1) * 255 / 8) as u8,
        ));
    }
    for level in 0..4u8 {
        palette.push(color_entry(
            cue.shadow_color,
            ((level as u16 + 1) * 255 / 4) as u8,
        ));
    }
    palette
}

fn color_entry(color: [u8; 4], alpha: u8) -> [u8; 5] {
    let r = f32::from(color[0]);
    let g = f32::from(color[1]);
    let b = f32::from(color[2]);
    let y = (0.299 * r + 0.587 * g + 0.114 * b)
        .round()
        .clamp(16.0, 235.0) as u8;
    let cb = (-0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0)
        .round()
        .clamp(16.0, 240.0) as u8;
    let cr = (0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0)
        .round()
        .clamp(16.0, 240.0) as u8;
    [1, y, cr, cb, alpha]
}

fn push_segment(
    output: &mut Vec<u8>,
    kind: u8,
    pts: u32,
    payload: &[u8],
) -> Result<(), SubtitleError> {
    let length = u16::try_from(payload.len())
        .map_err(|_| SubtitleError::Raster("segmento PGS demasiado grande".to_string()))?;
    output.extend_from_slice(b"PG");
    output.extend_from_slice(&pts.to_be_bytes());
    output.extend_from_slice(&pts.to_be_bytes());
    output.push(kind);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(payload);
    Ok(())
}
