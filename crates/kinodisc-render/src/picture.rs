//! FFmpeg filters for the per-episode picture fixes (`VideoFilters`). One
//! builder for every encoder (DVD MPEG-2, AVCHD/Blu-ray H.264) and for the
//! editor's preview frame, so what the user sees is what gets burned. The
//! chain goes *before* each encoder's own scale/pad chain:
//!
//! deinterlace → crop → rotate/mirror → denoise → colour → blurred sides

use kinodisc_domain::{
    Deinterlace, Denoise, MediaProbe, Rotation, SideFill, VideoFilters, VideoScaleMode,
};

/// What the encoder will produce.
#[derive(Debug, Clone, Copy)]
pub struct PictureTarget {
    /// Display aspect of the disc picture (16/9 or 4/3).
    pub display_aspect: f64,
    /// The disc picture is progressive (DVD keeps interlaced sources
    /// interlaced; 1080p/720p Blu-ray is progressive).
    pub progressive: bool,
    pub scale_mode: VideoScaleMode,
}

/// Source geometry from the probe.
#[derive(Debug, Clone, Copy)]
pub struct PictureSource {
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio.
    pub sar: f64,
    pub interlaced: bool,
}

impl PictureSource {
    pub fn from_probe(probe: &MediaProbe) -> Option<Self> {
        let video = probe.video.as_ref()?;
        let sar = video
            .sample_aspect_ratio
            .as_deref()
            .and_then(parse_ratio)
            .filter(|value| *value > 0.0)
            .unwrap_or(1.0);
        Some(Self {
            width: video.width,
            height: video.height,
            sar,
            // Same rule as the encoders (`is_interlaced` in lib.rs).
            interlaced: video
                .field_order
                .as_deref()
                .is_some_and(|order| order != "progressive"),
        })
    }
}

fn parse_ratio(value: &str) -> Option<f64> {
    let (num, den) = value.split_once(':').or_else(|| value.split_once('/'))?;
    let (num, den) = (
        num.trim().parse::<f64>().ok()?,
        den.trim().parse::<f64>().ok()?,
    );
    (den > 0.0).then_some(num / den)
}

/// Whether the encoded picture stays interlaced: forced deinterlacing makes
/// it progressive.
pub fn keeps_interlacing(filters: &VideoFilters, source_interlaced: bool) -> bool {
    source_interlaced && filters.deinterlace != Deinterlace::On
}

