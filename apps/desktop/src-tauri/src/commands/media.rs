use base64::Engine;
use dvdforge_domain::MediaProbe;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use tokio::process::Command;

/// Elementos de GStreamer que WebKitGTK necesita para montar un `<video>`.
/// En macOS/Windows WebKit usa AVFoundation/Media Foundation y no requiere
/// esta comprobación.
const LINUX_PLAYBACK_ELEMENTS: [&str; 2] = ["autoaudiosink", "autovideosink"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSupport {
    pub supported: bool,
    pub platform: String,
    pub missing: Vec<String>,
    pub detail: String,
    pub install_hint: Option<String>,
}

struct MediaServer {
    base_url: String,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

static MEDIA_SERVER: OnceLock<MediaServer> = OnceLock::new();

fn media_server() -> Result<&'static MediaServer, String> {
    if let Some(server) = MEDIA_SERVER.get() {
        return Ok(server);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("No se pudo abrir el servidor de vista previa: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let files = Arc::new(Mutex::new(HashMap::new()));
    let registry = Arc::clone(&files);
    std::thread::Builder::new()
        .name("dvdforge-media-server".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let registry = Arc::clone(&registry);
                let _ = std::thread::Builder::new()
                    .name("dvdforge-media-http".to_string())
                    .spawn(move || {
                        let _ = serve_media_request(stream, &registry);
                    });
            }
        })
        .map_err(|error| format!("No se pudo iniciar el servidor de vista previa: {error}"))?;
    let server = MediaServer {
        base_url: format!("http://127.0.0.1:{port}/media"),
        files,
    };
    let _ = MEDIA_SERVER.set(server);
    MEDIA_SERVER
        .get()
        .ok_or_else(|| "El servidor de vista previa no está disponible.".to_string())
}

/// WebKitGTK no reproduce `<video>` desde el esquema `asset://`; este comando
/// registra el archivo en un servidor HTTP local con soporte de `Range`.
#[tauri::command]
#[specta::specta]
pub fn media_preview_url(video_path: String) -> Result<String, String> {
    let path = resolve_media_path(&video_path)?;
    if !path.is_file() {
        return Err(format!(
            "El vídeo no existe o no es accesible: {}",
            path.display()
        ));
    }
    let server = media_server()?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    server
        .files
        .lock()
        .map_err(|_| "No se pudo registrar la vista previa.".to_string())?
        .insert(token.clone(), path);
    Ok(format!("{}/{}", server.base_url, token))
}

fn serve_media_request(
    mut stream: TcpStream,
    registry: &Arc<Mutex<HashMap<String, PathBuf>>>,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let mut range_header = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range_header = Some(value.trim().to_string());
            }
        }
    }
    if method != "GET" && method != "HEAD" {
        return write_http_status(&mut stream, "405 Method Not Allowed", "Método no permitido");
    }
    let target = target.split('?').next().unwrap_or(target);
    let token = target.strip_prefix("/media/").unwrap_or_default();
    let path = registry
        .lock()
        .ok()
        .and_then(|files| files.get(token).cloned());
    let Some(path) = path else {
        return write_http_status(&mut stream, "404 Not Found", "Vista previa no encontrada");
    };
    let metadata = std::fs::metadata(&path)?;
    let total = metadata.len();
    let (start, end, partial) = match range_header.as_deref() {
        Some(value) => match parse_range_header(value, total) {
            Some((start, end)) => (start, end, true),
            None => {
                let response = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(response.as_bytes())?;
                return Ok(());
            }
        },
        None => (0, total.saturating_sub(1), false),
    };
    let length = if total == 0 {
        0
    } else {
        end.saturating_sub(start).saturating_add(1)
    };
    let mut headers = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n",
        if partial {
            "206 Partial Content"
        } else {
            "200 OK"
        },
        mime_for_path(&path)
    );
    if partial {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    headers.push_str("\r\n");
    stream.write_all(headers.as_bytes())?;
    if method == "HEAD" || length == 0 {
        return Ok(());
    }
    let mut file = std::fs::File::open(&path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = length;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let chunk = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..chunk])?;
        if read == 0 {
            break;
        }
        stream.write_all(&buffer[..read])?;
        remaining = remaining.saturating_sub(read as u64);
    }
    Ok(())
}

