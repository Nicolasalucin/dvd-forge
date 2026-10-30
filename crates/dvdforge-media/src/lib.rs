//! FFprobe adapter.
//!
//! The parser keeps rational frame rates (30000/1001), SAR/DAR strings and
//! audio stream metadata. It does not guess duration or resolution from the
//! file name; everything comes from FFprobe JSON.

use dvdforge_domain::{
    AudioStreamInfo, FrameRate, MediaChapter, MediaProbe, SubtitleStreamInfo, VideoStreamInfo,
};
use dvdforge_toolchain::{resolve_binary, MEDIA_PROTOCOL_WHITELIST};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub const CRATE_NAME: &str = "dvdforge-media";
/// Upper bound for a local probe. A hung or malformed file must not keep an
/// IPC command alive forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("no se pudo ejecutar ffprobe: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffprobe falló con código {code}: {stderr}")]
    FfprobeFailed { code: i32, stderr: String },
    #[error("ffprobe superó el tiempo límite")]
    Timeout,
    #[error("la ruta no es un archivo regular: {0}")]
    NotAFile(String),
    #[error("salida JSON de ffprobe inválida: {0}")]
    Json(#[from] serde_json::Error),
    #[error("el stream de video {0} no tiene framerate válido")]
    InvalidFrameRate(u32),
    #[error("el stream de video {0} no tiene dimensiones")]
    MissingVideoDimensions(u32),
}

/// Async wrapper around the `ffprobe` binary.
#[derive(Debug, Clone)]
pub struct Ffprobe {
    pub binary: PathBuf,
}

impl Default for Ffprobe {
    fn default() -> Self {
        Self {
            binary: resolve_binary("ffprobe"),
        }
    }
}

impl Ffprobe {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub async fn probe(&self, path: impl AsRef<Path>) -> Result<MediaProbe, MediaError> {
        let path = path.as_ref();
        if !path.is_file() {
            return Err(MediaError::NotAFile(path.display().to_string()));
        }
        let output = timeout(
            PROBE_TIMEOUT,
            Command::from(dvdforge_toolchain::background_command(&self.binary))
                .kill_on_drop(true)
                .arg("-v")
                .arg("error")
                .arg("-protocol_whitelist")
                .arg(MEDIA_PROTOCOL_WHITELIST)
                .arg("-print_format")
                .arg("json")
                .arg("-show_format")
                .arg("-show_streams")
                .arg("-show_chapters")
                .arg(path)
                .output(),
        )
        .await
        .map_err(|_| MediaError::Timeout)??;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(MediaError::FfprobeFailed { code, stderr });
        }

        parse_probe_json(&output.stdout)
    }
}

/// Probes a file using `ffprobe` from `PATH`.
pub async fn probe(path: impl AsRef<Path>) -> Result<MediaProbe, MediaError> {
    Ffprobe::default().probe(path).await
}

#[derive(Debug, Default, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: FfprobeFormat,
    #[serde(default)]
    chapters: Vec<FfprobeChapter>,
}

#[derive(Debug, Default, Deserialize)]
struct FfprobeChapter {
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Default, Deserialize)]
struct FfprobeFormat {
    #[serde(default)]
    format_name: Option<String>,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    r_frame_rate: Option<String>,
    #[serde(default)]
    avg_frame_rate: Option<String>,
    #[serde(default)]
    sample_aspect_ratio: Option<String>,
    #[serde(default)]
    display_aspect_ratio: Option<String>,
    #[serde(default)]
    pix_fmt: Option<String>,
    #[serde(default)]
    field_order: Option<String>,
    #[serde(default)]
    bits_per_raw_sample: Option<String>,
    #[serde(default)]
    bits_per_sample: Option<u32>,
    #[serde(default)]
    channels: Option<u16>,
    #[serde(default)]
    sample_rate: Option<String>,
    #[serde(default)]
    bit_rate: Option<String>,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    tags: Option<HashMap<String, String>>,
    #[serde(default)]
    disposition: Option<HashMap<String, serde_json::Value>>,
}

