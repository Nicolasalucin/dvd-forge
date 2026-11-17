//! Photo slideshows: a list of pictures (+ optional music) becomes an
//! ordinary H.264 video that is then added to the project like any other
//! episode, so DVD, AVCHD and Blu-ray authoring need no special case.
//!
//! Pictures are letterboxed (never stretched or cropped) onto the target
//! frame and joined with cross-fades. The result is cached by content: the
//! same pictures and settings reuse the existing file.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Pictures per FFmpeg run; longer slideshows are built in chunks that are
/// then cross-faded together (keeps the filter graph and open files small).
const CHUNK: usize = 40;
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"];
const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "ac3"];

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowRequest {
    pub images: Vec<String>,
    /// Seconds each picture stays fully visible (1–60).
    pub seconds_per_image: f64,
    /// Cross-fade length in seconds (0 = hard cut, max 3).
    pub crossfade_seconds: f64,
    #[serde(default)]
    pub music: Option<String>,
    /// `true` for a 16:9 frame (1920x1080), `false` for 4:3 (1440x1080).
    pub widescreen: bool,
    /// Frames per second of the generated video (25 or 29.97 recommended).
    pub fps: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SlideshowOutcome {
    pub path: String,
    pub duration_ms: f64,
    pub cached: bool,
}

fn has_extension(path: &Path, list: &[&str]) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| list.contains(&value.to_ascii_lowercase().as_str()))
}

/// Validated, absolute inputs.
pub fn validate(request: &SlideshowRequest) -> Result<(Vec<PathBuf>, Option<PathBuf>), String> {
    if request.images.is_empty() {
        return Err("Elige al menos una foto.".to_string());
    }
    if request.images.len() > 2_000 {
        return Err("Una presentación admite hasta 2000 fotos.".to_string());
    }
    if !(1.0..=60.0).contains(&request.seconds_per_image) {
        return Err("Cada foto debe durar entre 1 y 60 segundos.".to_string());
    }
    if !(0.0..=3.0).contains(&request.crossfade_seconds)
        || request.crossfade_seconds >= request.seconds_per_image
    {
        return Err(
            "El fundido debe durar entre 0 y 3 segundos y menos que cada foto.".to_string(),
        );
    }
    if !(10.0..=60.0).contains(&request.fps) {
        return Err("Velocidad de fotogramas no válida.".to_string());
    }
    let check = |value: &str, kinds: &[&str], what: &str| -> Result<PathBuf, String> {
        let path = PathBuf::from(value);
        if !path.is_absolute() || !path.is_file() || !has_extension(&path, kinds) {
            return Err(format!("{what} no válido: {value}"));
        }
        Ok(path)
    };
    let images = request
        .images
        .iter()
        .map(|image| check(image, IMAGE_EXTENSIONS, "Archivo de imagen"))
        .collect::<Result<Vec<_>, _>>()?;
    let music = request
        .music
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| check(value, AUDIO_EXTENSIONS, "Archivo de música"))
        .transpose()?;
    Ok((images, music))
}

pub fn frame_size(widescreen: bool) -> (u32, u32) {
    if widescreen {
        (1920, 1080)
    } else {
        (1440, 1080)
    }
}

/// Total length in seconds of `count` pictures.
pub fn total_seconds(count: usize, seconds: f64, fade: f64) -> f64 {
    count as f64 * seconds + fade
}