/// The filter chain for `filters`, or `None` when nothing needs to run.
pub fn picture_filter(
    filters: &VideoFilters,
    source: Option<PictureSource>,
    target: PictureTarget,
) -> Option<String> {
    let mut chain: Vec<String> = Vec::new();
    let interlaced = source.is_some_and(|source| source.interlaced);
    match filters.deinterlace {
        Deinterlace::On => chain.push("bwdif=mode=send_frame:deint=all".to_string()),
        Deinterlace::Auto if interlaced && target.progressive => {
            chain.push("bwdif=mode=send_frame:deint=interlaced".to_string())
        }
        _ => {}
    }
    // Displayed size of the source after crop and rotation (for side fill).
    let mut shown = source.map(|source| (source.width as f64 * source.sar, source.height as f64));
    if let Some(crop) = filters.crop.filter(|crop| *crop != Default::default()) {
        let even = |value: u32| value - value % 2;
        let (left, top) = (even(crop.left), even(crop.top));
        let horizontal = even(crop.left + crop.right);
        let vertical = even(crop.top + crop.bottom);
        chain.push(format!("crop=iw-{horizontal}:ih-{vertical}:{left}:{top}"));
        if let (Some((width, height)), Some(source)) = (shown.as_mut(), source) {
            *width -= f64::from(horizontal) * source.sar;
            *height -= f64::from(vertical);
        }
    }
    match filters.rotate {
        Rotation::None => {}
        Rotation::Clockwise => chain.push("transpose=clock".to_string()),
        Rotation::CounterClockwise => chain.push("transpose=cclock".to_string()),
        Rotation::Half => chain.push("hflip,vflip".to_string()),
    }
    if matches!(
        filters.rotate,
        Rotation::Clockwise | Rotation::CounterClockwise
    ) {
        if let Some((width, height)) = shown.as_mut() {
            std::mem::swap(width, height);
        }
    }
    if filters.flip_horizontal {
        chain.push("hflip".to_string());
    }
    match filters.denoise {
        Denoise::Off => {}
        Denoise::Light => chain.push("hqdn3d=2:1.5:3:2.25".to_string()),
        Denoise::Strong => chain.push("hqdn3d=5:4:8:6".to_string()),
    }
    if filters.brightness != 0 || filters.contrast != 0 || filters.saturation != 0 {
        let brightness = f64::from(filters.brightness.clamp(-100, 100)) / 100.0 * 0.25;
        let contrast = 1.0 + f64::from(filters.contrast.clamp(-100, 100)) / 100.0 * 0.6;
        let saturation = 1.0 + f64::from(filters.saturation.clamp(-100, 100)) / 100.0;
        chain.push(format!(
            "eq=brightness={brightness:.3}:contrast={contrast:.3}:saturation={saturation:.3}"
        ));
    }
    let letterbox = matches!(
        target.scale_mode,
        VideoScaleMode::Letterbox | VideoScaleMode::Contain
    );
    let narrower = shown.is_some_and(|(width, height)| {
        height > 0.0 && width / height < target.display_aspect * 0.97
    });
    if filters.side_fill == SideFill::Blur && letterbox && narrower {
        // A display-shaped canvas: the encoder's own scale/pad then has
        // nothing left to pad.
        let (canvas_w, canvas_h) = if target.display_aspect > 1.5 {
            (1280, 720)
        } else {
            (960, 720)
        };
        chain.push(format!(
            "scale=iw*sar:ih,setsar=1,split=2[kdbg][kdfg];\
[kdbg]scale={canvas_w}:{canvas_h}:force_original_aspect_ratio=increase,crop={canvas_w}:{canvas_h},boxblur=24:2,eq=brightness=-0.06[kdbgb];\
[kdfg]scale={canvas_w}:{canvas_h}:force_original_aspect_ratio=decrease[kdfgs];\
[kdbgb][kdfgs]overlay=(W-w)/2:(H-h)/2"
        ));
    }
    (!chain.is_empty()).then(|| chain.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kinodisc_domain::CropMargins;

    fn target(progressive: bool) -> PictureTarget {
        PictureTarget {
            display_aspect: 16.0 / 9.0,
            progressive,
            scale_mode: VideoScaleMode::Letterbox,
        }
    }

    fn source(width: u32, height: u32, interlaced: bool) -> Option<PictureSource> {
        Some(PictureSource {
            width,
            height,
            sar: 1.0,
            interlaced,
        })
    }

    #[test]
    fn neutral_filters_add_nothing() {
        let filters = VideoFilters::default();
        assert_eq!(
            picture_filter(&filters, source(1920, 1080, false), target(true)),
            None
        );
        // An interlaced source on an interlaced disc stays interlaced.
        assert_eq!(
            picture_filter(&filters, source(1920, 1080, true), target(false)),
            None
        );
    }

    #[test]
    fn interlaced_sources_are_deinterlaced_for_progressive_discs() {
        let filters = VideoFilters::default();
        let chain =
            picture_filter(&filters, source(1920, 1080, true), target(true)).expect("chain");
        assert!(chain.starts_with("bwdif"));
        assert!(keeps_interlacing(&filters, true));
        let forced = VideoFilters {
            deinterlace: Deinterlace::On,
            ..Default::default()
        };
        assert!(!keeps_interlacing(&forced, true));
    }

    #[test]
    fn filters_run_in_order() {
        let filters = VideoFilters {
            rotate: Rotation::Clockwise,
            flip_horizontal: true,
            crop: Some(CropMargins {
                left: 9,
                top: 0,
                right: 11,
                bottom: 140,
            }),
            brightness: 20,
            saturation: -100,
            denoise: Denoise::Light,
            ..Default::default()
        };
        let chain =
            picture_filter(&filters, source(1920, 1080, false), target(true)).expect("chain");
        let order = [
            "crop=iw-20:ih-140:8:0",
            "transpose=clock",
            "hflip",
            "hqdn3d",
            "eq=brightness=0.050",
        ];
        let mut last = 0;
        for part in order {
            let at = chain
                .find(part)
                .unwrap_or_else(|| panic!("{part} missing in {chain}"));
            assert!(at >= last, "{part} out of order in {chain}");
            last = at;
        }
        assert!(chain.contains("saturation=0.000"));
    }

    #[test]
    fn blurred_sides_only_for_narrow_pictures() {
        let blur = VideoFilters {
            side_fill: SideFill::Blur,
            ..Default::default()
        };
        let phone = picture_filter(&blur, source(1080, 1920, false), target(true)).expect("phone");
        assert!(phone.contains("boxblur") && phone.contains("overlay"));
        // A widescreen video fills the screen: nothing to blur.
        assert_eq!(
            picture_filter(&blur, source(1920, 1080, false), target(true)),
            None
        );
        // A landscape video rotated to portrait needs it.
        let rotated = VideoFilters {
            rotate: Rotation::Clockwise,
            ..blur
        };
        assert!(
            picture_filter(&rotated, source(1920, 1080, false), target(true))
                .expect("rotated")
                .contains("boxblur")
        );
        // Crop/fill modes never show sides.
        let fill = PictureTarget {
            scale_mode: VideoScaleMode::CropFill,
            ..target(true)
        };
        assert_eq!(picture_filter(&blur, source(1080, 1920, false), fill), None);
    }
}
