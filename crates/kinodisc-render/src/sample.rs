//! "Test the quality": encode a few seconds of an episode exactly as the
//! disc will (same encoder, arguments, bitrate from the plan and picture
//! fixes), then compare against a lossless reference scaled the same way.
//! PSNR measures only what the compression loses; the two middle frames go
//! to the side-by-side view.

use crate::picture::{picture_filter, PictureSource, PictureTarget};
use kinodisc_domain::{AspectRatio, AuthoringMode, DvdProfile, EpisodeId, Project, VideoConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SAMPLE_SECONDS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QualitySample {
    /// Average luma+chroma PSNR of the sample against the source.
    pub psnr_db: f64,
    pub video_kbps: u32,
    pub start_ms: f64,
    pub seconds: f64,
    /// PNG of the source frame (scaled like the disc).
    pub original_png: String,
    /// PNG of the same frame after disc compression.
    pub disc_png: String,
}

/// Quality wording for a PSNR value (also used by the UI).
pub fn verdict(psnr_db: f64) -> &'static str {
    if psnr_db >= 42.0 {
        "excellent"
    } else if psnr_db >= 38.0 {
        "veryGood"
    } else if psnr_db >= 34.0 {
        "good"
    } else {
        "visible"
    }
}

/// Parses `average:NN.NN` from FFmpeg's psnr filter (`inf` = identical).
pub fn parse_psnr(stderr: &str) -> Option<f64> {
    let line = stderr.lines().rev().find(|line| line.contains("PSNR"))?;
    let value = &line[line.find("average:")? + "average:".len()..];
    let token: String = value.chars().take_while(|ch| !ch.is_whitespace()).collect();
    if token.starts_with("inf") {
        return Some(99.0);
    }
    token.parse::<f64>().ok()
}

async fn ffmpeg(args: Vec<String>) -> Result<String, String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(15 * 60),
        tokio::process::Command::from(kinodisc_toolchain::background_command(
            kinodisc_toolchain::resolve_binary("ffmpeg"),
        ))
        .args(&args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output(),
    )
    .await
    .map_err(|_| "FFmpeg tardó demasiado con la muestra.".to_string())?
    .map_err(|error| format!("No se pudo ejecutar FFmpeg: {error}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stderr)
    } else {
        Err(format!(
            "FFmpeg falló con la muestra: {}",
            stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
        ))
    }
}

fn input_args(path: &Path, start_ms: u64, seconds: u64) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", start_ms as f64 / 1000.0),
        "-protocol_whitelist".to_string(),
        crate::MEDIA_PROTOCOL_WHITELIST.to_string(),
        "-i".to_string(),
        path.display().to_string(),
        "-t".to_string(),
        seconds.to_string(),
    ]
}