fn write_http_status(stream: &mut TcpStream, status: &str, message: &str) -> std::io::Result<()> {
    let body = message.as_bytes();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn parse_range_header(value: &str, total: u64) -> Option<(u64, u64)> {
    let range = value.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (start, end) = range.split_once('-')?;
    if total == 0 {
        return None;
    }
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.max(1);
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= total {
        return None;
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().ok()?.min(total - 1)
    };
    (start <= end).then_some((start, end))
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "ogv" | "ogg" => "video/ogg",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

#[tauri::command]
#[specta::specta]
pub async fn media_probe_file(path: String) -> Result<MediaProbe, String> {
    dvdforge_media::probe(path)
        .await
        .map_err(|error| error.to_string())
}

/// Comprueba si WebKitGTK puede reproducir vídeo. Evita montar `<video>`
/// cuando faltan los sinks de GStreamer, que en WebKitGTK provocan un cuelgue
/// del proceso web en lugar de un error recuperable.
#[tauri::command]
#[specta::specta]
pub async fn media_playback_support() -> PlaybackSupport {
    let platform = std::env::consts::OS;
    if platform != "linux" {
        return playback_support_from_missing(platform, Vec::new());
    }

    let mut missing = Vec::new();
    let mut probe_error = None;
    for element in LINUX_PLAYBACK_ELEMENTS {
        match probe_gst_element(element).await {
            Ok(true) => {}
            Ok(false) => missing.push(element.to_string()),
            Err(error) => {
                missing = LINUX_PLAYBACK_ELEMENTS
                    .iter()
                    .map(|element| (*element).to_string())
                    .collect();
                probe_error = Some(error);
                break;
            }
        }
    }

    let mut support = playback_support_from_missing(platform, missing);
    if let Some(error) = probe_error {
        support.detail = format!("{} {error}", support.detail);
    }
    support
}

async fn probe_gst_element(element: &str) -> Result<bool, String> {
    let result = Command::new(gst_inspect_binary())
        .arg(element)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|error| format!("No se pudo ejecutar gst-inspect-1.0: {error}"))?;
    Ok(result.status.success())
}

fn gst_inspect_binary() -> PathBuf {
    if let Some(custom) = std::env::var_os("DVD_FORGE_GST_INSPECT") {
        return PathBuf::from(custom);
    }
    dvdforge_toolchain::find_binary_in_dirs(
        &["gst-inspect-1.0"],
        std::env::consts::OS,
        &dvdforge_toolchain::default_search_directories(),
    )
    .unwrap_or_else(|| PathBuf::from("gst-inspect-1.0"))
}

fn playback_support_from_missing(platform: &str, missing: Vec<String>) -> PlaybackSupport {
    let linux = platform == "linux";
    let supported = !linux || missing.is_empty();
    let detail = if !linux {
        "La reproducción usa el motor multimedia nativo del sistema.".to_string()
    } else if supported {
        "GStreamer puede reproducir vídeo y audio en el editor.".to_string()
    } else {
        format!(
            "WebKitGTK no puede reproducir vídeo porque faltan elementos de GStreamer: {}.",
            missing.join(", ")
        )
    };
    let install_hint = if linux && !supported {
        Some(
            "Arch/CachyOS: sudo pacman -S gst-plugins-good gst-libav. \
             Debian/Ubuntu: sudo apt install gstreamer1.0-plugins-good gstreamer1.0-libav."
                .to_string(),
        )
    } else {
        None
    };

    PlaybackSupport {
        supported,
        platform: platform.to_string(),
        missing,
        detail,
        install_hint,
    }
}

/// Extrae un fotograma exacto con FFmpeg. Sin `output_png` el fichero se
/// escribe en el directorio temporal de DVD Forge y se devuelve su ruta.
#[tauri::command]
#[specta::specta]
pub async fn media_extract_frame(
    video_path: String,
    timestamp_ms: u64,
    output_png: Option<String>,
) -> Result<String, String> {
    let input = resolve_media_path(&video_path)?;
    if !input.is_file() {
        return Err(format!(
            "El vídeo no existe o no es accesible: {}",
            input.display()
        ));
    }
    let output = match output_png {
        Some(path) => PathBuf::from(path),
        None => {
            frame_cache_dir().join(format!("{}-{}.png", path_fingerprint(&input), timestamp_ms))
        }
    };
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let seconds = format!("{:.3}", timestamp_ms as f64 / 1000.0);
    let args = build_frame_args(&input, &output, &seconds);
    let output_result = Command::new(ffmpeg_binary())
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("No se pudo ejecutar FFmpeg: {error}"))?;
    if !output_result.status.success() {
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        return Err(format!(
            "FFmpeg no pudo extraer el fotograma: {}",
            stderr.trim()
        ));
    }
    if !output.is_file() {
        return Err("FFmpeg terminó sin generar la imagen solicitada.".to_string());
    }
    Ok(output.display().to_string())
}

