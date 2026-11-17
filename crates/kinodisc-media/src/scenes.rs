//! Automatic chapters from scene changes: FFmpeg's scene score
//! (`select='gt(scene,T)'` + `showinfo`, available in every FFmpeg version)
//! on a small decoded copy of the video, then a minimum gap between
//! chapters and the disc's chapter limit.

use crate::MediaError;
use kinodisc_toolchain::{resolve_binary, MEDIA_PROTOCOL_WHITELIST};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// How strong a cut must be. Lower finds more scenes.
#[derive(Debug, Clone, Copy)]
pub struct SceneOptions {
    /// FFmpeg scene score threshold, 0.05..0.9 (0.33 is a good default).
    pub threshold: f64,
    /// No two chapters closer than this (milliseconds).
    pub min_gap_ms: u64,
    /// Only cuts inside `start_ms..end_ms` of the source count.
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    /// Chapter limit of the disc (99 on DVD, 999 on Blu-ray), the start
    /// chapter included.
    pub max_chapters: usize,
}

/// Times of scene changes as offsets from `start_ms`, starting with 0 (the
/// first chapter), at most `max_chapters` entries.
pub async fn detect_scenes(
    path: impl AsRef<Path>,
    options: SceneOptions,
) -> Result<Vec<u64>, MediaError> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(MediaError::NotAFile(path.display().to_string()));
    }
    let threshold = options.threshold.clamp(0.05, 0.9);
    let filter = format!("scale=320:-2,select='gt(scene,{threshold:.3})',showinfo");
    let mut command = Command::from(kinodisc_toolchain::background_command(resolve_binary(
        "ffmpeg",
    )));
    command
        .args(["-hide_banner", "-nostats", "-loglevel", "info"])
        .arg("-protocol_whitelist")
        .arg(MEDIA_PROTOCOL_WHITELIST)
        .arg("-i")
        .arg(path)
        .args(["-an", "-sn", "-dn", "-vf", &filter, "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Decoding a feature film at 320 px takes a few minutes at most.
    let output = tokio::time::timeout(Duration::from_secs(60 * 60), command.output())
        .await
        .map_err(|_| MediaError::Timeout)??;
    if !output.status.success() {
        return Err(MediaError::FfprobeFailed {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(400)
                .collect(),
        });
    }
    let cuts = parse_scene_times(&String::from_utf8_lossy(&output.stderr));
    Ok(pick_chapters(&cuts, options))
}

/// `pts_time:` values of the frames `showinfo` printed, in milliseconds.
pub fn parse_scene_times(stderr: &str) -> Vec<u64> {
    stderr
        .lines()
        .filter(|line| line.contains("Parsed_showinfo"))
        .filter_map(|line| {
            let rest = &line[line.find("pts_time:")? + "pts_time:".len()..];
            let value: String = rest
                .trim_start()
                .chars()
                .take_while(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == '-')
                .collect();
            value.parse::<f64>().ok().filter(|seconds| *seconds >= 0.0)
        })
        .map(|seconds| (seconds * 1000.0).round() as u64)
        .collect()
}

/// Keeps cuts inside the trim range, at least `min_gap_ms` apart (also from
/// the start and the end), as offsets from the range start, chapter 0 first.
pub fn pick_chapters(cuts_ms: &[u64], options: SceneOptions) -> Vec<u64> {
    let mut chapters = vec![0u64];
    let end = options.end_ms.unwrap_or(u64::MAX);
    let mut sorted: Vec<u64> = cuts_ms.to_vec();
    sorted.sort_unstable();
    for cut in sorted {
        if chapters.len() >= options.max_chapters.max(1) {
            break;
        }
        if cut <= options.start_ms || cut >= end {
            continue;
        }
        let offset = cut - options.start_ms;
        let last = *chapters.last().unwrap_or(&0);
        if offset < last + options.min_gap_ms {
            continue;
        }
        if end != u64::MAX && end - cut < options.min_gap_ms / 2 {
            continue;
        }
        chapters.push(offset);
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> SceneOptions {
        SceneOptions {
            threshold: 0.33,
            min_gap_ms: 30_000,
            start_ms: 0,
            end_ms: None,
            max_chapters: 99,
        }
    }

    #[test]
    fn showinfo_times_are_parsed() {
        let stderr = "\
[Parsed_showinfo_2 @ 000001] n:   0 pts:  15360 pts_time:15      duration:512 fmt:yuv420p
[Parsed_showinfo_2 @ 000001] n:   1 pts:  30720 pts_time:30.04   duration:512
frame=  120 fps=0.0 q=-0.0 size=N/A time=00:00:45.00
[Parsed_showinfo_2 @ 000001] n:   2 pts:  46080 pts_time:45.5 duration:512";
        assert_eq!(parse_scene_times(stderr), vec![15_000, 30_040, 45_500]);
    }

    #[test]
    fn chapters_keep_a_minimum_gap() {
        let cuts = [5_000, 31_000, 40_000, 70_000, 95_000, 125_000];
        assert_eq!(
            pick_chapters(&cuts, options()),
            vec![0, 31_000, 70_000, 125_000]
        );
    }

    #[test]
    fn chapters_follow_the_trim_range_and_the_limit() {
        let cuts = [10_000, 50_000, 90_000, 130_000, 170_000];
        let trimmed = SceneOptions {
            start_ms: 40_000,
            end_ms: Some(140_000),
            ..options()
        };
        // Offsets from the trim start; 130 s is only 10 s from the end.
        assert_eq!(pick_chapters(&cuts, trimmed), vec![0, 50_000]);
        let limited = SceneOptions {
            max_chapters: 2,
            min_gap_ms: 1_000,
            ..options()
        };
        assert_eq!(pick_chapters(&cuts, limited), vec![0, 10_000]);
    }
}
