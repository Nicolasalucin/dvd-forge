//! AVCHD/BDMV engine: H.264 encoding, AC-3 audio, elementary-stream
//! concatenation, tsMuxeR metafile generation and ISO/folder muxing. Blu-ray
//! capacities switch the metafile to native `--blu-ray` mode and can carry an
//! HDMV Interactive Graphics stream compiled by `dvdforge-hdmv`.

use dvdforge_domain::{
    AspectRatio, AvchdResolution, AvchdVideoConfig, Diagnostic, DiscCapacity, FrameRange,
    FrameRate, VideoScaleMode,
};
use dvdforge_toolchain::{resolve_binary, MEDIA_PROTOCOL_WHITELIST};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

pub mod native;

pub const CRATE_NAME: &str = "dvdforge-avchd";

pub const AVCHD_MAX_VIDEO_KBPS: u32 = 24_000;
pub const BLURAY_MAX_VIDEO_KBPS: u32 = 38_000;
pub const BLURAY_VBV_MIN_KBITS: u32 = 30_000;
pub const BLURAY_VBV_MAX_KBITS: u32 = 40_000;
pub const AVCHD_TS_OVERHEAD_PPM: u32 = 50_000;
pub const AVCHD_HEADER_MARGIN_PPM: u32 = 30_000;
pub const AVCHD_UDF_REVISION: &str = "2.50";
pub const AVCHD_VIDEO_CODEC: &str = "h264";
pub const AVCHD_AUDIO_CODEC: &str = "ac3";
pub const AVCHD_X264_LEVEL: &str = "4.1";
pub const AVCHD_MUX_TIMEOUT: Duration = Duration::from_secs(3_600);
/// A single FFmpeg stage may legitimately run for hours on slow hardware, but
/// an unresponsive process must not block a render forever.
pub const AVCHD_STAGE_TIMEOUT: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionSpec {
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
}

impl ResolutionSpec {
    pub const fn from_resolution(resolution: AvchdResolution) -> Self {
        let (width, height) = resolution.dimensions();
        Self {
            width,
            height,
            interlaced: resolution.interlaced(),
        }
    }

    pub const fn from_config(config: &AvchdVideoConfig) -> Self {
        Self::from_resolution(config.resolution)
    }
}

pub fn header_margin_bytes(capacity: DiscCapacity) -> u64 {
    (capacity.nominal_bytes() as u128 * AVCHD_HEADER_MARGIN_PPM as u128 / 1_000_000) as u64
}

pub fn ts_overhead_bytes(bytes: u64) -> u64 {
    (bytes as u128 * AVCHD_TS_OVERHEAD_PPM as u128 / 1_000_000) as u64
}