/// Devuelve un fichero reproducible por `<video>` en WebKitGTK. Los MP4/WebM
/// con códecs compatibles se usan directamente; el resto se remuxea o se
/// transcodifica a un proxy H.264 de baja latencia en el directorio temporal.
#[tauri::command]
#[specta::specta]
pub async fn media_prepare_preview(video_path: String) -> Result<String, String> {
    let input = resolve_media_path(&video_path)?;
    if !input.is_file() {
        return Err(format!(
            "El vídeo no existe o no es accesible: {}",
            input.display()
        ));
    }
    let probe = dvdforge_media::probe(&input)
        .await
        .map_err(|error| format!("No se pudo analizar el vídeo: {error}"))?;
    let codec = probe
        .video
        .as_ref()
        .map(|stream| stream.codec.to_ascii_lowercase())
        .unwrap_or_default();
    let audio_codec = probe
        .audio
        .first()
        .map(|stream| stream.codec.to_ascii_lowercase())
        .unwrap_or_default();
    let extension = input
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if preview_is_direct(&extension, &codec, &audio_codec) {
        return Ok(input.display().to_string());
    }

    let preview_dir = preview_cache_dir();
    std::fs::create_dir_all(&preview_dir)
        .map_err(|error| format!("No se pudo crear la caché de vista previa: {error}"))?;
    let preview = preview_dir.join(format!("{}.mp4", path_fingerprint(&input)));
    if preview.is_file() && std::fs::metadata(&preview).is_ok_and(|meta| meta.len() > 0) {
        return Ok(preview.display().to_string());
    }
    // Remove partial files left by older builds that used `hash.mp4.part`.
    let legacy_partial = preview.with_extension("mp4.part");
    let _ = std::fs::remove_file(&legacy_partial);
    let partial = preview_partial_path(&preview);
    let _ = std::fs::remove_file(&partial);
    let args = build_preview_args(&input, &partial, &codec);
    let result = Command::new(ffmpeg_binary())
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|error| format!("No se pudo ejecutar FFmpeg: {error}"))?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(format!(
            "FFmpeg no pudo preparar la vista previa: {}",
            stderr.trim()
        ));
    }
    std::fs::rename(&partial, &preview).map_err(|error| error.to_string())?;
    Ok(preview.display().to_string())
}