fn parse_probe_json(bytes: &[u8]) -> Result<MediaProbe, MediaError> {
    let raw: FfprobeOutput = serde_json::from_slice(bytes)?;
    let format = raw.format;

    let video_stream = raw
        .streams
        .iter()
        .find(|stream| is_type(stream, "video") && !is_attached_picture(stream));

    let video = match video_stream {
        Some(stream) => {
            let frame_rate = parse_frame_rate(stream.r_frame_rate.as_deref())
                .or_else(|| parse_frame_rate(stream.avg_frame_rate.as_deref()))
                .ok_or(MediaError::InvalidFrameRate(stream.index))?;
            let width = stream
                .width
                .ok_or(MediaError::MissingVideoDimensions(stream.index))?;
            let height = stream
                .height
                .ok_or(MediaError::MissingVideoDimensions(stream.index))?;
            Some(VideoStreamInfo {
                index: stream.index,
                codec: stream
                    .codec_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                width,
                height,
                frame_rate,
                sample_aspect_ratio: stream.sample_aspect_ratio.clone(),
                display_aspect_ratio: stream.display_aspect_ratio.clone(),
                pix_fmt: stream.pix_fmt.clone(),
                field_order: stream.field_order.clone(),
                bit_depth: stream
                    .bits_per_raw_sample
                    .as_deref()
                    .and_then(parse_u8)
                    .or(stream
                        .bits_per_sample
                        .and_then(|value| u8::try_from(value).ok())),
            })
        }
        None => None,
    };

    let audio = raw
        .streams
        .iter()
        .filter(|stream| is_type(stream, "audio"))
        .map(|stream| AudioStreamInfo {
            index: stream.index,
            codec: stream
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            channels: stream.channels.unwrap_or(0),
            sample_rate: stream
                .sample_rate
                .as_deref()
                .and_then(parse_u32)
                .unwrap_or(0),
            bitrate_bps: stream.bit_rate.as_deref().and_then(parse_u64),
            language: stream.tags.as_ref().and_then(tag_language),
            title: stream.tags.as_ref().and_then(tag_title),
            default: disposition_flag(stream, "default"),
            commentary: disposition_flag(stream, "comment"),
        })
        .collect();

    let subtitle_tracks: Vec<SubtitleStreamInfo> = raw
        .streams
        .iter()
        .filter(|stream| is_type(stream, "subtitle"))
        .map(|stream| SubtitleStreamInfo {
            index: stream.index,
            codec: stream
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            language: stream.tags.as_ref().and_then(tag_language),
            title: stream.tags.as_ref().and_then(tag_title),
            default: disposition_flag(stream, "default"),
            forced: disposition_flag(stream, "forced"),
            bitmap: stream
                .codec_name
                .as_deref()
                .is_some_and(is_bitmap_subtitle_codec),
        })
        .collect();
    let subtitle_streams = subtitle_tracks.len() as u32;

    let chapters = raw
        .chapters
        .iter()
        .enumerate()
        .filter_map(|(index, chapter)| {
            let start_ms = parse_duration_ms(chapter.start_time.as_deref())?;
            Some(MediaChapter {
                start_ms,
                end_ms: parse_duration_ms(chapter.end_time.as_deref()),
                title: chapter
                    .tags
                    .as_ref()
                    .and_then(tag_title)
                    .unwrap_or_else(|| format!("Capítulo {}", index + 1)),
                language: chapter.tags.as_ref().and_then(tag_language),
            })
        })
        .collect();

    let stream_duration_ms = raw
        .streams
        .iter()
        .filter_map(|stream| parse_duration_ms(stream.duration.as_deref()))
        .max();

    let duration_ms = parse_duration_ms(format.duration.as_deref()).or(stream_duration_ms);

    let size_bytes = parse_u64_opt(format.size.as_deref());
    let bitrate_bps = parse_u64_opt(format.bit_rate.as_deref());

    Ok(MediaProbe {
        format_name: format.format_name.unwrap_or_else(|| "unknown".to_string()),
        duration_ms,
        size_bytes,
        bitrate_bps,
        video,
        audio,
        subtitle_streams,
        subtitle_tracks,
        chapters,
    })
}

fn is_type(stream: &FfprobeStream, kind: &str) -> bool {
    stream.codec_type.as_deref() == Some(kind)
}

