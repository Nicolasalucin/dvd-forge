//! Smart Stream Copy: decides whether an episode's video can be put on an
//! AVCHD/Blu-ray disc as-is, without re-encoding.
//!
//! A copied stream is only accepted when every property the disc format and
//! the muxers depend on matches the project exactly. Anything unknown is a
//! "no": a needless re-encode costs time, a wrong copy costs a coaster.

use dvdforge_domain::{
    AvchdVideoConfig, DiscCapacity, Episode, FrameRate, HdScan, MediaProbe, VideoEncodeMode,
};

/// Outcome of the stream-copy check for one episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyDecision {
    pub copy: bool,
    /// Video bitrate of the copied stream (kbps), used by the planner.
    pub video_kbps: Option<u32>,
    /// Stable code: `COPY_OK` or the first failed check.
    pub code: &'static str,
    /// Spanish explanation shown in the UI.
    pub note: String,
}

impl CopyDecision {
    fn no(code: &'static str, note: impl Into<String>) -> Self {
        Self {
            copy: false,
            video_kbps: None,
            code,
            note: note.into(),
        }
    }
}

fn same_rate(a: FrameRate, b: FrameRate) -> bool {
    a.is_valid() && b.is_valid() && (a.as_f64() - b.as_f64()).abs() < 0.01
}