pub fn config_errors(config: &AvchdVideoConfig) -> Vec<Diagnostic> {
    config
        .validate()
        .into_iter()
        .filter(Diagnostic::is_error)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvchdAudioCodec {
    Ac3,
    Dts,
    Lpcm,
}

impl AvchdAudioCodec {
    pub const fn meta_name(self) -> &'static str {
        match self {
            Self::Ac3 => "A_AC3",
            Self::Dts => "A_DTS",
            Self::Lpcm => "A_LPCM",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AvchdAudioConfig {
    pub source_stream: u32,
    pub bitrate_kbps: u16,
    pub channels: u8,
    pub sample_rate: u32,
    pub pre_filter: Option<String>,
    pub gain_db: f32,
    pub delay_ms: i32,
    pub duration_ms: u64,
    pub language: String,
    /// Stream-copy an already compatible DTS/LPCM/AC-3 stream.
    pub copy: bool,
    /// Codec declared in the tsMuxeR meta for copy or AC-3 encode.
    pub codec: AvchdAudioCodec,
    pub default: bool,
    pub commentary: bool,
}

/// One bitmap subtitle track passed through to the H.264/AC-3 MKV. Only
/// `hdmv_pgs_subtitle` tracks are valid here; the render layer filters them.
#[derive(Debug, Clone)]
pub struct AvchdSubtitleConfig {
    /// Embedded PGS stream index in the source video.
    pub source_stream: Option<u32>,
    /// External `.sup` file. Exactly one of `source_stream`/`source_path` is set.
    pub source_path: Option<PathBuf>,
    pub language: String,
    pub forced: bool,
    pub default: bool,
    pub delay_ms: i32,
}

#[derive(Debug, Clone)]
pub struct AvchdEncodeRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub stem: String,
    pub config: AvchdVideoConfig,
    pub disc_capacity: DiscCapacity,
    pub aspect: AspectRatio,
    pub video_kbps: u32,
    pub maxrate_kbps: u32,
    pub vbv_bufsize_kbps: u32,
    pub audios: Vec<AvchdAudioConfig>,
    pub subtitles: Vec<AvchdSubtitleConfig>,
    pub trim: Option<FrameRange>,
    pub trim_start_ms: Option<u64>,
    pub trim_end_ms: Option<u64>,
    pub scale_mode: VideoScaleMode,
    pub fps: FrameRate,
    pub duration_ms: u64,
    /// Resolved hardware encoder, or `None` for the CPU/libx264 path.
    pub hardware_encoder: Option<dvdforge_toolchain::HardwareEncoder>,
}

impl AvchdEncodeRequest {
    pub fn stream_mkv(&self) -> PathBuf {
        self.output_dir.join(format!("{}.mkv", self.stem))
    }

    pub fn validate(&self) -> Result<(), AvchdError> {
        if self.stem.trim().is_empty() || self.stem.contains('/') || self.stem.contains('\\') {
            return Err(AvchdError::InvalidConfig(
                "el nombre base de salida no es válido".to_string(),
            ));
        }
        let errors = self.config.validate_for_capacity(self.disc_capacity);
        let errors: Vec<_> = errors.iter().filter(|item| item.is_error()).collect();
        if !errors.is_empty() {
            return Err(AvchdError::InvalidConfig(
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        let max_video = if self.disc_capacity.is_blu_ray() {
            BLURAY_MAX_VIDEO_KBPS
        } else {
            AVCHD_MAX_VIDEO_KBPS
        };
        if self.video_kbps == 0 || self.video_kbps > max_video {
            return Err(AvchdError::InvalidConfig(format!(
                "bitrate de video fuera de rango: {} kbps (máximo {})",
                self.video_kbps, max_video
            )));
        }
        if self.maxrate_kbps < self.video_kbps || self.maxrate_kbps > max_video {
            return Err(AvchdError::InvalidConfig(format!(
                "maxrate inválido: {} kbps (máximo {})",
                self.maxrate_kbps, max_video
            )));
        }
        if self.vbv_bufsize_kbps == 0 {
            return Err(AvchdError::InvalidConfig(
                "el VBV bufsize no puede ser cero".to_string(),
            ));
        }
        if self.duration_ms == 0 {
            return Err(AvchdError::InvalidConfig(
                "la duración AVCHD no puede ser cero".to_string(),
            ));
        }
        for audio in &self.audios {
            if audio.copy {
                continue;
            }
            if !matches!(audio.bitrate_kbps, 192 | 224 | 256 | 320 | 384 | 448) {
                return Err(AvchdError::InvalidConfig(format!(
                    "bitrate AC-3 no soportado: {} kbps",
                    audio.bitrate_kbps
                )));
            }
            if audio.sample_rate != 48_000 {
                return Err(AvchdError::InvalidConfig(
                    "el audio AVCHD debe ser AC-3 a 48 kHz".to_string(),
                ));
            }
            if !matches!(audio.channels, 1 | 2 | 6) {
                return Err(AvchdError::InvalidConfig(
                    "el audio AVCHD admite 1, 2 o 6 canales".to_string(),
                ));
            }
        }
        for subtitle in &self.subtitles {
            if subtitle.source_stream.is_some() == subtitle.source_path.is_some() {
                return Err(AvchdError::InvalidConfig(
                    "cada subtítulo AVCHD debe tener exactamente un origen".to_string(),
                ));
            }
        }
        if let Some(trim) = self.trim {
            if !trim.is_valid() {
                return Err(AvchdError::InvalidConfig(
                    "el rango de recorte no es válido".to_string(),
                ));
            }
            if !self.fps.is_valid() {
                return Err(AvchdError::InvalidConfig(
                    "el framerate del recorte no es válido".to_string(),
                ));
            }
        }
        if let (Some(start), Some(end)) = (self.trim_start_ms, self.trim_end_ms) {
            if end <= start {
                return Err(AvchdError::InvalidConfig(
                    "el fin del recorte debe ser posterior al inicio".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct AvchdOutputs {
    pub stream_mkv: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EncodeProgress {
    pub stage: &'static str,
    pub out_time_us: u64,
    pub speed_x: Option<f32>,
    pub fps: Option<f32>,
}

pub type EncodeProgressSink = mpsc::Sender<EncodeProgress>;

#[derive(Debug, Error)]
pub enum AvchdError {
    #[error("configuración AVCHD inválida: {0}")]
    InvalidConfig(String),
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("FFmpeg falló en {stage} con código {code}: {stderr}")]
    Ffmpeg {
        stage: &'static str,
        code: i32,
        stderr: String,
    },
    #[error("tsMuxeR falló con código {code}: {stderr}")]
    TsMuxer { code: i32, stderr: String },
    #[error("tsMuxeR no generó la salida esperada: {0}")]
    MissingOutput(String),
    #[error("la operación AVCHD excedió el tiempo máximo")]
    Timeout,
}

#[derive(Debug, Clone)]
pub struct AvchdEncoder {
    pub ffmpeg: PathBuf,
}

impl Default for AvchdEncoder {
    fn default() -> Self {
        Self {
            ffmpeg: resolve_binary("ffmpeg"),
        }
    }
}

impl AvchdEncoder {
    pub fn new(ffmpeg: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
        }
    }

    pub async fn encode(&self, request: &AvchdEncodeRequest) -> Result<AvchdOutputs, AvchdError> {
        self.encode_with_progress(request, None).await
    }

    pub async fn encode_with_progress(
        &self,
        request: &AvchdEncodeRequest,
        progress: Option<EncodeProgressSink>,
    ) -> Result<AvchdOutputs, AvchdError> {
        request.validate()?;
        std::fs::create_dir_all(&request.output_dir)?;

        let args = build_encode_args(request);
        run_stage(&self.ffmpeg, &args, "avchd", progress.as_ref()).await?;

        Ok(AvchdOutputs {
            stream_mkv: request.stream_mkv(),
        })
    }
}

/// Appends a local file input with an explicit protocol whitelist so crafted
/// playlists or IPC paths cannot make FFmpeg open remote resources.
fn push_file_input(args: &mut Vec<String>, path: &Path) {
    args.extend([
        "-protocol_whitelist".to_string(),
        MEDIA_PROTOCOL_WHITELIST.to_string(),
        "-i".to_string(),
        path.display().to_string(),
    ]);
}

fn audio_filter_chain(audio: &AvchdAudioConfig) -> Option<String> {
    let mut filters = Vec::new();
    if let Some(pre_filter) = &audio.pre_filter {
        if !pre_filter.trim().is_empty() {
            filters.push(pre_filter.clone());
        }
    }
    if audio.gain_db.abs() > 0.01 {
        filters.push(format!("volume={:.3}dB", audio.gain_db));
    }
    if audio.delay_ms > 0 {
        filters.push(format!("adelay={}:all=1", audio.delay_ms));
    } else if audio.delay_ms < 0 {
        let mut filter = format!(
            "atrim=start={:.3},asetpts=PTS-STARTPTS",
            audio.delay_ms.unsigned_abs() as f64 / 1000.0
        );
        if audio.duration_ms > 0 {
            filter.push_str(&format!(
                ",apad=whole_dur={:.3}",
                audio.duration_ms as f64 / 1000.0
            ));
        }
        filters.push(filter);
    }
    (!filters.is_empty()).then(|| filters.join(","))
}

pub fn build_encode_args(request: &AvchdEncodeRequest) -> Vec<String> {
    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    if request.trim_start_ms.is_some() || request.trim_end_ms.is_some() {
        if let Some(start) = request.trim_start_ms {
            if start > 0 {
                args.push("-ss".to_string());
                args.push(format_millis(start));
            }
        }
    } else if let Some(trim) = request.trim {
        args.push("-ss".to_string());
        args.push(format_seconds(trim.start, request.fps));
    }
    push_file_input(&mut args, &request.input);
    if request.audios.is_empty() {
        args.extend([
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "anullsrc=r=48000:cl=stereo".to_string(),
        ]);
    }
    let mut external_inputs = Vec::new();
    let mut next_input = if request.audios.is_empty() {
        2usize
    } else {
        1usize
    };
    for subtitle in &request.subtitles {
        if let Some(path) = &subtitle.source_path {
            if subtitle.delay_ms != 0 {
                args.extend([
                    "-itsoffset".to_string(),
                    format!("{:.3}", f64::from(subtitle.delay_ms) / 1000.0),
                ]);
            }
            args.extend(["-f".to_string(), "sup".to_string()]);
            push_file_input(&mut args, path);
            external_inputs.push(next_input);
            next_input += 1;
        }
    }
    let duration_ms = if request.trim_end_ms.is_some() {
        let start = request.trim_start_ms.unwrap_or(0);
        request
            .trim_end_ms
            .unwrap_or(request.duration_ms)
            .saturating_sub(start)
            .max(1)
    } else {
        let base = request.duration_ms.max(1);
        match request.trim {
            Some(trim) if request.fps.is_valid() => {
                let trim_ms = ((trim.duration() as f64 * 1000.0) / request.fps.as_f64())
                    .round()
                    .max(1.0) as u64;
                base.min(trim_ms).max(1)
            }
            _ => base,
        }
    };
    args.extend(["-t".to_string(), format_millis(duration_ms)]);

    let spec = ResolutionSpec::from_config(&request.config);
    let mut filter = build_filter(request, spec);
    let keyint = request.config.keyint_frames.max(1);
    let maxrate = request.maxrate_kbps.max(request.video_kbps);
    let bufsize = request.vbv_bufsize_kbps.max(1);
    let hardware = request.hardware_encoder;
    let codec = hardware
        .map(dvdforge_toolchain::HardwareEncoder::ffmpeg_name)
        .unwrap_or("libx264");
    if hardware == Some(dvdforge_toolchain::HardwareEncoder::VaapiH264) {
        filter.push_str(",format=nv12,hwupload");
    }
    args.extend([
        "-vf".to_string(),
        filter,
        "-c:v".to_string(),
        codec.to_string(),
    ]);
    if let Some(hardware) = hardware {
        if hardware == dvdforge_toolchain::HardwareEncoder::NvencH264 {
            args.extend(["-preset".to_string(), "p5".to_string()]);
        } else if hardware == dvdforge_toolchain::HardwareEncoder::QsvH264 {
            args.extend(["-preset".to_string(), "medium".to_string()]);
        }
        args.extend([
            "-profile:v".to_string(),
            "high".to_string(),
            "-level:v".to_string(),
            AVCHD_X264_LEVEL.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-b:v".to_string(),
            format!("{}k", request.video_kbps),
            "-maxrate".to_string(),
            format!("{maxrate}k"),
            "-bufsize".to_string(),
            format!("{bufsize}k"),
            "-g".to_string(),
            keyint.to_string(),
            "-bf".to_string(),
            "0".to_string(),
        ]);
    } else {
        let mut x264 = format!(
            "bluray-compat=1:open-gop=0:slices=4:nal-hrd=vbr:vbv-maxrate={maxrate}:vbv-bufsize={bufsize}:aud=1:colorprim=bt709:transfer=bt709:colormatrix=bt709:keyint={keyint}:min-keyint={keyint}:scenecut=0"
        );
        if spec.interlaced {
            x264.push_str(":tff=1");
        }
        args.extend([
            "-preset".to_string(),
            "medium".to_string(),
            "-profile:v".to_string(),
            "high".to_string(),
            "-level:v".to_string(),
            AVCHD_X264_LEVEL.to_string(),
            "-pix_fmt".to_string(),
            "yuv420p".to_string(),
            "-b:v".to_string(),
            format!("{}k", request.video_kbps),
            "-maxrate".to_string(),
            format!("{maxrate}k"),
            "-bufsize".to_string(),
            format!("{bufsize}k"),
            "-g".to_string(),
            keyint.to_string(),
            "-bf".to_string(),
            "0".to_string(),
            "-x264-params".to_string(),
            x264,
        ]);
    }
    args.extend([
        "-color_primaries".to_string(),
        "bt709".to_string(),
        "-color_trc".to_string(),
        "bt709".to_string(),
        "-colorspace".to_string(),
        "bt709".to_string(),
    ]);
    if spec.interlaced {
        args.extend([
            "-flags".to_string(),
            "+ildct+ilme".to_string(),
            "-top".to_string(),
            "1".to_string(),
        ]);
    }

    args.extend(["-map".to_string(), "0:v:0".to_string()]);
    if request.audios.is_empty() {
        args.extend(["-map".to_string(), "1:a:0".to_string()]);
    } else {
        for audio in &request.audios {
            args.extend(["-map".to_string(), format!("0:{}", audio.source_stream)]);
        }
    }
    let mut external_cursor = 0usize;
    for subtitle in &request.subtitles {
        let source = if let Some(stream) = subtitle.source_stream {
            format!("0:{stream}")
        } else {
            let input = external_inputs.get(external_cursor).copied().unwrap_or(1);
            external_cursor += 1;
            format!("{input}:s:0")
        };
        args.extend(["-map".to_string(), source]);
    }
    if request.audios.is_empty() {
        args.extend([
            "-c:a".to_string(),
            "ac3".to_string(),
            "-b:a".to_string(),
            format!("{AVCHD_DEFAULT_AUDIO_KBPS}k"),
            "-ar".to_string(),
            "48000".to_string(),
            "-ac".to_string(),
            "2".to_string(),
        ]);
    } else {
        for (index, audio) in request.audios.iter().enumerate() {
            if audio.copy {
                args.extend([format!("-c:a:{index}"), "copy".to_string()]);
            } else {
                args.extend([
                    format!("-c:a:{index}"),
                    "ac3".to_string(),
                    format!("-b:a:{index}"),
                    format!("{}k", audio.bitrate_kbps),
                    format!("-ar:a:{index}"),
                    audio.sample_rate.to_string(),
                    format!("-ac:a:{index}"),
                    audio.channels.to_string(),
                ]);
                if let Some(filters) = audio_filter_chain(audio) {
                    args.extend([format!("-filter:a:{index}"), filters]);
                }
            }
            args.extend([
                format!("-metadata:s:a:{index}"),
                format!("language={}", normalize_language(&audio.language)),
            ]);
            if audio.default {
                args.extend([format!("-disposition:a:{index}"), "default".to_string()]);
            }
            if audio.commentary {
                args.extend([format!("-disposition:a:{index}"), "comment".to_string()]);
            }
        }
    }
    if request.subtitles.is_empty() {
        args.push("-sn".to_string());
    } else {
        args.push("-c:s".to_string());
        args.push("copy".to_string());
        for (index, subtitle) in request.subtitles.iter().enumerate() {
            args.push(format!("-metadata:s:s:{index}"));
            args.push(format!(
                "language={}",
                normalize_language(&subtitle.language)
            ));
            if subtitle.forced || subtitle.default {
                let mut disposition = Vec::new();
                if subtitle.default {
                    disposition.push("default");
                }
                if subtitle.forced {
                    disposition.push("forced");
                }
                args.push(format!("-disposition:s:{index}"));
                args.push(disposition.join("+"));
            }
        }
    }
    args.extend([
        "-dn".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "matroska".to_string(),
        request.stream_mkv().display().to_string(),
    ]);
    args
}

pub const AVCHD_DEFAULT_AUDIO_KBPS: u16 = 192;

fn build_filter(request: &AvchdEncodeRequest, spec: ResolutionSpec) -> String {
    let interlaced = if spec.interlaced { ":interl=1" } else { "" };
    let sar = sar_for(request.config.resolution, request.aspect);
    let fps = format!(
        "fps={}/{}",
        request.config.frame_rate.num, request.config.frame_rate.den
    );
    match request.scale_mode {
        VideoScaleMode::Letterbox | VideoScaleMode::Contain => format!(
            "scale=iw*sar:ih,setsar=1,\
scale={}:{}:force_original_aspect_ratio=decrease:flags=lanczos{},\
pad={}:{}:(ow-iw)/2:(oh-ih)/2,\
setsar={}:{},{},format=yuv420p",
            spec.width, spec.height, interlaced, spec.width, spec.height, sar.0, sar.1, fps
        ),
        VideoScaleMode::CropFill | VideoScaleMode::Cover => format!(
            "scale=iw*sar:ih,setsar=1,\
scale={}:{}:force_original_aspect_ratio=increase:flags=lanczos{},\
crop={}:{},setsar={}:{},{},format=yuv420p",
            spec.width, spec.height, interlaced, spec.width, spec.height, sar.0, sar.1, fps
        ),
        VideoScaleMode::Stretch => format!(
            "scale=iw*sar:ih,setsar=1,scale={}:{}:flags=lanczos{},\
setsar={}:{},{},format=yuv420p",
            spec.width, spec.height, interlaced, sar.0, sar.1, fps
        ),
    }
}

fn format_millis(millis: u64) -> String {
    format!("{:.3}", millis as f64 / 1000.0)
}

fn sar_for(resolution: AvchdResolution, aspect: AspectRatio) -> (u32, u32) {
    match resolution {
        AvchdResolution::R1080p | AvchdResolution::R1080i | AvchdResolution::R720p => (1, 1),
        AvchdResolution::Sd480i => {
            if aspect == AspectRatio::SixteenNine {
                (32, 27)
            } else {
                (8, 9)
            }
        }
        AvchdResolution::Sd576i => {
            if aspect == AspectRatio::SixteenNine {
                (64, 45)
            } else {
                (16, 15)
            }
        }
    }
}

fn format_seconds(frames: u64, fps: FrameRate) -> String {
    format!("{:.6}", frames as f64 / fps.as_f64())
}

async fn run_stage(
    ffmpeg: &Path,
    args: &[String],
    stage: &'static str,
    progress: Option<&EncodeProgressSink>,
) -> Result<(), AvchdError> {
    let mut child = Command::from(dvdforge_toolchain::background_command(ffmpeg))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AvchdError::InvalidConfig("no se pudo capturar stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| AvchdError::InvalidConfig("no se pudo capturar stderr".to_string()))?;
    let (progress_task, drain_task) = if let Some(sink) = progress {
        let sink = sink.clone();
        (
            Some(tokio::spawn(read_encode_progress(stdout, stage, sink))),
            None,
        )
    } else {
        (None, Some(tokio::spawn(drain_stdout(stdout))))
    };
    let stderr_task = tokio::spawn(read_stderr(stderr));
    let status = match timeout(AVCHD_STAGE_TIMEOUT, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            let _ = child.kill().await;
            if let Some(task) = &progress_task {
                task.abort();
            }
            if let Some(task) = &drain_task {
                task.abort();
            }
            stderr_task.abort();
            return Err(AvchdError::Timeout);
        }
    };
    if let Some(task) = progress_task {
        let _ = task.await;
    }
    if let Some(task) = drain_task {
        let _ = task.await;
    }
    let stderr_log = stderr_task.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(AvchdError::Ffmpeg {
            stage,
            code: status.code().unwrap_or(-1),
            stderr: tail(&stderr_log, 4_000),
        })
    }
}

async fn read_encode_progress(stdout: ChildStdout, stage: &'static str, sink: EncodeProgressSink) {
    let mut lines = BufReader::new(stdout).lines();
    let mut out_time_us = 0u64;
    let mut speed_x = None;
    let mut fps = None;
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(value) = line.strip_prefix("out_time_us=") {
            out_time_us = value.trim().parse().unwrap_or(out_time_us);
        } else if let Some(value) = line.strip_prefix("speed=") {
            speed_x = value.trim().trim_end_matches('x').parse::<f32>().ok();
        } else if let Some(value) = line.strip_prefix("fps=") {
            fps = value.trim().parse::<f32>().ok();
        } else if line.starts_with("progress=") {
            let _ = sink
                .send(EncodeProgress {
                    stage,
                    out_time_us,
                    speed_x,
                    fps,
                })
                .await;
        }
    }
}

async fn drain_stdout(mut stdout: ChildStdout) {
    let mut buffer = [0u8; 4096];
    while let Ok(read) = stdout.read(&mut buffer).await {
        if read == 0 {
            break;
        }
    }
}

async fn read_stderr(mut stderr: ChildStderr) -> String {
    let mut buffer = Vec::new();
    let _ = stderr.read_to_end(&mut buffer).await;
    String::from_utf8_lossy(&buffer).into_owned()
}

fn tail(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        value.to_string()
    } else {
        value.chars().skip(count - max_chars).collect()
    }
}

/// Concatenates intermediate Matroska files with a stream copy. The combined
/// file keeps continuous timestamps for tsMuxeR.
pub async fn concat_mkv(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    output: &Path,
) -> Result<(), AvchdError> {
    if inputs.is_empty() {
        return Err(AvchdError::InvalidConfig(
            "no hay streams para concatenar".to_string(),
        ));
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if inputs.len() == 1 {
        if inputs[0].as_path() == output {
            return Ok(());
        }
        // tsMuxeR only reads the stream: hard-link it instead of copying many
        // GB; fall back to a copy across volumes.
        if output.exists() {
            std::fs::remove_file(output)?;
        }
        if std::fs::hard_link(&inputs[0], output).is_err() {
            std::fs::copy(&inputs[0], output)?;
        }
        return Ok(());
    }
    let list_path = output.with_extension("concat.txt");
    let list = inputs
        .iter()
        .map(|path| {
            let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            format!(
                "file '{}'
",
                absolute.to_string_lossy().replace('\'', "'\\''")
            )
        })
        .collect::<String>();
    std::fs::write(&list_path, list)?;
    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-f".to_string(),
        "concat".to_string(),
        "-safe".to_string(),
        "0".to_string(),
    ];
    push_file_input(&mut args, &list_path);
    args.extend([
        "-map".to_string(),
        "0".to_string(),
        "-c".to_string(),
        "copy".to_string(),
        "-f".to_string(),
        "matroska".to_string(),
        output.display().to_string(),
    ]);
    run_stage(ffmpeg, &args, "matroska concat", None).await?;
    let _ = std::fs::remove_file(&list_path);
    if !output.is_file() || output.metadata()?.len() == 0 {
        return Err(AvchdError::MissingOutput(output.display().to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub struct AvchdMetaSubtitle<'a> {
    /// Path tsMuxeR can open; `combined.mkv` for embedded PGS tracks.
    pub source: &'a str,
    pub track: u32,
    pub language: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct AvchdMetaAudio<'a> {
    pub source: &'a str,
    pub track: u32,
    pub language: &'a str,
    pub codec: AvchdAudioCodec,
}

#[derive(Debug, Clone)]
pub struct AvchdMetaRequest<'a> {
    pub video: &'a str,
    pub audios: &'a [AvchdMetaAudio<'a>],
    pub video_track: u32,
    pub fps: FrameRate,
    /// Chapter start timestamps in milliseconds, relative to the final track.
    pub chapters: &'a [u64],
    pub capacity: DiscCapacity,
    pub maxrate_kbps: u32,
    pub vbv_bufsize_kbps: u32,
    /// Flujo HDMV IG compilado (PES). tsMuxeR lo recibe como pista de
    /// gráficos; si el muxer no lo soporta, el render reintenta sin él.
    pub interactive_graphics: Option<&'a str>,
    /// PGS tracks carried by `video`/external `.sup` inputs.
    pub subtitles: &'a [AvchdMetaSubtitle<'a>],
}

pub fn build_meta(request: &AvchdMetaRequest<'_>) -> String {
    let mut chapters: Vec<u64> = request.chapters.to_vec();
    chapters.sort_unstable();
    chapters.dedup();
    if chapters.first().copied().unwrap_or(1) != 0 {
        chapters.insert(0, 0);
    }
    let chapter_list = chapters
        .iter()
        .map(|millis| format_chapter(*millis))
        .collect::<Vec<_>>()
        .join(";");
    let header = if request.capacity.is_blu_ray() {
        "MUXOPT --blu-ray"
    } else {
        "MUXOPT --avchd"
    };
    let mut meta = format!(
        "{header} --vbv-bufsize={} --vbv-maxrate={} --custom-chapters={}\n",
        request.vbv_bufsize_kbps, request.maxrate_kbps, chapter_list
    );
    meta.push_str(&format!(
        "V_MPEG4/ISO/AVC, {}, track={}, fps={:.3}, insertSEI, contSPS\n",
        request.video,
        request.video_track,
        request.fps.as_f64()
    ));
    for audio in request.audios {
        meta.push_str(&format!(
            "{}, {}, track={}, lang={}\n",
            audio.codec.meta_name(),
            audio.source,
            audio.track,
            normalize_language(audio.language)
        ));
    }
    if let Some(graphics) = request.interactive_graphics {
        meta.push_str(&format!("S_HDMV/PGS, {graphics}, track=1, lang=und\n"));
    }
    for subtitle in request.subtitles {
        meta.push_str(&format!(
            "S_HDMV/PGS, {}, track={}, lang={}\n",
            subtitle.source,
            subtitle.track,
            normalize_language(subtitle.language)
        ));
    }
    meta
}

pub fn write_meta(path: &Path, request: &AvchdMetaRequest<'_>) -> Result<(), AvchdError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, build_meta(request))?;
    Ok(())
}

fn format_chapter(millis: u64) -> String {
    let total_seconds = millis / 1000;
    let milliseconds = millis % 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}")
}

fn normalize_language(language: &str) -> String {
    dvdforge_domain::LanguageCode::from_code(language)
        .unwrap_or(dvdforge_domain::LanguageCode::Undetermined)
        .iso_639_2()
        .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TsMuxerTargetKind {
    Iso,
    Folder,
}

#[derive(Debug, Clone)]
pub struct TsMuxerReport {
    pub path: PathBuf,
    pub kind: TsMuxerTargetKind,
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct TsMuxer {
    pub binary: PathBuf,
}

impl Default for TsMuxer {
    fn default() -> Self {
        Self {
            binary: {
                let primary = resolve_binary("tsMuxeR");
                if primary.components().count() > 1 {
                    primary
                } else {
                    resolve_binary("tsmuxer")
                }
            },
        }
    }
}

impl TsMuxer {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub async fn mux_iso(
        &self,
        meta: &Path,
        output: &Path,
        work_dir: &Path,
    ) -> Result<TsMuxerReport, AvchdError> {
        self.mux(meta, output, work_dir, TsMuxerTargetKind::Iso)
            .await
    }

    pub async fn mux_folder(
        &self,
        meta: &Path,
        output: &Path,
        work_dir: &Path,
    ) -> Result<TsMuxerReport, AvchdError> {
        self.mux(meta, output, work_dir, TsMuxerTargetKind::Folder)
            .await
    }

    async fn mux(
        &self,
        meta: &Path,
        output: &Path,
        work_dir: &Path,
        kind: TsMuxerTargetKind,
    ) -> Result<TsMuxerReport, AvchdError> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match kind {
            TsMuxerTargetKind::Iso => {
                if output.exists() {
                    std::fs::remove_file(output)?;
                }
            }
            TsMuxerTargetKind::Folder => {
                if output.exists() {
                    std::fs::remove_dir_all(output)?;
                }
                std::fs::create_dir_all(output)?;
            }
        }
        let child = Command::from(dvdforge_toolchain::background_command(&self.binary))
            .arg(meta)
            .arg(output)
            .current_dir(work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let result = timeout(AVCHD_MUX_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| AvchdError::Timeout)??;
        if !result.status.success() {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);
            let log = format!("{stderr}\n{stdout}");
            return Err(AvchdError::TsMuxer {
                code: result.status.code().unwrap_or(-1),
                stderr: tail(log.trim(), 4_000),
            });
        }
        match kind {
            TsMuxerTargetKind::Iso => {
                if !output.is_file() || output.metadata()?.len() == 0 {
                    return Err(AvchdError::MissingOutput(output.display().to_string()));
                }
            }
            TsMuxerTargetKind::Folder => {
                if !output.join("BDMV").join("index.bdmv").is_file() {
                    return Err(AvchdError::MissingOutput(format!(
                        "{} (BDMV/index.bdmv)",
                        output.display()
                    )));
                }
                std::fs::create_dir_all(output.join("CERTIFICATE").join("BACKUP"))?;
            }
        }
        let size_bytes = match kind {
            TsMuxerTargetKind::Iso => output.metadata()?.len(),
            TsMuxerTargetKind::Folder => directory_size(output)?,
        };
        Ok(TsMuxerReport {
            path: output.to_path_buf(),
            kind,
            size_bytes,
        })
    }
}

fn directory_size(path: &Path) -> Result<u64, AvchdError> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AvchdVideoConfig {
        AvchdVideoConfig {
            resolution: AvchdResolution::R720p,
            frame_rate: FrameRate::FPS_23_976,
            ..AvchdVideoConfig::default()
        }
    }

    fn request() -> AvchdEncodeRequest {
        AvchdEncodeRequest {
            input: PathBuf::from("input.mp4"),
            output_dir: PathBuf::from("out"),
            stem: "episode-01".to_string(),
            config: config(),
            disc_capacity: DiscCapacity::Dvd5,
            aspect: AspectRatio::SixteenNine,
            video_kbps: 8_000,
            maxrate_kbps: 24_000,
            vbv_bufsize_kbps: 24_000,
            audios: vec![AvchdAudioConfig {
                source_stream: 1,
                bitrate_kbps: 192,
                channels: 2,
                sample_rate: 48_000,
                pre_filter: None,
                gain_db: 0.0,
                delay_ms: 0,
                duration_ms: 0,
                language: "eng".to_string(),
                copy: false,
                codec: AvchdAudioCodec::Ac3,
                default: true,
                commentary: false,
            }],
            subtitles: Vec::new(),
            trim: None,
            trim_start_ms: None,
            trim_end_ms: None,
            scale_mode: VideoScaleMode::Letterbox,
            fps: FrameRate::NTSC,
            duration_ms: 20_000,
            hardware_encoder: None,
        }
    }

    #[test]
    fn video_args_include_required_bluray_flags() {
        let args = build_encode_args(&request()).join(" ");
        assert!(args.contains("bluray-compat=1"));
        assert!(args.contains("open-gop=0"));
        assert!(args.contains("slices=4"));
        assert!(args.contains("nal-hrd=vbr"));
        assert!(args.contains("vbv-maxrate=24000"));
        assert!(args.contains("vbv-bufsize=24000"));
        assert!(args.contains("aud=1"));
        assert!(args.contains("colorprim=bt709"));
        assert!(args.contains("-bf 0"));
        assert!(args.contains("-profile:v high"));
        assert!(args.contains("-level:v 4.1"));
        assert!(args.contains("-f matroska"));
        assert!(args.contains("-c:a:0 ac3"));
    }

    #[test]
    fn interlaced_configs_add_tff_flags() {
        let mut request = request();
        request.config.resolution = AvchdResolution::R1080i;
        request.config.frame_rate = FrameRate::FPS_29_97;
        let args = build_encode_args(&request).join(" ");
        assert!(args.contains("tff=1"));
        assert!(args.contains("+ildct+ilme"));
        assert!(args.contains("-top 1"));
    }

    #[test]
    fn hardware_encoder_switches_codec_and_keeps_bluray_vbv_args() {
        let mut request = request();
        request.hardware_encoder = Some(dvdforge_toolchain::HardwareEncoder::NvencH264);
        let args = build_encode_args(&request).join(" ");
        assert!(args.contains("-c:v h264_nvenc"));
        assert!(!args.contains("libx264"));
        assert!(args.contains("-profile:v high"));
        assert!(args.contains("-maxrate 24000k"));
        assert!(args.contains("-bufsize 24000k"));
        assert!(args.contains("-g 24"));
    }

    #[test]
    fn pgs_tracks_are_mapped_and_copied_into_the_mkv() {
        let mut request = request();
        request.subtitles = vec![AvchdSubtitleConfig {
            source_stream: Some(3),
            source_path: None,
            language: "spa".to_string(),
            forced: false,
            default: true,
            delay_ms: 0,
        }];
        let args = build_encode_args(&request).join(" ");
        assert!(args.contains("-map 0:3"), "{args}");
        assert!(args.contains("-c:s copy"), "{args}");
        assert!(!args.contains("-sn"), "{args}");
        assert!(args.contains("-metadata:s:s:0 language=spa"), "{args}");
        assert!(args.contains("-disposition:s:0 default"), "{args}");
    }

    #[test]
    fn multiple_audio_tracks_are_mapped_and_copied_per_stream() {
        let mut request = request();
        request.audios.push(AvchdAudioConfig {
            source_stream: 2,
            bitrate_kbps: 768,
            channels: 6,
            sample_rate: 48_000,
            pre_filter: None,
            gain_db: 0.0,
            delay_ms: 0,
            duration_ms: 0,
            language: "spa".to_string(),
            copy: true,
            codec: AvchdAudioCodec::Dts,
            default: false,
            commentary: false,
        });
        let args = build_encode_args(&request).join(" ");
        assert!(args.contains("-map 0:1"), "{args}");
        assert!(args.contains("-map 0:2"), "{args}");
        assert!(args.contains("-c:a:0 ac3"), "{args}");
        assert!(args.contains("-c:a:1 copy"), "{args}");
        assert!(args.contains("-metadata:s:a:1 language=spa"), "{args}");
        let audio = [
            AvchdMetaAudio {
                source: "combined.mkv",
                track: 2,
                language: "eng",
                codec: AvchdAudioCodec::Ac3,
            },
            AvchdMetaAudio {
                source: "combined.mkv",
                track: 3,
                language: "spa",
                codec: AvchdAudioCodec::Dts,
            },
        ];
        let meta = build_meta(&AvchdMetaRequest {
            video: "combined.mkv",
            audios: &audio,
            video_track: 1,
            fps: FrameRate::FPS_23_976,
            chapters: &[0],
            capacity: DiscCapacity::Dvd5,
            maxrate_kbps: 24_000,
            vbv_bufsize_kbps: 24_000,
            interactive_graphics: None,
            subtitles: &[],
        });
        assert!(
            meta.contains("A_AC3, combined.mkv, track=2, lang=eng"),
            "{meta}"
        );
        assert!(
            meta.contains("A_DTS, combined.mkv, track=3, lang=spa"),
            "{meta}"
        );
    }

    #[test]
    fn audio_gain_and_delay_are_applied() {
        let mut request = request();
        if let Some(audio) = request.audios.first_mut() {
            audio.gain_db = 3.0;
            audio.delay_ms = 250;
        }
        let args = build_encode_args(&request).join(" ");
        assert!(
            args.contains("-filter:a:0 volume=3.000dB,adelay=250:all=1"),
            "{args}"
        );
    }

    #[test]
    fn meta_contains_pgs_passthrough_tracks() {
        let audio = [AvchdMetaAudio {
            source: "combined.mkv",
            track: 2,
            language: "eng",
            codec: AvchdAudioCodec::Ac3,
        }];
        let meta = build_meta(&AvchdMetaRequest {
            video: "combined.mkv",
            audios: &audio,
            video_track: 1,
            fps: FrameRate::FPS_23_976,
            chapters: &[0],
            capacity: DiscCapacity::Bd25,
            maxrate_kbps: 38_000,
            vbv_bufsize_kbps: 35_000,
            interactive_graphics: None,
            subtitles: &[AvchdMetaSubtitle {
                source: "combined.mkv",
                track: 3,
                language: "spa",
            }],
        });
        assert!(
            meta.contains("S_HDMV/PGS, combined.mkv, track=3, lang=spa"),
            "{meta}"
        );
    }

    #[test]
    fn meta_contains_avchd_header_and_sorted_chapters() {
        let audio = [AvchdMetaAudio {
            source: "combined.mkv",
            track: 2,
            language: "spa",
            codec: AvchdAudioCodec::Ac3,
        }];
        let meta = build_meta(&AvchdMetaRequest {
            video: "combined.mkv",
            audios: &audio,
            video_track: 1,
            fps: FrameRate::FPS_23_976,
            chapters: &[20_000, 0, 20_000, 5_000],
            capacity: DiscCapacity::Dvd5,
            maxrate_kbps: 24_000,
            vbv_bufsize_kbps: 24_000,
            interactive_graphics: None,
            subtitles: &[],
        });
        assert!(meta.starts_with("MUXOPT --avchd --vbv-bufsize=24000 --vbv-maxrate=24000"));
        assert!(meta.contains("--custom-chapters=00:00:00.000;00:00:05.000;00:00:20.000"));
        assert!(meta.contains("V_MPEG4/ISO/AVC, combined.mkv, track=1"));
        assert!(meta.contains("A_AC3, combined.mkv, track=2, lang=spa"));
    }

    #[test]
    fn meta_normalizes_iso_639_1_and_bibliographic_codes() {
        let chapters = [0];
        for (input, expected) in [
            ("es", "spa"),
            ("eng", "eng"),
            ("fr", "fra"),
            ("fre", "fra"),
            ("de", "deu"),
            ("ger", "deu"),
            ("ja", "jpn"),
            ("zh", "zho"),
            ("chi", "zho"),
            ("eu", "eus"),
            ("baq", "eus"),
            ("und", "und"),
            ("unknown", "und"),
        ] {
            let audio = [AvchdMetaAudio {
                source: "combined.mkv",
                track: 2,
                language: input,
                codec: AvchdAudioCodec::Ac3,
            }];
            let meta = build_meta(&AvchdMetaRequest {
                video: "combined.mkv",
                audios: &audio,
                video_track: 1,
                fps: FrameRate::FPS_23_976,
                chapters: &chapters,
                capacity: DiscCapacity::Bd25,
                maxrate_kbps: 24_000,
                vbv_bufsize_kbps: 30_000,
                interactive_graphics: None,
                subtitles: &[],
            });
            assert!(
                meta.contains(&format!("lang={expected}")),
                "input {input} did not normalize to {expected}: {meta}"
            );
        }
    }

    #[test]
    fn meta_injects_blu_ray_header_and_interactive_graphics() {
        let audio = [AvchdMetaAudio {
            source: "combined.mkv",
            track: 2,
            language: "spa",
            codec: AvchdAudioCodec::Ac3,
        }];
        let meta = build_meta(&AvchdMetaRequest {
            video: "combined.mkv",
            audios: &audio,
            video_track: 1,
            fps: FrameRate::FPS_23_976,
            chapters: &[0],
            capacity: DiscCapacity::Bd25,
            maxrate_kbps: 38_000,
            vbv_bufsize_kbps: 35_000,
            interactive_graphics: Some("/tmp/root.igs"),
            subtitles: &[],
        });
        assert!(meta.starts_with("MUXOPT --blu-ray --vbv-bufsize=35000 --vbv-maxrate=38000"));
        assert!(meta.contains("S_HDMV/PGS, /tmp/root.igs, track=1, lang=und"));
    }

    #[test]
    fn blu_ray_encoder_uses_38_mbps_vbv() {
        let mut request = request();
        request.disc_capacity = DiscCapacity::Bd25;
        request.video_kbps = 30_000;
        request.maxrate_kbps = 38_000;
        request.vbv_bufsize_kbps = 35_000;
        let args = build_encode_args(&request).join(" ");
        assert!(args.contains("vbv-maxrate=38000"));
        assert!(args.contains("vbv-bufsize=35000"));
        assert!(args.contains("-maxrate 38000k"));
        assert!(args.contains("-bufsize 35000k"));
    }

    #[test]
    fn resolution_specs_match_avchd_standards() {
        assert_eq!(
            ResolutionSpec::from_resolution(AvchdResolution::R1080p),
            ResolutionSpec {
                width: 1920,
                height: 1080,
                interlaced: false
            }
        );
        assert_eq!(
            ResolutionSpec::from_resolution(AvchdResolution::Sd576i),
            ResolutionSpec {
                width: 720,
                height: 576,
                interlaced: true
            }
        );
    }

    #[test]
    fn trim_ms_args_seek_before_input_and_use_effective_duration() {
        let mut req = request();
        req.trim_start_ms = Some(2_000);
        req.trim_end_ms = Some(12_000);
        let args = build_encode_args(&req);
        let seek = args.iter().position(|arg| arg == "-ss").expect("seek");
        let input = args.iter().position(|arg| arg == "-i").expect("input");
        assert!(seek < input);
        assert_eq!(args[seek + 1], "2.000");
        assert!(args.windows(2).any(|pair| pair == ["-t", "10.000"]));
    }

    #[test]
    fn frame_range_limits_duration_when_ms_bounds_are_missing() {
        let mut req = request();
        req.duration_ms = 60_000;
        req.fps = FrameRate::NTSC;
        req.trim = Some(FrameRange {
            start: 300,
            end: 780,
        });
        let args = build_encode_args(&req);
        assert!(args.windows(2).any(|pair| pair == ["-t", "16.016"]));
    }

    #[test]
    fn scale_modes_map_to_avchd_filters() {
        let spec = ResolutionSpec::from_config(&config());
        let letterbox = build_filter(&request(), spec);
        assert!(letterbox.contains("force_original_aspect_ratio=decrease"));
        assert!(letterbox.contains("pad=1280:720"));

        let mut crop = request();
        crop.scale_mode = VideoScaleMode::CropFill;
        let crop_filter = build_filter(&crop, spec);
        assert!(crop_filter.contains("force_original_aspect_ratio=increase"));
        assert!(crop_filter.contains("crop=1280:720"));

        let mut stretch = request();
        stretch.scale_mode = VideoScaleMode::Stretch;
        let stretch_filter = build_filter(&stretch, spec);
        assert!(stretch_filter.contains("scale=1280:720"));
        assert!(!stretch_filter.contains("force_original_aspect_ratio"));
    }

    #[test]
    fn invalid_resolution_fps_pair_is_rejected() {
        let mut config = config();
        config.resolution = AvchdResolution::R1080p;
        config.frame_rate = FrameRate::FPS_29_97;
        let errors = config_errors(&config);
        assert!(errors
            .iter()
            .any(|error| error.code == "E_AVCHD_FPS_INCOMPATIBLE"));
    }
}