fn is_attached_picture(stream: &FfprobeStream) -> bool {
    stream
        .disposition
        .as_ref()
        .and_then(|disposition| disposition.get("attached_pic"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        != 0
}

fn tag_language(tags: &HashMap<String, String>) -> Option<String> {
    tags.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("language"))
        .and_then(|(_, value)| dvdforge_domain::LanguageCode::from_code(value))
        .map(|language| language.iso_639_1().to_string())
}

fn tag_title(tags: &HashMap<String, String>) -> Option<String> {
    tags.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("title"))
        .map(|(_, value)| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn disposition_flag(stream: &FfprobeStream, key: &str) -> bool {
    stream
        .disposition
        .as_ref()
        .and_then(|disposition| disposition.get(key))
        .is_some_and(|value| {
            value
                .as_bool()
                .unwrap_or_else(|| value.as_i64().is_some_and(|number| number != 0))
        })
}

fn is_bitmap_subtitle_codec(codec: &str) -> bool {
    matches!(
        codec,
        "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub"
    )
}

fn parse_frame_rate(value: Option<&str>) -> Option<FrameRate> {
    let value = value?.trim();
    let (num, den) = value.split_once('/')?;
    let num = num.trim().parse::<u32>().ok()?;
    let den = den.trim().parse::<u32>().ok()?;
    if num == 0 || den == 0 {
        return None;
    }
    Some(FrameRate { num, den })
}

fn parse_duration_ms(value: Option<&str>) -> Option<u64> {
    let value = value?.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("N/A") {
        return None;
    }
    let (seconds, fraction) = value.split_once('.').unwrap_or((value, ""));
    let seconds = seconds.parse::<u64>().ok()?;
    let mut fraction: String = fraction.chars().take(4).collect();
    while fraction.len() < 4 {
        fraction.push('0');
    }
    let fraction = fraction.parse::<u64>().ok()?;
    Some(
        seconds
            .saturating_mul(1000)
            .saturating_add((fraction + 5) / 10),
    )
}

fn parse_u8(value: &str) -> Option<u8> {
    value.trim().parse::<u8>().ok()
}

fn parse_u32(value: &str) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

fn parse_u64(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

fn parse_u64_opt(value: Option<&str>) -> Option<u64> {
    value.and_then(parse_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
    {
      "streams": [
        {
          "index": 0,
          "codec_name": "h264",
          "codec_type": "video",
          "width": 1920,
          "height": 1080,
          "r_frame_rate": "30000/1001",
          "avg_frame_rate": "30000/1001",
          "sample_aspect_ratio": "1:1",
          "display_aspect_ratio": "16:9",
          "pix_fmt": "yuv420p",
          "field_order": "progressive",
          "bits_per_raw_sample": "8",
          "disposition": { "attached_pic": 0 }
        },
        {
          "index": 1,
          "codec_name": "aac",
          "codec_type": "audio",
          "channels": 2,
          "sample_rate": "48000",
          "bit_rate": "192000",
          "tags": { "language": "spa", "title": "Comentario del director" },
          "disposition": { "default": 1, "comment": 1 }
        },
        {
          "index": 2,
          "codec_name": "subrip",
          "codec_type": "subtitle",
          "tags": { "language": "eng", "title": "English" },
          "disposition": { "default": 1, "forced": 0 }
        }
      ],
      "chapters": [
        {
          "start_time": "0.000000",
          "end_time": "60.000000",
          "tags": { "title": "Inicio" }
        },
        {
          "start_time": "60.000000",
          "end_time": "120.500000",
          "tags": { "title": "Segunda parte", "language": "spa" }
        }
      ],
      "format": {
        "format_name": "mov,mp4,m4a,3gp,3g2,mj2",
        "duration": "120.500000",
        "size": "12345678",
        "bit_rate": "8000000"
      }
    }
    "#;

    #[test]
    fn parses_video_audio_and_container_metadata() {
        let probe = parse_probe_json(SAMPLE.as_bytes()).expect("valid probe");
        assert_eq!(probe.format_name, "mov,mp4,m4a,3gp,3g2,mj2");
        assert_eq!(probe.duration_ms, Some(120_500));
        assert_eq!(probe.size_bytes, Some(12_345_678));
        assert_eq!(probe.bitrate_bps, Some(8_000_000));
        assert_eq!(probe.subtitle_streams, 1);

        let video = probe.video.expect("video stream");
        assert_eq!(video.codec, "h264");
        assert_eq!((video.width, video.height), (1920, 1080));
        assert_eq!(
            video.frame_rate,
            FrameRate {
                num: 30_000,
                den: 1_001
            }
        );
        assert_eq!(video.sample_aspect_ratio.as_deref(), Some("1:1"));
        assert_eq!(video.display_aspect_ratio.as_deref(), Some("16:9"));
        assert_eq!(video.bit_depth, Some(8));

        let audio = &probe.audio[0];
        assert_eq!(audio.codec, "aac");
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.bitrate_bps, Some(192_000));
        assert_eq!(audio.language.as_deref(), Some("es"));
        assert_eq!(audio.title.as_deref(), Some("Comentario del director"));
        assert!(audio.default);
        assert!(audio.commentary);

        let subtitle = &probe.subtitle_tracks[0];
        assert_eq!(subtitle.language.as_deref(), Some("en"));
        assert_eq!(subtitle.title.as_deref(), Some("English"));
        assert!(subtitle.default);
        assert!(!subtitle.forced);
        assert!(!subtitle.bitmap);

        assert_eq!(probe.chapters.len(), 2);
        assert_eq!(probe.chapters[0].start_ms, 0);
        assert_eq!(probe.chapters[1].start_ms, 60_000);
        assert_eq!(probe.chapters[1].title, "Segunda parte");
        assert_eq!(probe.chapters[1].language.as_deref(), Some("es"));
    }

    #[test]
    fn bitmap_subtitle_codecs_are_detected() {
        assert!(is_bitmap_subtitle_codec("hdmv_pgs_subtitle"));
        assert!(is_bitmap_subtitle_codec("dvd_subtitle"));
        assert!(is_bitmap_subtitle_codec("dvb_subtitle"));
        assert!(is_bitmap_subtitle_codec("xsub"));
        assert!(!is_bitmap_subtitle_codec("subrip"));
    }

    #[test]
    fn audio_language_tags_are_normalized_through_the_catalog() {
        let mut tags = HashMap::new();
        tags.insert("language".to_string(), "fre".to_string());
        assert_eq!(tag_language(&tags).as_deref(), Some("fr"));
        tags.insert("language".to_string(), "deu".to_string());
        assert_eq!(tag_language(&tags).as_deref(), Some("de"));
        tags.insert("language".to_string(), "JPN".to_string());
        assert_eq!(tag_language(&tags).as_deref(), Some("ja"));
        tags.insert("language".to_string(), "unknown".to_string());
        assert_eq!(tag_language(&tags), None);
        tags.clear();
        assert_eq!(tag_language(&tags), None);
    }

    #[test]
    fn duration_rounds_to_nearest_millisecond() {
        assert_eq!(parse_duration_ms(Some("123.456789")), Some(123_457));
        assert_eq!(parse_duration_ms(Some("0.0004")), Some(0));
        assert_eq!(parse_duration_ms(Some("0.0005")), Some(1));
        assert_eq!(parse_duration_ms(Some("N/A")), None);
    }

    #[test]
    fn frame_rate_rejects_zero_denominator() {
        assert_eq!(parse_frame_rate(Some("0/0")), None);
        assert_eq!(
            parse_frame_rate(Some("25/1")),
            Some(FrameRate { num: 25, den: 1 })
        );
    }

    #[test]
    fn attached_picture_is_not_used_as_video_stream() {
        let json = r#"
        {
          "streams": [
            {
              "index": 0,
              "codec_name": "mjpeg",
              "codec_type": "video",
              "width": 600,
              "height": 600,
              "r_frame_rate": "90000/1",
              "disposition": { "attached_pic": 1 }
            }
          ],
          "format": { "format_name": "mp3", "duration": "10.0" }
        }
        "#;
        let probe = parse_probe_json(json.as_bytes()).expect("valid probe");
        assert!(probe.video.is_none());
        assert_eq!(probe.duration_ms, Some(10_000));
    }
}