/// Checks an episode against the project's AVCHD/Blu-ray video settings.
pub fn avchd_copy_decision(
    episode: &Episode,
    probe: &MediaProbe,
    config: &AvchdVideoConfig,
    capacity: DiscCapacity,
    max_video_kbps: u32,
) -> CopyDecision {
    if episode.video_mode == VideoEncodeMode::Reencode {
        return CopyDecision::no("COPY_DISABLED", "Recodificación elegida para este episodio.");
    }
    let Some(video) = probe.video.as_ref() else {
        return CopyDecision::no("COPY_NO_VIDEO", "El archivo no tiene vídeo analizable.");
    };
    if !video.codec.eq_ignore_ascii_case("h264") {
        return CopyDecision::no(
            "COPY_CODEC",
            format!("El vídeo es {} y el disco necesita H.264.", video.codec),
        );
    }
    let profile = video.profile.as_deref().unwrap_or("");
    if !matches!(profile, "High" | "Main") {
        return CopyDecision::no(
            "COPY_PROFILE",
            if profile.is_empty() {
                "No se conoce el perfil H.264 (vuelve a importar el archivo).".to_string()
            } else {
                format!("Perfil H.264 «{profile}»; el disco necesita High o Main.")
            },
        );
    }
    let max_level = if capacity.is_blu_ray() || config.is_high_frame_rate_1080p() {
        42
    } else {
        41
    };
    match video.level {
        Some(level) if level <= max_level => {}
        Some(level) => {
            return CopyDecision::no(
                "COPY_LEVEL",
                format!(
                    "Nivel H.264 {:.1}; el máximo es {:.1}.",
                    f64::from(level) / 10.0,
                    f64::from(max_level) / 10.0
                ),
            )
        }
        None => {
            return CopyDecision::no(
                "COPY_LEVEL",
                "No se conoce el nivel H.264 (vuelve a importar el archivo).",
            )
        }
    }
    if video.pix_fmt.as_deref() != Some("yuv420p") {
        return CopyDecision::no(
            "COPY_PIXEL_FORMAT",
            format!(
                "Formato de color {}; el disco necesita yuv420p (8 bits).",
                video.pix_fmt.as_deref().unwrap_or("desconocido")
            ),
        );
    }
    let (width, height) = config.resolution.dimensions();
    if video.width != width || video.height != height {
        return CopyDecision::no(
            "COPY_RESOLUTION",
            format!(
                "El vídeo es {}×{} y el proyecto usa {width}×{height}.",
                video.width, video.height
            ),
        );
    }
    let scan = config.scan();
    let field_rate = FrameRate {
        num: config.frame_rate.num.saturating_mul(2),
        den: config.frame_rate.den,
    };
    let rate_ok = same_rate(video.frame_rate, config.frame_rate)
        || (scan != HdScan::Progressive && same_rate(video.frame_rate, field_rate));
    if !rate_ok {
        return CopyDecision::no(
            "COPY_FRAME_RATE",
            format!(
                "El vídeo va a {:.3} fps y el proyecto a {:.3} fps.",
                video.frame_rate.as_f64(),
                config.frame_rate.as_f64()
            ),
        );
    }
    let field_order = video.field_order.as_deref().unwrap_or("unknown");
    let interlaced_source = matches!(field_order, "tt" | "tb" | "bt" | "bb");
    match scan {
        HdScan::Interlaced if !interlaced_source => {
            return CopyDecision::no(
                "COPY_SCAN",
                "El proyecto es entrelazado y el vídeo no lo es.",
            )
        }
        HdScan::Progressive if interlaced_source => {
            return CopyDecision::no(
                "COPY_SCAN",
                "El proyecto es progresivo y el vídeo es entrelazado.",
            )
        }
        _ => {}
    }
    if episode.trim_start_ms.unwrap_or(0) > 0 || episode.trim.is_some_and(|trim| trim.start > 0) {
        return CopyDecision::no(
            "COPY_TRIM",
            "El episodio está recortado al principio; para copiar sin recodificar no puede haber recorte inicial.",
        );
    }
    let audio_bps: u64 = probe
        .audio
        .iter()
        .filter_map(|audio| audio.bitrate_bps)
        .sum();
    let bitrate_bps = video.bitrate_bps.or_else(|| {
        probe
            .bitrate_bps
            .map(|total| total.saturating_sub(audio_bps))
            .filter(|bps| *bps > 0)
    });
    let Some(bitrate_bps) = bitrate_bps else {
        return CopyDecision::no(
            "COPY_BITRATE",
            "No se conoce el bitrate del vídeo; se recodifica por seguridad.",
        );
    };
    let video_kbps = u32::try_from(bitrate_bps.div_ceil(1_000)).unwrap_or(u32::MAX);
    if video_kbps > max_video_kbps {
        return CopyDecision::no(
            "COPY_BITRATE",
            format!(
                "El vídeo usa {video_kbps} kbps y el máximo de {} es {max_video_kbps} kbps.",
                capacity.label()
            ),
        );
    }
    CopyDecision {
        copy: true,
        video_kbps: Some(video_kbps.max(1)),
        code: "COPY_OK",
        note: "Compatible: el vídeo se copia sin recodificar (más rápido y sin pérdida).".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvdforge_domain::{AvchdResolution, MediaProbe, VideoStreamInfo};

    fn probe(width: u32, height: u32, rate: FrameRate, field: &str) -> MediaProbe {
        MediaProbe {
            format_name: "mpegts".into(),
            duration_ms: Some(60_000),
            size_bytes: None,
            bitrate_bps: Some(20_000_000),
            video: Some(VideoStreamInfo {
                index: 0,
                codec: "h264".into(),
                width,
                height,
                frame_rate: rate,
                sample_aspect_ratio: None,
                display_aspect_ratio: None,
                pix_fmt: Some("yuv420p".into()),
                field_order: Some(field.into()),
                bit_depth: Some(8),
                profile: Some("High".into()),
                level: Some(41),
                bitrate_bps: Some(18_000_000),
            }),
            audio: Vec::new(),
            subtitle_streams: 0,
            subtitle_tracks: Vec::new(),
            chapters: Vec::new(),
        }
    }

    fn episode() -> Episode {
        serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "number": 1,
            "title": "Ep",
            "video": "00000000-0000-0000-0000-000000000002"
        }))
        .expect("episode")
    }

    fn config(resolution: AvchdResolution, rate: FrameRate) -> AvchdVideoConfig {
        AvchdVideoConfig {
            resolution,
            frame_rate: rate,
            ..AvchdVideoConfig::default()
        }
    }

    #[test]
    fn compliant_1080p24_stream_is_copied() {
        let decision = avchd_copy_decision(
            &episode(),
            &probe(1920, 1080, FrameRate::FPS_23_976, "progressive"),
            &config(AvchdResolution::R1080p, FrameRate::FPS_23_976),
            DiscCapacity::Bd25,
            38_000,
        );
        assert!(decision.copy, "{decision:?}");
        assert_eq!(decision.video_kbps, Some(18_000));
    }

    #[test]
    fn camcorder_1080i_reporting_field_rate_is_copied() {
        let decision = avchd_copy_decision(
            &episode(),
            &probe(1920, 1080, FrameRate::FPS_50, "tt"),
            &config(AvchdResolution::R1080i, FrameRate::PAL),
            DiscCapacity::Bd25,
            38_000,
        );
        assert!(decision.copy, "{decision:?}");
    }

    #[test]
    fn mismatches_and_trims_force_a_reencode() {
        let cfg = config(AvchdResolution::R1080p, FrameRate::FPS_23_976);
        let wrong_size = probe(1280, 720, FrameRate::FPS_23_976, "progressive");
        assert_eq!(
            avchd_copy_decision(&episode(), &wrong_size, &cfg, DiscCapacity::Bd25, 38_000).code,
            "COPY_RESOLUTION"
        );
        let wrong_rate = probe(1920, 1080, FrameRate::PAL, "progressive");
        assert_eq!(
            avchd_copy_decision(&episode(), &wrong_rate, &cfg, DiscCapacity::Bd25, 38_000).code,
            "COPY_FRAME_RATE"
        );
        let mut trimmed = episode();
        trimmed.trim_start_ms = Some(1_000);
        let ok = probe(1920, 1080, FrameRate::FPS_23_976, "progressive");
        assert_eq!(
            avchd_copy_decision(&trimmed, &ok, &cfg, DiscCapacity::Bd25, 38_000).code,
            "COPY_TRIM"
        );
        assert_eq!(
            avchd_copy_decision(&episode(), &ok, &cfg, DiscCapacity::Dvd5, 15_000).code,
            "COPY_BITRATE"
        );
        let mut reencode = episode();
        reencode.video_mode = VideoEncodeMode::Reencode;
        assert!(!avchd_copy_decision(&reencode, &ok, &cfg, DiscCapacity::Bd25, 38_000).copy);
        let mut hevc = ok.clone();
        hevc.video.as_mut().unwrap().codec = "hevc".into();
        assert_eq!(
            avchd_copy_decision(&episode(), &hevc, &cfg, DiscCapacity::Bd25, 38_000).code,
            "COPY_CODEC"
        );
    }
}