#[tauri::command]
#[specta::specta]
pub fn read_image_data_url(path: String) -> Result<String, String> {
    let resolved = resolve_media_path(&path)?;
    let bytes = std::fs::read(&resolved).map_err(|error| error.to_string())?;
    let mime = if path.to_ascii_lowercase().ends_with(".jpg")
        || path.to_ascii_lowercase().ends_with(".jpeg")
    {
        "image/jpeg"
    } else if path.to_ascii_lowercase().ends_with(".webp") {
        "image/webp"
    } else {
        "image/png"
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn build_frame_args(input: &Path, output: &Path, seconds: &str) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-ss".to_string(),
        seconds.to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-frames:v".to_string(),
        "1".to_string(),
        "-q:v".to_string(),
        "2".to_string(),
        output.display().to_string(),
    ]
}

fn build_preview_args(input: &Path, output: &Path, codec: &str) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-map".to_string(),
        "0:v:0".to_string(),
        "-map".to_string(),
        "0:a:0?".to_string(),
        "-sn".to_string(),
        "-dn".to_string(),
    ];
    if codec == "h264" {
        args.extend(["-c:v".to_string(), "copy".to_string()]);
    } else {
        args.extend([
            "-c:v".to_string(),
            "libx264".to_string(),
            "-preset".to_string(),
            "ultrafast".to_string(),
            "-crf".to_string(),
            "28".to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-vf".to_string(),
            "scale=-2:480".to_string(),
        ]);
    }
    args.extend([
        "-c:a".to_string(),
        "aac".to_string(),
        "-ac".to_string(),
        "2".to_string(),
        "-b:a".to_string(),
        "128k".to_string(),
        "-movflags".to_string(),
        "+faststart".to_string(),
        "-f".to_string(),
        "mp4".to_string(),
        output.display().to_string(),
    ]);
    args
}

/// The partial file must keep an `.mp4` extension so FFmpeg can infer the
/// muxer even before `-f mp4` is applied. The final atomic rename removes the
/// `.part` suffix.
fn preview_partial_path(preview: &Path) -> PathBuf {
    preview.with_extension("part.mp4")
}

fn preview_is_direct(extension: &str, video_codec: &str, audio_codec: &str) -> bool {
    let no_audio = audio_codec.is_empty();
    match extension {
        "mp4" | "m4v" => {
            matches!(video_codec, "h264" | "vp8" | "vp9" | "av1")
                && (no_audio || matches!(audio_codec, "aac" | "mp3"))
        }
        "mov" => {
            matches!(video_codec, "h264") && (no_audio || matches!(audio_codec, "aac" | "mp3"))
        }
        "webm" => {
            matches!(video_codec, "vp8" | "vp9" | "av1")
                && (no_audio || matches!(audio_codec, "opus" | "vorbis"))
        }
        "ogv" | "ogg" => {
            matches!(video_codec, "theora" | "vp8" | "vp9" | "av1")
                && (no_audio || matches!(audio_codec, "opus" | "vorbis"))
        }
        _ => false,
    }
}

fn resolve_media_path(path: &str) -> Result<PathBuf, String> {
    let mut resolved = PathBuf::from(path);
    if resolved.is_relative() {
        if let Some(project_dir) = crate::context::project_dir() {
            resolved = project_dir.join(resolved);
        }
    }
    Ok(resolved)
}