/// Encodes `SAMPLE_SECONDS` of the episode from `at_ms` (offset inside its
/// trimmed range) at `video_kbps` into `out_dir` and measures it.
pub async fn quality_sample(
    project: &Project,
    project_dir: &Path,
    episode_id: EpisodeId,
    at_ms: u64,
    video_kbps: u32,
    out_dir: &Path,
) -> Result<QualitySample, String> {
    let episode = project
        .episodes
        .iter()
        .find(|episode| episode.id == episode_id)
        .ok_or("Episodio inexistente.")?;
    let asset = project
        .asset(episode.video)
        .ok_or("El episodio no tiene vídeo.")?;
    let input = crate::asset_path(project_dir, asset).ok_or("Ruta de vídeo inválida.")?;
    if !input.is_file() {
        return Err(format!("No se encuentra «{}».", input.display()));
    }
    let probe = kinodisc_media::probe(&input)
        .await
        .map_err(|error| error.to_string())?;
    let duration = probe.duration_ms.unwrap_or(0);
    let trim_start = episode.trim_start_ms.unwrap_or(0);
    let trim_end = episode.trim_end_ms.unwrap_or(duration).min(duration.max(1));
    let span = trim_end.saturating_sub(trim_start);
    if span < 1_000 {
        return Err("El episodio es demasiado corto para probar la calidad.".to_string());
    }
    let seconds = SAMPLE_SECONDS.min(span / 1000).max(1);
    let start = trim_start + at_ms.min(span.saturating_sub(seconds * 1000));
    std::fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    let source = PictureSource::from_probe(&probe);
    let fps = probe
        .video
        .as_ref()
        .map(|video| video.frame_rate)
        .unwrap_or_else(|| project.disc.standard.frame_rate());
    let sample_episode = kinodisc_domain::Episode {
        trim: None,
        trim_start_ms: Some(start),
        trim_end_ms: Some(start + seconds * 1000),
        ..episode.clone()
    };

    // 1) The disc encode and 2) the reference chain, per format.
    let (disc_video, reference_chain) = match (&project.disc.mode, &project.disc.video) {
        (AuthoringMode::Avchd, VideoConfig::Avchd(config)) => {
            let progressive = !config.resolution.interlaced();
            let pre = picture_filter(
                &episode.filters,
                source,
                PictureTarget {
                    display_aspect: 16.0 / 9.0,
                    progressive,
                    scale_mode: episode.scale_mode,
                },
            );
            let blu_ray = project.disc.capacity.is_blu_ray();
            let max = if blu_ray {
                kinodisc_avchd::BLURAY_MAX_VIDEO_KBPS
            } else {
                kinodisc_avchd::AVCHD_MAX_VIDEO_KBPS
            };
            let video_kbps = video_kbps.clamp(1, max);
            let request = kinodisc_avchd::AvchdEncodeRequest {
                input: input.clone(),
                output_dir: out_dir.to_path_buf(),
                stem: "sample".to_string(),
                config: config.clone(),
                disc_capacity: project.disc.capacity,
                aspect: project.disc.aspect,
                video_kbps,
                maxrate_kbps: if blu_ray {
                    config.vbv_maxrate_kbps.min(max).max(video_kbps)
                } else {
                    max
                },
                vbv_bufsize_kbps: if blu_ray {
                    config.vbv_bufsize_kbps.clamp(
                        kinodisc_avchd::BLURAY_VBV_MIN_KBITS,
                        kinodisc_avchd::BLURAY_VBV_MAX_KBITS,
                    )
                } else {
                    max
                },
                audios: Vec::new(),
                subtitles: Vec::new(),
                trim: None,
                trim_start_ms: sample_episode.trim_start_ms,
                trim_end_ms: sample_episode.trim_end_ms,
                scale_mode: episode.scale_mode,
                pre_filter: pre.clone(),
                fps,
                duration_ms: seconds * 1000,
                hardware_encoder: None,
                video_copy: false,
            };
            let spec = kinodisc_avchd::ResolutionSpec::from_config(config);
            let scale = kinodisc_avchd::build_filter(&request, spec);
            let encoded =
                kinodisc_avchd::AvchdEncoder::new(kinodisc_toolchain::resolve_binary("ffmpeg"))
                    .encode(&request)
                    .await
                    .map_err(|error| error.to_string())?;
            let chain = match pre {
                Some(pre) => format!("{pre},{scale}"),
                None => scale,
            };
            (encoded.stream_mkv, chain)
        }
        _ => {
            let interlaced = crate::picture::keeps_interlacing(
                &episode.filters,
                source.is_some_and(|source| source.interlaced),
            );
            let pre = picture_filter(
                &episode.filters,
                source,
                PictureTarget {
                    display_aspect: if project.disc.aspect == AspectRatio::SixteenNine {
                        16.0 / 9.0
                    } else {
                        4.0 / 3.0
                    },
                    progressive: !source.is_some_and(|source| source.interlaced),
                    scale_mode: episode.scale_mode,
                },
            );
            let video_kbps = video_kbps.clamp(1, kinodisc_dvd::DVD_MAX_VIDEO_KBPS);
            let video = kinodisc_dvd::VideoEncodeConfig {
                video_kbps,
                maxrate_kbps: (video_kbps + 3_000).min(kinodisc_dvd::DVD_MAX_VIDEO_KBPS),
                profile: DvdProfile::D1,
                standard: project.disc.standard,
                aspect: project.disc.aspect,
                gop_frames: match project.disc.standard {
                    kinodisc_domain::TvStandard::Ntsc => 18,
                    kinodisc_domain::TvStandard::Pal => 15,
                },
                b_frames: 2,
                interlaced,
                top_field_first: true,
                scale_mode: episode.scale_mode,
                pre_filter: pre.clone(),
            };
            let scale = kinodisc_dvd::build_scale_filter(&video);
            let request = kinodisc_dvd::EncodeRequest {
                input: input.clone(),
                output_dir: out_dir.to_path_buf(),
                stem: "sample".to_string(),
                video,
                audio_tracks: Vec::new(),
                trim: None,
                trim_start_ms: sample_episode.trim_start_ms,
                trim_end_ms: sample_episode.trim_end_ms,
                fps,
                passlog: out_dir.join("sample.passlog"),
            };
            let encoded =
                kinodisc_dvd::DvdEncoder::new(kinodisc_toolchain::resolve_binary("ffmpeg"))
                    .encode(&request)
                    .await
                    .map_err(|error| error.to_string())?;
            let chain = match pre {
                Some(pre) => format!("{pre},{scale}"),
                None => scale,
            };
            (encoded.video_m2v, chain)
        }
    };

    // Lossless reference through the same picture + scale chain.
    let reference = out_dir.join("reference.mkv");
    let mut args = input_args(&input, start, seconds);
    args.extend([
        "-an".into(),
        "-vf".into(),
        reference_chain,
        "-c:v".into(),
        "ffv1".into(),
    ]);
    args.push(reference.display().to_string());
    ffmpeg(args).await?;

    let metrics = ffmpeg(vec![
        "-hide_banner".into(),
        "-i".into(),
        disc_video.display().to_string(),
        "-i".into(),
        reference.display().to_string(),
        "-lavfi".into(),
        "[0:v]setsar=1[a];[1:v]setsar=1[b];[a][b]psnr".into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ])
    .await?;
    let psnr_db = parse_psnr(&metrics).ok_or("No se pudo medir la calidad.")?;

    // Pick the middle frame by number: a raw .m2v has no timestamps on its
    // B-frames, so seeking by time lands on different frames in each file.
    let disc_fps = match &project.disc.video {
        VideoConfig::Avchd(config) => config.frame_rate.as_f64(),
        _ => project.disc.standard.frame_rate().as_f64(),
    };
    let middle_frame = ((seconds as f64 * disc_fps) / 2.0).round() as u64;
    let frame = |source: &PathBuf, name: &str| {
        let png = out_dir.join(name);
        (
            vec![
                "-hide_banner".to_string(),
                "-y".to_string(),
                "-i".to_string(),
                source.display().to_string(),
                "-frames:v".to_string(),
                "1".to_string(),
                "-vf".to_string(),
                // Square pixels so both pictures show at the TV's shape.
                format!("select=eq(n\\,{middle_frame}),scale=iw*sar:ih,setsar=1"),
                png.display().to_string(),
            ],
            png,
        )
    };
    let (original_args, original_png) = frame(&reference, "original.png");
    let (disc_args, disc_png) = frame(&disc_video, "disc.png");
    ffmpeg(original_args).await?;
    ffmpeg(disc_args).await?;
    Ok(QualitySample {
        psnr_db: (psnr_db * 100.0).round() / 100.0,
        video_kbps,
        start_ms: start as f64,
        seconds: seconds as f64,
        original_png: original_png.display().to_string(),
        disc_png: disc_png.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psnr_lines_are_parsed() {
        let stderr =
            "[Parsed_psnr_2 @ 0x1] PSNR y:44.12 u:47.30 v:47.80 average:45.20 min:41.00 max:50.00";
        assert_eq!(parse_psnr(stderr), Some(45.2));
        assert_eq!(
            parse_psnr("[Parsed_psnr_2 @ 0x1] PSNR y:inf u:inf v:inf average:inf min:inf"),
            Some(99.0)
        );
        assert_eq!(parse_psnr("nothing"), None);
    }

    #[test]
    fn verdicts_follow_psnr() {
        assert_eq!(verdict(45.0), "excellent");
        assert_eq!(verdict(39.0), "veryGood");
        assert_eq!(verdict(35.0), "good");
        assert_eq!(verdict(30.0), "visible");
    }
}