fn fmt(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Video filter graph that joins `lengths.len()` inputs (each already a
/// clip of the given length in seconds) with cross-fades of `fade` seconds,
/// or with a plain concat when `fade` is 0. Output label `[vout]`.
pub fn join_graph(lengths: &[f64], fade: f64, prepared: &[String]) -> String {
    let count = lengths.len();
    if count == 1 {
        return format!("{}null[vout]", prepared[0]);
    }
    if fade <= 0.0 {
        let inputs: String = prepared.iter().map(String::as_str).collect();
        return format!("{inputs}concat=n={count}:v=1:a=0[vout]");
    }
    let mut graph = String::new();
    let mut previous = prepared[0].clone();
    let mut elapsed = lengths[0];
    for index in 1..count {
        let offset = elapsed - fade;
        let label = if index + 1 == count {
            "[vout]".to_string()
        } else {
            format!("[x{index}]")
        };
        graph.push_str(&format!(
            "{previous}{}xfade=transition=fade:duration={}:offset={}{label};",
            prepared[index],
            fmt(fade),
            fmt(offset)
        ));
        previous = label;
        elapsed = elapsed + lengths[index] - fade;
    }
    graph.trim_end_matches(';').to_string()
}

/// FFmpeg arguments for one chunk of pictures (video only).
pub fn chunk_args(
    images: &[PathBuf],
    seconds: f64,
    fade: f64,
    fps: f64,
    size: (u32, u32),
    output: &Path,
) -> Vec<String> {
    let (width, height) = size;
    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    let clip = seconds + fade;
    for image in images {
        args.extend([
            "-protocol_whitelist".to_string(),
            "file".to_string(),
            "-loop".to_string(),
            "1".to_string(),
            "-framerate".to_string(),
            fmt(fps),
            "-t".to_string(),
            fmt(clip),
            "-i".to_string(),
            image.display().to_string(),
        ]);
    }
    let mut graph = String::new();
    let mut labels = Vec::new();
    for index in 0..images.len() {
        graph.push_str(&format!(
            "[{index}:v]scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps={},format=yuv420p,trim=duration={}[p{index}];",
            fmt(fps),
            fmt(clip)
        ));
        labels.push(format!("[p{index}]"));
    }
    let lengths = vec![clip; images.len()];
    graph.push_str(&join_graph(&lengths, fade, &labels));
    args.extend([
        "-filter_complex".to_string(),
        graph,
        "-map".to_string(),
        "[vout]".to_string(),
    ]);
    args.extend(video_codec_args(fps));
    args.push(output.display().to_string());
    args
}

fn video_codec_args(fps: f64) -> Vec<String> {
    [
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "16", "-pix_fmt", "yuv420p", "-r",
    ]
    .iter()
    .map(|value| value.to_string())
    .chain(std::iter::once(fmt(fps)))
    .collect()
}

/// FFmpeg arguments that join the chunk videos and add the soundtrack
/// (music faded out at the end, or silence so every disc type has audio).
pub fn final_args(
    chunks: &[(PathBuf, f64)],
    fade: f64,
    fps: f64,
    music: Option<&Path>,
    total: f64,
    output: &Path,
) -> Vec<String> {
    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    for (chunk, _) in chunks {
        args.extend([
            "-protocol_whitelist".to_string(),
            "file".to_string(),
            "-i".to_string(),
            chunk.display().to_string(),
        ]);
    }
    let audio_index = chunks.len();
    match music {
        Some(path) => args.extend([
            "-protocol_whitelist".to_string(),
            "file".to_string(),
            "-i".to_string(),
            path.display().to_string(),
        ]),
        None => args.extend([
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "anullsrc=r=48000:cl=stereo".to_string(),
        ]),
    }
    let labels: Vec<String> = (0..chunks.len())
        .map(|index| format!("[{index}:v]"))
        .collect();
    let lengths: Vec<f64> = chunks.iter().map(|(_, length)| *length).collect();
    let mut graph = join_graph(&lengths, fade, &labels);
    let fade_out = (total - 2.0).max(0.0);
    graph.push_str(&format!(
        ";[{audio_index}:a]aformat=sample_rates=48000:channel_layouts=stereo,apad,atrim=0:{},afade=t=out:st={}:d=2[aout]",
        fmt(total),
        fmt(fade_out)
    ));
    args.extend([
        "-filter_complex".to_string(),
        graph,
        "-map".to_string(),
        "[vout]".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
    ]);
    args.extend(video_codec_args(fps));
    args.extend(
        ["-c:a", "aac", "-b:a", "192k", "-t"]
            .iter()
            .map(|value| value.to_string()),
    );
    args.push(fmt(total));
    args.extend(["-movflags".to_string(), "+faststart".to_string()]);
    args.push(output.display().to_string());
    args
}

/// Content key: settings plus every input's path, size and modification
/// time (changing a photo invalidates the cached video).
pub fn cache_key(request: &SlideshowRequest, images: &[PathBuf], music: Option<&Path>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"kinodisc-slideshow-v1");
    hasher.update(
        format!(
            "{}|{}|{}|{}",
            request.seconds_per_image, request.crossfade_seconds, request.widescreen, request.fps
        )
        .as_bytes(),
    );
    for path in images.iter().map(PathBuf::as_path).chain(music) {
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

async fn run_ffmpeg(args: &[String]) -> Result<(), String> {
    let ffmpeg = kinodisc_toolchain::resolve_binary("ffmpeg");
    let output = tokio::process::Command::from(kinodisc_toolchain::background_command(&ffmpeg))
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("No se pudo ejecutar FFmpeg: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail: Vec<&str> = stderr.lines().rev().take(6).collect();
    Err(format!(
        "FFmpeg no pudo crear la presentación: {}",
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    ))
}

/// Builds (or reuses) the slideshow video in `cache_dir`.
pub async fn build_slideshow(
    request: &SlideshowRequest,
    cache_dir: &Path,
) -> Result<SlideshowOutcome, String> {
    let (images, music) = validate(request)?;
    let fade = request.crossfade_seconds;
    let seconds = request.seconds_per_image;
    let total = total_seconds(images.len(), seconds, fade);
    let key = cache_key(request, &images, music.as_deref());
    std::fs::create_dir_all(cache_dir).map_err(|error| error.to_string())?;
    let target = cache_dir.join(format!("slideshow-{key}.mp4"));
    if target.is_file() && std::fs::metadata(&target).is_ok_and(|meta| meta.len() > 0) {
        return Ok(SlideshowOutcome {
            path: target.display().to_string(),
            duration_ms: (total * 1000.0).round(),
            cached: true,
        });
    }
    let work = cache_dir.join(format!(".work-{key}"));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let size = frame_size(request.widescreen);
    let result = async {
        let mut chunks = Vec::new();
        for (index, group) in images.chunks(CHUNK).enumerate() {
            let path = work.join(format!("chunk-{index:03}.mp4"));
            run_ffmpeg(&chunk_args(group, seconds, fade, request.fps, size, &path)).await?;
            chunks.push((path, total_seconds(group.len(), seconds, fade)));
        }
        let partial = work.join("slideshow.mp4");
        run_ffmpeg(&final_args(
            &chunks,
            fade,
            request.fps,
            music.as_deref(),
            total,
            &partial,
        ))
        .await?;
        std::fs::rename(&partial, &target).map_err(|error| error.to_string())
    }
    .await;
    let _ = std::fs::remove_dir_all(&work);
    result?;
    Ok(SlideshowOutcome {
        path: target.display().to_string(),
        duration_ms: (total * 1000.0).round(),
        cached: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfade_offsets_accumulate_correctly() {
        let labels: Vec<String> = (0..3).map(|i| format!("[p{i}]")).collect();
        // Three clips of 6 s (5 s visible + 1 s fade): offsets 5 and 10.
        let graph = join_graph(&[6.0, 6.0, 6.0], 1.0, &labels);
        assert_eq!(
            graph,
            "[p0][p1]xfade=transition=fade:duration=1:offset=5[x1];[x1][p2]xfade=transition=fade:duration=1:offset=10[vout]"
        );
        assert_eq!(total_seconds(3, 5.0, 1.0), 16.0);
        assert_eq!(
            join_graph(&[4.0, 4.0], 0.0, &labels[..2]),
            "[p0][p1]concat=n=2:v=1:a=0[vout]"
        );
        assert_eq!(join_graph(&[4.0], 1.0, &labels[..1]), "[p0]null[vout]");
    }

    #[test]
    fn chunked_join_matches_the_single_pass_length() {
        // 90 pictures → chunks of 40, 40, 10, joined with the same fade.
        let lengths = [
            total_seconds(40, 4.0, 1.0),
            total_seconds(40, 4.0, 1.0),
            total_seconds(10, 4.0, 1.0),
        ];
        let joined: f64 = lengths.iter().sum::<f64>() - 2.0 * 1.0;
        assert_eq!(joined, total_seconds(90, 4.0, 1.0));
    }

    #[test]
    fn inputs_are_validated() {
        let dir = std::env::temp_dir().join(format!("kinodisc-slides-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let photo = dir.join("a.jpg");
        std::fs::write(&photo, b"x").unwrap();
        let script = dir.join("a.exe");
        std::fs::write(&script, b"x").unwrap();
        let mut request = SlideshowRequest {
            images: vec![photo.display().to_string()],
            seconds_per_image: 5.0,
            crossfade_seconds: 1.0,
            music: None,
            widescreen: true,
            fps: 25.0,
        };
        assert!(validate(&request).is_ok());
        request.images.push(script.display().to_string());
        assert!(validate(&request).is_err());
        request.images.pop();
        request.images.push("relative.jpg".into());
        assert!(validate(&request).is_err());
        request.images.pop();
        request.crossfade_seconds = 5.0;
        assert!(validate(&request).is_err());
        let args = chunk_args(
            std::slice::from_ref(&photo),
            5.0,
            1.0,
            25.0,
            (1920, 1080),
            &dir.join("o.mp4"),
        );
        // Every input is preceded by a file-only protocol whitelist.
        let position = args.iter().position(|arg| arg == "-i").unwrap();
        assert_eq!(args[position - 7], "file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
