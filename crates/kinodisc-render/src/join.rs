//! "Join videos": several clips (the parts of a camcorder recording, phone
//! videos of one day…) become one episode. FFmpeg's concat filter joins
//! them after giving every clip the same size, frame rate and audio layout
//! (silence for clips without sound), into a high-quality H.264/AAC
//! intermediate in the cache — like photo slideshows. The disc encoder then
//! treats it as any other video, and a chapter starts at every clip.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_CLIPS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub clips: Vec<String>,
    /// `true` for a 16:9 frame (1920x1080), `false` for 4:3 (1440x1080).
    pub widescreen: bool,
    /// Frames per second of the joined video (the disc's rate).
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct JoinOutcome {
    pub path: String,
    pub duration_ms: f64,
    /// Where each clip starts in the joined video (first is 0).
    pub clip_starts_ms: Vec<f64>,
    pub cached: bool,
}

/// One clip as FFmpeg needs it.
#[derive(Debug, Clone)]
pub struct JoinClip {
    pub path: PathBuf,
    pub duration_s: f64,
    pub has_audio: bool,
}

pub fn validate(request: &JoinRequest) -> Result<Vec<PathBuf>, String> {
    if request.clips.len() < 2 {
        return Err("Elige al menos dos vídeos para unir.".to_string());
    }
    if request.clips.len() > MAX_CLIPS {
        return Err(format!(
            "Se pueden unir hasta {MAX_CLIPS} vídeos de una vez."
        ));
    }
    if !(10.0..=120.0).contains(&request.fps) {
        return Err("Fotogramas por segundo no válidos.".to_string());
    }
    request
        .clips
        .iter()
        .map(|clip| {
            let path = PathBuf::from(clip);
            if path.is_file() {
                Ok(path)
            } else {
                Err(format!("No se encuentra «{clip}»."))
            }
        })
        .collect()
}

pub fn frame_size(widescreen: bool) -> (u32, u32) {
    if widescreen {
        (1920, 1080)
    } else {
        (1440, 1080)
    }
}

fn fmt(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// FFmpeg arguments: every clip scaled into the frame (bars, never cropped),
/// deinterlaced if needed, same rate; stereo 48 kHz audio or silence.
pub fn join_args(clips: &[JoinClip], widescreen: bool, fps: f64, output: &Path) -> Vec<String> {
    let (width, height) = frame_size(widescreen);
    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    for clip in clips {
        args.extend([
            // A clip whose size or colour space changes midway (camera mode
            // switch, an already joined file) made FFmpeg rebuild the graph,
            // which stalled the concat for good. The chain below normalises
            // every frame anyway.
            "-reinit_filter".to_string(),
            "0".to_string(),
            "-protocol_whitelist".to_string(),
            "file".to_string(),
            "-i".to_string(),
            clip.path.display().to_string(),
        ]);
    }
    // Silence for clips without sound, one input each (exact length).
    let mut silence_inputs = Vec::new();
    for clip in clips.iter().filter(|clip| !clip.has_audio) {
        silence_inputs.push(clips.len() + silence_inputs.len());
        args.extend([
            "-f".to_string(),
            "lavfi".to_string(),
            "-t".to_string(),
            fmt(clip.duration_s.max(0.04)),
            "-i".to_string(),
            "anullsrc=r=48000:cl=stereo".to_string(),
        ]);
    }
    let mut graph = Vec::new();
    let mut silence = silence_inputs.iter();
    let mut pairs = String::new();
    for (index, clip) in clips.iter().enumerate() {
        graph.push(format!(
            "[{index}:v]bwdif=mode=send_frame:deint=interlaced,scale={width}:{height}:force_original_aspect_ratio=decrease:out_range=tv,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={},format=yuv420p,trim=duration={},setpts=PTS-STARTPTS[v{index}]",
            fmt(fps),
            fmt(clip.duration_s)
        ));
        let audio_input = if clip.has_audio {
            format!("{index}:a:0")
        } else {
            format!("{}:a", silence.next().copied().unwrap_or(index))
        };
        graph.push(format!(
            "[{audio_input}]aformat=sample_rates=48000:channel_layouts=stereo,aresample=async=1:first_pts=0,apad,atrim=duration={},asetpts=PTS-STARTPTS[a{index}]",
            fmt(clip.duration_s)
        ));
        pairs.push_str(&format!("[v{index}][a{index}]"));
    }
    graph.push(format!(
        "{pairs}concat=n={}:v=1:a=1[vout][aout]",
        clips.len()
    ));
    args.extend([
        "-filter_complex".to_string(),
        graph.join(";"),
        "-map".to_string(),
        "[vout]".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
    ]);
    // Near-lossless intermediate: the disc encoder compresses it later.
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "14",
            "-pix_fmt",
            "yuv420p",
            "-color_range",
            "tv",
            "-c:a",
            "aac",
            "-b:a",
            "320k",
            "-movflags",
            "+faststart",
        ]
        .iter()
        .map(|value| value.to_string()),
    );
    args.push(output.display().to_string());
    args
}

/// Start of every clip in the joined video, in milliseconds.
pub fn clip_starts_ms(clips: &[JoinClip]) -> Vec<f64> {
    let mut start = 0.0;
    clips
        .iter()
        .map(|clip| {
            let here = start;
            start += clip.duration_s;
            (here * 1000.0).round()
        })
        .collect()
}