fn ffmpeg_binary() -> PathBuf {
    if let Some(custom) = std::env::var_os("DVD_FORGE_FFMPEG") {
        return PathBuf::from(custom);
    }
    dvdforge_toolchain::find_binary_in_dirs(
        &["ffmpeg"],
        std::env::consts::OS,
        &dvdforge_toolchain::default_search_directories(),
    )
    .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn path_fingerprint(path: &Path) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"dvdforge-preview-v3");
    hasher.update(path.to_string_lossy().as_bytes());
    if let Ok(metadata) = std::fs::metadata(path) {
        hasher.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                hasher.update(duration.as_nanos().to_le_bytes());
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn frame_cache_dir() -> PathBuf {
    std::env::temp_dir().join("dvdforge-frames")
}

fn preview_cache_dir() -> PathBuf {
    std::env::temp_dir().join("dvdforge-preview")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_args_seek_before_input_and_extract_one_frame() {
        let args = build_frame_args(
            Path::new("/tmp/episode.mkv"),
            Path::new("/tmp/frame.png"),
            "12.345",
        );
        let seek = args.iter().position(|arg| arg == "-ss").expect("seek");
        let input = args.iter().position(|arg| arg == "-i").expect("input");
        assert!(seek < input);
        assert_eq!(args[seek + 1], "12.345");
        assert!(args.windows(2).any(|pair| pair == ["-frames:v", "1"]));
    }

    #[test]
    fn preview_is_direct_only_for_supported_containers() {
        assert!(preview_is_direct("mp4", "h264", "aac"));
        assert!(preview_is_direct("mp4", "h264", ""));
        assert!(preview_is_direct("webm", "vp9", "opus"));
        assert!(preview_is_direct("mov", "h264", "aac"));
        assert!(!preview_is_direct("mkv", "h264", "aac"));
        assert!(!preview_is_direct("mp4", "hevc", "aac"));
        assert!(!preview_is_direct("avi", "mpeg2video", "mp3"));
        assert!(!preview_is_direct("mp4", "h264", "ac3"));
        assert!(!preview_is_direct("webm", "vp9", "aac"));
    }

    #[test]
    fn preview_args_remux_h264_and_transcode_other_codecs() {
        let remux = build_preview_args(Path::new("/tmp/in.mkv"), Path::new("/tmp/out.mp4"), "h264");
        assert!(remux.windows(2).any(|pair| pair == ["-c:v", "copy"]));
        let transcode =
            build_preview_args(Path::new("/tmp/in.mkv"), Path::new("/tmp/out.mp4"), "hevc");
        assert!(transcode.windows(2).any(|pair| pair == ["-c:v", "libx264"]));
        assert!(transcode.windows(2).any(|pair| pair == ["-crf", "28"]));
        assert!(transcode
            .windows(2)
            .any(|pair| pair == ["-vf", "scale=-2:480"]));
        assert!(transcode
            .windows(2)
            .any(|pair| pair == ["-movflags", "+faststart"]));
        assert!(transcode.windows(2).any(|pair| pair == ["-f", "mp4"]));
        assert!(transcode.windows(2).any(|pair| pair == ["-ac", "2"]));
        assert!(transcode.windows(2).any(|pair| pair == ["-b:a", "128k"]));
        let partial = preview_partial_path(Path::new("/tmp/abc.mp4"));
        assert_eq!(partial, PathBuf::from("/tmp/abc.part.mp4"));
        assert_eq!(
            partial.extension().and_then(|value| value.to_str()),
            Some("mp4")
        );
    }

    #[test]
    fn cache_paths_are_stable_for_the_same_file() {
        let path = Path::new("/tmp/example.mkv");
        assert_eq!(path_fingerprint(path), path_fingerprint(path));
        assert_eq!(path_fingerprint(path).len(), 64);
    }

    #[test]
    fn playback_support_accepts_linux_with_all_gstreamer_elements() {
        let support = playback_support_from_missing("linux", Vec::new());
        assert!(support.supported);
        assert_eq!(support.platform, "linux");
        assert!(support.missing.is_empty());
        assert!(support.install_hint.is_none());
        assert!(support.detail.contains("GStreamer"));
    }

    #[test]
    fn playback_support_reports_missing_linux_elements_and_hint() {
        let support = playback_support_from_missing("linux", vec!["autoaudiosink".to_string()]);
        assert!(!support.supported);
        assert_eq!(support.missing, vec!["autoaudiosink"]);
        assert!(support.detail.contains("autoaudiosink"));
        let hint = support.install_hint.expect("install hint");
        assert!(hint.contains("gst-plugins-good"));
        assert!(hint.contains("gstreamer1.0-plugins-good"));
    }

    #[test]
    fn playback_support_is_native_off_linux() {
        for platform in ["macos", "windows"] {
            let support = playback_support_from_missing(platform, Vec::new());
            assert!(support.supported);
            assert_eq!(support.platform, platform);
            assert!(support.missing.is_empty());
            assert!(support.install_hint.is_none());
            assert!(support.detail.contains("nativo"));
        }
    }

    #[test]
    fn linux_playback_requires_audio_and_video_sinks() {
        assert_eq!(LINUX_PLAYBACK_ELEMENTS, ["autoaudiosink", "autovideosink"]);
    }

    #[test]
    fn range_header_parses_partial_open_and_suffix_ranges() {
        assert_eq!(parse_range_header("bytes=0-99", 1_000), Some((0, 99)));
        assert_eq!(parse_range_header("bytes=500-", 1_000), Some((500, 999)));
        assert_eq!(parse_range_header("bytes=-100", 1_000), Some((900, 999)));
        assert_eq!(
            parse_range_header("bytes=900-1200", 1_000),
            Some((900, 999))
        );
        assert_eq!(parse_range_header("bytes=1000-", 1_000), None);
        assert_eq!(parse_range_header("bytes=0-99", 0), None);
        assert_eq!(parse_range_header("items=0-99", 1_000), None);
    }

    #[test]
    fn preview_mime_types_cover_common_web_video_containers() {
        assert_eq!(mime_for_path(Path::new("clip.mp4")), "video/mp4");
        assert_eq!(mime_for_path(Path::new("clip.MKV")), "video/x-matroska");
        assert_eq!(mime_for_path(Path::new("clip.webm")), "video/webm");
        assert_eq!(
            mime_for_path(Path::new("clip.bin")),
            "application/octet-stream"
        );
    }
}

#[cfg(test)]
mod proxy_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requiere ffmpeg/ffprobe y los fixtures de vídeo"]
    async fn prepares_a_real_mkv_proxy_without_touching_the_source() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = root.join("fixtures/projects/assets/episode-01.mp4");
        let dir = std::env::temp_dir().join(format!(
            "dvdforge-proxy-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let input = dir.join("clip.mkv");
        let status = Command::new(ffmpeg_binary())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&source)
            .args(["-t", "1", "-c", "copy"])
            .arg(&input)
            .status()
            .await
            .expect("spawn ffmpeg");
        assert!(status.success(), "no se pudo crear el MKV de prueba");

        let source_bytes = std::fs::read(&input).expect("source bytes");
        let prepared = media_prepare_preview(input.display().to_string())
            .await
            .expect("preview");
        assert!(prepared.ends_with(".mp4"), "{prepared}");
        assert!(Path::new(&prepared).is_file());
        assert!(std::fs::metadata(&prepared).unwrap().len() > 0);
        assert_eq!(std::fs::read(&input).unwrap(), source_bytes);
        let again = media_prepare_preview(input.display().to_string())
            .await
            .expect("cached preview");
        assert_eq!(prepared, again);

        let _ = std::fs::remove_file(&prepared);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod proxy_transcode_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requiere ffmpeg/ffprobe y los fixtures de vídeo"]
    async fn transcodes_a_real_avi_to_a_web_playable_proxy() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let source = root.join("fixtures/projects/assets/episode-01.mp4");
        let dir = std::env::temp_dir().join(format!(
            "dvdforge-proxy-avi-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let input = dir.join("clip.avi");
        let status = Command::new(ffmpeg_binary())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&source)
            .args(["-t", "1", "-c:v", "mpeg4", "-c:a", "pcm_s16le"])
            .arg(&input)
            .status()
            .await
            .expect("spawn ffmpeg");
        assert!(status.success(), "no se pudo crear el AVI de prueba");

        let prepared = media_prepare_preview(input.display().to_string())
            .await
            .expect("avi preview");
        assert!(prepared.ends_with(".mp4"), "{prepared}");
        assert!(std::fs::metadata(&prepared).unwrap().len() > 0);
        assert!(input.is_file());

        let _ = std::fs::remove_file(&prepared);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