fn cache_key(request: &JoinRequest, clips: &[PathBuf]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"kinodisc-join-v2");
    hasher.update(format!("{}|{}", request.widescreen, request.fps).as_bytes());
    for path in clips {
        hasher.update(path.display().to_string().as_bytes());
        if let Ok(metadata) = std::fs::metadata(path) {
            hasher.update(metadata.len().to_le_bytes());
            if let Ok(modified) = metadata.modified() {
                if let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH) {
                    hasher.update(since.as_nanos().to_le_bytes());
                }
            }
        }
        hasher.update([0]);
    }
    hex::encode(&hasher.finalize()[..16])
}

/// Joins (or reuses) the clips into one video in `cache_dir`.
pub async fn build_join(request: &JoinRequest, cache_dir: &Path) -> Result<JoinOutcome, String> {
    let paths = validate(request)?;
    let mut clips = Vec::with_capacity(paths.len());
    for path in &paths {
        let probe = kinodisc_media::probe(path)
            .await
            .map_err(|error| format!("«{}»: {error}", path.display()))?;
        if probe.video.is_none() {
            return Err(format!("«{}» no tiene vídeo.", path.display()));
        }
        let duration_ms = probe.duration_ms.unwrap_or(0);
        if duration_ms == 0 {
            return Err(format!("No se conoce la duración de «{}».", path.display()));
        }
        clips.push(JoinClip {
            path: path.clone(),
            duration_s: duration_ms as f64 / 1000.0,
            has_audio: !probe.audio.is_empty(),
        });
    }
    let starts = clip_starts_ms(&clips);
    let total_ms: f64 = clips
        .iter()
        .map(|clip| clip.duration_s * 1000.0)
        .sum::<f64>()
        .round();
    let key = cache_key(request, &paths);
    std::fs::create_dir_all(cache_dir).map_err(|error| error.to_string())?;
    let target = cache_dir.join(format!("join-{key}.mp4"));
    if target.is_file() && std::fs::metadata(&target).is_ok_and(|meta| meta.len() > 0) {
        return Ok(JoinOutcome {
            path: target.display().to_string(),
            duration_ms: total_ms,
            clip_starts_ms: starts,
            cached: true,
        });
    }
    let partial = cache_dir.join(format!("join-{key}.part.mp4"));
    let args = join_args(&clips, request.widescreen, request.fps, &partial);
    let ffmpeg = kinodisc_toolchain::resolve_binary("ffmpeg");
    // Never wait forever on a stalled FFmpeg: 10 min plus 3x the length.
    let limit = std::time::Duration::from_secs(600 + (total_ms / 1000.0 * 3.0) as u64);
    let output = tokio::time::timeout(
        limit,
        tokio::process::Command::from(kinodisc_toolchain::background_command(&ffmpeg))
            .args(&args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        let _ = std::fs::remove_file(&partial);
        "FFmpeg dejó de avanzar al unir los vídeos.".to_string()
    })?
    .map_err(|error| format!("No se pudo ejecutar FFmpeg: {error}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&partial);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
        return Err(format!(
            "FFmpeg no pudo unir los vídeos: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
        ));
    }
    std::fs::rename(&partial, &target).map_err(|error| error.to_string())?;
    Ok(JoinOutcome {
        path: target.display().to_string(),
        duration_ms: total_ms,
        clip_starts_ms: starts,
        cached: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(name: &str, seconds: f64, audio: bool) -> JoinClip {
        JoinClip {
            path: PathBuf::from(name),
            duration_s: seconds,
            has_audio: audio,
        }
    }

    #[test]
    fn clips_start_one_after_the_other() {
        let clips = [
            clip("a.mp4", 12.5, true),
            clip("b.mp4", 30.0, false),
            clip("c.mp4", 4.0, true),
        ];
        assert_eq!(clip_starts_ms(&clips), vec![0.0, 12_500.0, 42_500.0]);
    }

    #[test]
    fn silent_clips_get_their_own_silence() {
        let clips = [clip("a.mp4", 10.0, true), clip("b.mp4", 5.0, false)];
        let args = join_args(&clips, true, 25.0, Path::new("out.mp4"));
        let joined = args.join(" ");
        // Two files plus one anullsrc of exactly 5 s, used by clip 1.
        assert!(joined.contains("-t 5 -i anullsrc=r=48000:cl=stereo"));
        assert!(joined.contains("[2:a]aformat"));
        assert!(joined.contains("[0:a:0]aformat"));
        assert!(joined.contains("concat=n=2:v=1:a=1[vout][aout]"));
        assert!(joined.contains("scale=1920:1080:force_original_aspect_ratio=decrease"));
        assert_eq!(joined.matches("-reinit_filter 0").count(), 2);
    }

    #[test]
    fn requests_are_checked() {
        let one = JoinRequest {
            clips: vec!["a.mp4".to_string()],
            widescreen: true,
            fps: 25.0,
        };
        assert!(validate(&one).is_err());
        let missing = JoinRequest {
            clips: vec!["/nope/a.mp4".to_string(), "/nope/b.mp4".to_string()],
            widescreen: true,
            fps: 25.0,
        };
        assert!(validate(&missing).is_err());
    }
}
