//! DVD-Video encoding and authoring pipeline: MPEG-2 two-pass video, AC-3
//! audio, MPEG-PS multiplexing and dvdauthor XML generation.

pub mod authoring;

pub use authoring::{
    apply_region_mask, build_dvdauthor_xml, write_dvdauthor_xml, AuthorError, AuthoredAudioTrack,
    AuthoredButton, AuthoredSubtitleTrack, AuthoringRequest, DvdAuthor, EpisodeFile, RenderedMenu,
    VMG_REGION_OFFSET,
};

use dvdforge_domain::{
    AspectRatio, ButtonShapeMask, DvdProfile, FrameRange, FrameRate, MenuScaleMode, TvStandard,
    VideoScaleMode,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout, Command};
use tokio::sync::mpsc;

pub const CRATE_NAME: &str = "dvdforge-dvd";

pub const DVD_MAX_VIDEO_KBPS: u32 = 9_800;
pub const DVD_MAX_TOTAL_KBPS: u32 = 10_080;
pub const DVD_VBV_BITS: u32 = 1_835_008;
pub const DVD_MUX_OVERHEAD_PPM: u32 = 25_000;
pub const HALF_D1_AFTER_MS: u64 = 9_000_000;

#[derive(Debug, Clone)]
pub struct VideoEncodeConfig {
    pub video_kbps: u32,
    pub maxrate_kbps: u32,
    pub profile: DvdProfile,
    pub standard: TvStandard,
    pub aspect: AspectRatio,
    pub gop_frames: u16,
    pub b_frames: u8,
    pub interlaced: bool,
    pub top_field_first: bool,
    pub scale_mode: VideoScaleMode,
}

#[derive(Debug, Clone)]
pub struct AudioEncodeConfig {
    pub source_stream: u32,
    pub bitrate_kbps: u16,
    pub channels: u8,
    pub sample_rate: u32,
    /// Desfase aplicado con `adelay` (positivo) o `atrim` (negativo).
    pub delay_ms: i32,
}

#[derive(Debug, Clone)]
pub struct EncodeRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub stem: String,
    pub video: VideoEncodeConfig,
    pub audio_tracks: Vec<AudioEncodeConfig>,
    pub trim: Option<FrameRange>,
    pub trim_start_ms: Option<u64>,
    pub trim_end_ms: Option<u64>,
    pub fps: FrameRate,
}

#[derive(Debug, Clone)]
pub struct EncodeOutputs {
    pub video_m2v: PathBuf,
    pub audio_ac3: Vec<PathBuf>,
    pub muxed_mpg: PathBuf,
    pub passlog: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EncodeProgress {
    pub stage: &'static str,
    pub out_time_us: u64,
    pub speed_x: Option<f32>,
    pub fps: Option<f32>,
}

pub type EncodeProgressSink = mpsc::Sender<EncodeProgress>;

#[derive(Debug, Clone)]
pub struct MenuEncodeRequest {
    pub background_png: PathBuf,
    pub audio_ac3: Option<PathBuf>,
    pub output_mpg: PathBuf,
    pub duration_ms: u64,
    pub standard: TvStandard,
    pub aspect: AspectRatio,
    pub video_kbps: u32,
    pub maxrate_kbps: u32,
    pub fps: FrameRate,
}

#[derive(Debug, Clone)]
pub struct MotionButtonEncode {
    pub input: PathBuf,
    pub mask_png: Option<PathBuf>,
    pub source_start_ms: u64,
    pub loop_duration_ms: u64,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub shape_mask: ButtonShapeMask,
}

#[derive(Debug, Clone)]
pub struct MotionMenuEncodeRequest {
    /// Fondo compuesto sin botones (PNG) o vídeo de fondo de menú.
    pub background: PathBuf,
    pub background_is_video: bool,
    /// Textos, sombras, marcos e iconos con transparencia.
    pub labels_overlay: PathBuf,
    pub buttons: Vec<MotionButtonEncode>,
    pub audio_ac3: Option<PathBuf>,
    pub output_mpg: PathBuf,
    pub duration_ms: u64,
    pub standard: TvStandard,
    pub aspect: AspectRatio,
    pub fps: FrameRate,
    pub video_kbps: u32,
    pub maxrate_kbps: u32,
    pub dim_percent: u8,
    pub blur_px: u8,
    pub scale_mode: MenuScaleMode,
}

impl EncodeRequest {
    pub fn video_m2v(&self) -> PathBuf {
        self.output_dir.join(format!("{}.m2v", self.stem))
    }

    pub fn audio_ac3(&self, index: usize) -> PathBuf {
        if self.audio_tracks.len() <= 1 {
            self.output_dir.join(format!("{}.ac3", self.stem))
        } else {
            self.output_dir
                .join(format!("{}.a{}.ac3", self.stem, index + 1))
        }
    }

    pub fn muxed_mpg(&self) -> PathBuf {
        self.output_dir.join(format!("{}.mpg", self.stem))
    }

    pub fn passlog(&self) -> PathBuf {
        self.output_dir.join(format!("{}.passlog", self.stem))
    }

    pub fn validate(&self) -> Result<(), EncodeError> {
        if self.stem.trim().is_empty() || self.stem.contains('/') || self.stem.contains('\\') {
            return Err(EncodeError::InvalidConfig(
                "el nombre base de salida no es válido".to_string(),
            ));
        }
        if self.video.video_kbps == 0 || self.video.video_kbps > DVD_MAX_VIDEO_KBPS {
            return Err(EncodeError::InvalidConfig(format!(
                "bitrate de video fuera de rango: {} kbps",
                self.video.video_kbps
            )));
        }
        if self.video.maxrate_kbps < self.video.video_kbps
            || self.video.maxrate_kbps > DVD_MAX_VIDEO_KBPS
        {
            return Err(EncodeError::InvalidConfig(format!(
                "maxrate inválido: {} kbps",
                self.video.maxrate_kbps
            )));
        }
        let audio_total_kbps: u32 = self
            .audio_tracks
            .iter()
            .map(|audio| audio.bitrate_kbps as u32)
            .sum();
        if self.video.maxrate_kbps + audio_total_kbps + 100 > DVD_MAX_TOTAL_KBPS {
            return Err(EncodeError::InvalidConfig(
                "la suma de video y audio supera el máximo de DVD".to_string(),
            ));
        }
        if self.video.gop_frames == 0 {
            return Err(EncodeError::InvalidConfig(
                "el GOP no puede ser cero".to_string(),
            ));
        }
        for audio in &self.audio_tracks {
            if !matches!(audio.bitrate_kbps, 192 | 224 | 256 | 320 | 384 | 448) {
                return Err(EncodeError::InvalidConfig(format!(
                    "bitrate AC-3 no soportado: {} kbps",
                    audio.bitrate_kbps
                )));
            }
            if audio.sample_rate != 48_000 {
                return Err(EncodeError::InvalidConfig(
                    "DVD exige audio AC-3 a 48 kHz".to_string(),
                ));
            }
            if !matches!(audio.channels, 1 | 2 | 6) {
                return Err(EncodeError::InvalidConfig(
                    "DVD solo admite 1, 2 o 6 canales AC-3".to_string(),
                ));
            }
        }
        if let Some(trim) = self.trim {
            if !trim.is_valid() {
                return Err(EncodeError::InvalidConfig(
                    "el rango de recorte no es válido".to_string(),
                ));
            }
            if !self.fps.is_valid() {
                return Err(EncodeError::InvalidConfig(
                    "el framerate del recorte no es válido".to_string(),
                ));
            }
        }
        if let (Some(start), Some(end)) = (self.trim_start_ms, self.trim_end_ms) {
            if end <= start {
                return Err(EncodeError::InvalidConfig(
                    "el fin del recorte debe ser posterior al inicio".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("configuración de codificación inválida: {0}")]
    InvalidConfig(String),
    #[error("no se pudo preparar el directorio de salida: {0}")]
    Io(#[from] std::io::Error),
    #[error("FFmpeg falló en {stage} con código {code}: {stderr}")]
    Ffmpeg {
        stage: &'static str,
        code: i32,
        stderr: String,
    },
}

#[derive(Debug, Clone)]
pub struct DvdEncoder {
    pub ffmpeg: PathBuf,
}

impl Default for DvdEncoder {
    fn default() -> Self {
        Self {
            ffmpeg: PathBuf::from("ffmpeg"),
        }
    }
}

impl DvdEncoder {
    pub fn new(ffmpeg: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
        }
    }

    pub async fn encode(&self, request: &EncodeRequest) -> Result<EncodeOutputs, EncodeError> {
        self.encode_with_progress(request, None).await
    }

    pub async fn encode_with_progress(
        &self,
        request: &EncodeRequest,
        progress: Option<EncodeProgressSink>,
    ) -> Result<EncodeOutputs, EncodeError> {
        request.validate()?;
        std::fs::create_dir_all(&request.output_dir)?;

        let pass1 = build_pass1_args(request);
        run_stage(&self.ffmpeg, &pass1, "pass1", progress.as_ref()).await?;

        let pass2 = build_pass2_args(request);
        run_stage(&self.ffmpeg, &pass2, "pass2", progress.as_ref()).await?;

        let mut audio_ac3 = Vec::with_capacity(request.audio_tracks.len());
        for index in 0..request.audio_tracks.len() {
            let audio = build_audio_args(request, index);
            run_stage(&self.ffmpeg, &audio, "audio", progress.as_ref()).await?;
            audio_ac3.push(request.audio_ac3(index));
        }

        let mux = build_mux_args(request);
        run_stage(&self.ffmpeg, &mux, "mux", progress.as_ref()).await?;

        Ok(EncodeOutputs {
            video_m2v: request.video_m2v(),
            audio_ac3,
            muxed_mpg: request.muxed_mpg(),
            passlog: request.passlog(),
        })
    }

    pub async fn encode_menu(&self, request: &MenuEncodeRequest) -> Result<PathBuf, EncodeError> {
        self.encode_menu_with_progress(request, None).await
    }

    pub async fn encode_menu_with_progress(
        &self,
        request: &MenuEncodeRequest,
        progress: Option<EncodeProgressSink>,
    ) -> Result<PathBuf, EncodeError> {
        if request.duration_ms == 0 || !request.fps.is_valid() {
            return Err(EncodeError::InvalidConfig(
                "duración o framerate de menú inválidos".to_string(),
            ));
        }
        if request.video_kbps == 0
            || request.video_kbps > DVD_MAX_VIDEO_KBPS
            || request.maxrate_kbps < request.video_kbps
            || request.maxrate_kbps > DVD_MAX_VIDEO_KBPS
        {
            return Err(EncodeError::InvalidConfig(
                "bitrate de menú inválido".to_string(),
            ));
        }
        if let Some(parent) = request.output_mpg.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let args = build_menu_encode_args(request);
        run_stage(&self.ffmpeg, &args, "menu", progress.as_ref()).await?;
        Ok(request.output_mpg.clone())
    }

    pub async fn encode_motion_menu_with_progress(
        &self,
        request: &MotionMenuEncodeRequest,
        progress: Option<EncodeProgressSink>,
    ) -> Result<PathBuf, EncodeError> {
        if request.duration_ms == 0 || !request.fps.is_valid() {
            return Err(EncodeError::InvalidConfig(
                "duración o framerate de menú animado inválidos".to_string(),
            ));
        }
        if request.video_kbps == 0
            || request.video_kbps > DVD_MAX_VIDEO_KBPS
            || request.maxrate_kbps < request.video_kbps
            || request.maxrate_kbps > DVD_MAX_VIDEO_KBPS
        {
            return Err(EncodeError::InvalidConfig(
                "bitrate de menú animado inválido".to_string(),
            ));
        }
        if request
            .buttons
            .iter()
            .any(|button| button.width == 0 || button.height == 0)
        {
            return Err(EncodeError::InvalidConfig(
                "un botón animado tiene geometría vacía".to_string(),
            ));
        }
        if let Some(parent) = request.output_mpg.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let args = build_motion_menu_args(request);
        run_stage(&self.ffmpeg, &args, "motion-menu", progress.as_ref()).await?;
        Ok(request.output_mpg.clone())
    }
}

async fn run_stage(
    ffmpeg: &Path,
    args: &[String],
    stage: &'static str,
    progress: Option<&EncodeProgressSink>,
) -> Result<(), EncodeError> {
    let mut child = Command::new(ffmpeg)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| EncodeError::InvalidConfig("no se pudo capturar stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| EncodeError::InvalidConfig("no se pudo capturar stderr".to_string()))?;
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
    let status = child.wait().await?;
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
        Err(EncodeError::Ffmpeg {
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
            let value = value.trim().trim_end_matches('x');
            speed_x = value.parse::<f32>().ok();
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

/// Builds the Lanczos scale/pad chain for the requested DVD profile.
pub fn build_scale_filter(video: &VideoEncodeConfig) -> String {
    let (display_width, display_height) = display_canvas(video.standard, video.aspect);
    let (target_width, target_height) = target_canvas(video.standard, video.profile);
    let sar = target_sar(video.standard, video.aspect, video.profile);
    let scale_flags = if video.interlaced {
        "flags=lanczos:interl=1"
    } else {
        "flags=lanczos"
    };
    match video.scale_mode {
        VideoScaleMode::Letterbox | VideoScaleMode::Contain => format!(
            "scale=iw*sar:ih,setsar=1,\
scale={display_width}:{display_height}:force_original_aspect_ratio=decrease:{scale_flags},\
pad={display_width}:{display_height}:(ow-iw)/2:(oh-ih)/2,\
scale={target_width}:{target_height}:{scale_flags},setsar={sar},format=yuv420p"
        ),
        VideoScaleMode::CropFill | VideoScaleMode::Cover => format!(
            "scale=iw*sar:ih,setsar=1,\
scale={target_width}:{target_height}:force_original_aspect_ratio=increase:{scale_flags},\
crop={target_width}:{target_height},setsar={sar},format=yuv420p"
        ),
        VideoScaleMode::Stretch => format!(
            "scale=iw*sar:ih,setsar=1,scale={target_width}:{target_height}:{scale_flags},\
setsar={sar},format=yuv420p"
        ),
    }
}

pub fn build_pass1_args(request: &EncodeRequest) -> Vec<String> {
    let mut args = input_args(request);
    args.extend(video_codec_args(&request.video));
    args.extend([
        "-pass".to_string(),
        "1".to_string(),
        "-passlogfile".to_string(),
        request.passlog().display().to_string(),
        "-an".to_string(),
        "-sn".to_string(),
        "-dn".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "null".to_string(),
        "-".to_string(),
    ]);
    args
}

pub fn build_pass2_args(request: &EncodeRequest) -> Vec<String> {
    let mut args = input_args(request);
    args.extend(video_codec_args(&request.video));
    args.extend([
        "-pass".to_string(),
        "2".to_string(),
        "-passlogfile".to_string(),
        request.passlog().display().to_string(),
        "-an".to_string(),
        "-sn".to_string(),
        "-dn".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "mpeg2video".to_string(),
        request.video_m2v().display().to_string(),
    ]);
    args
}

pub fn build_audio_args(request: &EncodeRequest, index: usize) -> Vec<String> {
    let Some(audio) = request.audio_tracks.get(index) else {
        return Vec::new();
    };
    let mut args = input_args(request);
    args.extend([
        "-map".to_string(),
        format!("0:{}", audio.source_stream),
        "-c:a".to_string(),
        "ac3".to_string(),
        "-b:a".to_string(),
        format!("{}k", audio.bitrate_kbps),
        "-ar".to_string(),
        audio.sample_rate.to_string(),
        "-ac".to_string(),
        audio.channels.to_string(),
    ]);
    if audio.delay_ms > 0 {
        args.extend([
            "-af".to_string(),
            format!("adelay={}:all=1", audio.delay_ms),
        ]);
    } else if audio.delay_ms < 0 {
        args.extend([
            "-af".to_string(),
            format!(
                "atrim=start={:.3},asetpts=PTS-STARTPTS",
                audio.delay_ms.unsigned_abs() as f64 / 1000.0
            ),
        ]);
    }
    args.extend([
        "-vn".to_string(),
        "-sn".to_string(),
        "-dn".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "ac3".to_string(),
        request.audio_ac3(index).display().to_string(),
    ]);
    args
}

pub fn build_mux_args(request: &EncodeRequest) -> Vec<String> {
    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        request.video_m2v().display().to_string(),
    ];
    for index in 0..request.audio_tracks.len() {
        args.extend([
            "-i".to_string(),
            request.audio_ac3(index).display().to_string(),
        ]);
    }
    args.extend(["-map".to_string(), "0:v:0".to_string()]);
    for index in 0..request.audio_tracks.len() {
        args.extend(["-map".to_string(), format!("{}:a:0", index + 1)]);
    }
    args.extend([
        "-c".to_string(),
        "copy".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "dvd".to_string(),
        request.muxed_mpg().display().to_string(),
    ]);
    args
}

pub fn build_menu_encode_args(request: &MenuEncodeRequest) -> Vec<String> {
    let video = VideoEncodeConfig {
        video_kbps: request.video_kbps,
        maxrate_kbps: request.maxrate_kbps,
        profile: DvdProfile::D1,
        standard: request.standard,
        aspect: request.aspect,
        gop_frames: 12,
        b_frames: 2,
        interlaced: false,
        top_field_first: true,
        scale_mode: VideoScaleMode::Letterbox,
    };
    let mut args = vec![
        "-hide_banner".to_string(),
        "-y".to_string(),
        "-loop".to_string(),
        "1".to_string(),
        "-framerate".to_string(),
        format!("{}/{}", request.fps.num, request.fps.den),
        "-i".to_string(),
        request.background_png.display().to_string(),
    ];
    if let Some(audio) = &request.audio_ac3 {
        args.extend(["-i".to_string(), audio.display().to_string()]);
    } else {
        args.extend([
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "anullsrc=r=48000:cl=stereo".to_string(),
        ]);
    }
    args.extend([
        "-t".to_string(),
        format!("{:.3}", request.duration_ms as f64 / 1000.0),
    ]);
    args.extend(video_codec_args(&video));
    args.extend([
        "-map".to_string(),
        "0:v:0".to_string(),
        "-map".to_string(),
        "1:a:0".to_string(),
    ]);
    if request.audio_ac3.is_some() {
        args.extend(["-c:a".to_string(), "copy".to_string()]);
    } else {
        args.extend([
            "-c:a".to_string(),
            "ac3".to_string(),
            "-b:a".to_string(),
            "192k".to_string(),
            "-ar".to_string(),
            "48000".to_string(),
            "-ac".to_string(),
            "2".to_string(),
        ]);
    }
    args.extend([
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-f".to_string(),
        "dvd".to_string(),
        request.output_mpg.display().to_string(),
    ]);
    args
}

/// Construye el grafo `filter_complex` para un menú animado con vídeo en los
/// botones. Es una función pura para poder validar los índices de entrada y el
/// grafo con tests sin ejecutar FFmpeg.
pub fn build_motion_menu_args(request: &MotionMenuEncodeRequest) -> Vec<String> {
    let (width, height) = match request.standard {
        TvStandard::Ntsc => (720, 480),
        TvStandard::Pal => (720, 576),
    };
    let fps = format!("{}/{}", request.fps.num, request.fps.den);
    let duration = format!("{:.3}", request.duration_ms as f64 / 1000.0);
    let button_count = request.buttons.len();
    let labels_index = 1 + button_count;
    let mask_base = labels_index + 1;
    let mask_count = request
        .buttons
        .iter()
        .filter(|button| button.mask_png.is_some())
        .count();
    let audio_index = mask_base + mask_count;

    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    if request.background_is_video {
        args.extend(["-stream_loop".to_string(), "-1".to_string()]);
    } else {
        args.extend([
            "-loop".to_string(),
            "1".to_string(),
            "-framerate".to_string(),
            fps.clone(),
        ]);
    }
    args.extend(["-i".to_string(), request.background.display().to_string()]);
    for button in &request.buttons {
        args.extend([
            "-stream_loop".to_string(),
            "-1".to_string(),
            "-i".to_string(),
            button.input.display().to_string(),
        ]);
    }
    args.extend([
        "-loop".to_string(),
        "1".to_string(),
        "-framerate".to_string(),
        fps.clone(),
        "-i".to_string(),
        request.labels_overlay.display().to_string(),
    ]);
    for button in request
        .buttons
        .iter()
        .filter(|button| button.mask_png.is_some())
    {
        args.extend([
            "-loop".to_string(),
            "1".to_string(),
            "-framerate".to_string(),
            fps.clone(),
            "-i".to_string(),
            button
                .mask_png
                .as_ref()
                .expect("mask filtered above")
                .display()
                .to_string(),
        ]);
    }
    if let Some(audio) = &request.audio_ac3 {
        args.extend(["-i".to_string(), audio.display().to_string()]);
    } else {
        args.extend([
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "anullsrc=r=48000:cl=stereo".to_string(),
        ]);
    }

    let mut filters = Vec::new();
    let scale = match request.scale_mode {
        MenuScaleMode::Cover => format!(
            "scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"
        ),
        MenuScaleMode::Contain => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2"
        ),
        MenuScaleMode::Stretch => format!("scale={width}:{height}"),
    };
    let mut background_chain = format!("[0:v]{scale},setsar=1");
    if request.blur_px > 0 {
        background_chain.push_str(&format!(",boxblur={}:1", request.blur_px.min(20)));
    }
    if request.dim_percent > 0 {
        let alpha = request.dim_percent.min(80) as f32 / 100.0;
        background_chain.push_str(&format!(
            ",drawbox=x=0:y=0:w=iw:h=ih:color=black@{alpha:.2}:t=fill"
        ));
    }
    background_chain.push_str(&format!(
        ",trim=duration={duration},setpts=PTS-STARTPTS,fps={fps},format=yuv420p[bg]"
    ));
    filters.push(background_chain);

    let mut mask_index = mask_base;
    for (index, button) in request.buttons.iter().enumerate() {
        let source_start = format!("{:.3}", button.source_start_ms as f64 / 1000.0);
        let loop_duration = format!("{:.3}", button.loop_duration_ms as f64 / 1000.0);
        let loop_frames =
            ((button.loop_duration_ms as f64 * request.fps.as_f64() / 1000.0).ceil() as u64).max(1);
        let raw = format!("btn{index}raw");
        let output = format!("btn{index}");
        filters.push(format!(
            "[{}:v]trim=start={source_start}:duration={loop_duration},setpts=PTS-STARTPTS,fps={fps},loop=loop=-1:size={loop_frames},setpts=N/FRAME_RATE/TB,scale={}:{}:force_original_aspect_ratio=increase,crop={}:{},setsar=1,format=rgba[{raw}]",
            index + 1,
            button.width,
            button.height,
            button.width,
            button.height
        ));
        if button.mask_png.is_some() {
            let mask = format!("mask{index}");
            filters.push(format!("[{mask_index}:v]format=gray[{mask}]"));
            filters.push(format!("[{raw}][{mask}]alphamerge[{output}]"));
            mask_index += 1;
        } else {
            filters.push(format!("[{raw}]null[{output}]"));
        }
    }

    filters.push(format!("[{labels_index}:v]format=rgba[labels]"));
    let mut current = "bg".to_string();
    for (index, button) in request.buttons.iter().enumerate() {
        let output = format!("overlay{index}");
        filters.push(format!(
            "[{current}][btn{index}]overlay=x={}:y={}:shortest=0:format=auto[{output}]",
            button.x, button.y
        ));
        current = output;
    }
    filters.push(format!(
        "[{current}][labels]overlay=0:0:shortest=0:format=auto[composed]"
    ));
    filters.push("[composed]format=yuv420p[vout]".to_string());
    filters.push(format!(
        "[{audio_index}:a]atrim=duration={duration},asetpts=PTS-STARTPTS[aout]"
    ));

    args.extend([
        "-filter_complex".to_string(),
        filters.join(";"),
        "-map".to_string(),
        "[vout]".to_string(),
        "-map".to_string(),
        "[aout]".to_string(),
        "-c:v".to_string(),
        "mpeg2video".to_string(),
        "-target".to_string(),
        match request.standard {
            TvStandard::Ntsc => "ntsc-dvd",
            TvStandard::Pal => "pal-dvd",
        }
        .to_string(),
        "-b:v".to_string(),
        format!("{}k", request.video_kbps),
        "-maxrate".to_string(),
        format!("{}k", request.maxrate_kbps),
        "-bufsize".to_string(),
        "1835k".to_string(),
        "-g".to_string(),
        "15".to_string(),
        "-aspect".to_string(),
        match request.aspect {
            AspectRatio::FourThree => "4:3",
            AspectRatio::SixteenNine => "16:9",
        }
        .to_string(),
        "-c:a".to_string(),
        "ac3".to_string(),
        "-b:a".to_string(),
        "192k".to_string(),
        "-ar".to_string(),
        "48000".to_string(),
        "-ac".to_string(),
        "2".to_string(),
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        "-t".to_string(),
        duration,
        "-f".to_string(),
        "dvd".to_string(),
        request.output_mpg.display().to_string(),
    ]);
    args
}

fn input_args(request: &EncodeRequest) -> Vec<String> {
    let mut args = vec!["-hide_banner".to_string(), "-y".to_string()];
    if request.trim_start_ms.is_some() || request.trim_end_ms.is_some() {
        let start_ms = request.trim_start_ms.unwrap_or(0);
        if start_ms > 0 {
            args.push("-ss".to_string());
            args.push(format_millis(start_ms));
        }
        args.extend(["-i".to_string(), request.input.display().to_string()]);
        let end_ms = request.trim_end_ms;
        let duration = end_ms
            .map(|end| end.saturating_sub(start_ms))
            .filter(|duration| *duration > 0);
        if let Some(duration) = duration {
            args.push("-t".to_string());
            args.push(format_millis(duration));
        }
        return args;
    }
    if let Some(trim) = request.trim {
        args.push("-ss".to_string());
        args.push(format_seconds(trim.start, request.fps));
    }
    args.extend(["-i".to_string(), request.input.display().to_string()]);
    if let Some(trim) = request.trim {
        args.push("-t".to_string());
        args.push(format_seconds(trim.duration(), request.fps));
    }
    args
}

fn format_millis(millis: u64) -> String {
    format!("{:.3}", millis as f64 / 1000.0)
}

fn video_codec_args(video: &VideoEncodeConfig) -> Vec<String> {
    let mut args = vec![
        "-vf".to_string(),
        build_scale_filter(video),
        "-c:v".to_string(),
        "mpeg2video".to_string(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        "-b:v".to_string(),
        format!("{}k", video.video_kbps),
        "-maxrate".to_string(),
        format!("{}k", video.maxrate_kbps),
        "-bufsize".to_string(),
        DVD_VBV_BITS.to_string(),
        "-g".to_string(),
        video.gop_frames.to_string(),
        "-bf".to_string(),
        video.b_frames.to_string(),
        "-b_strategy".to_string(),
        "2".to_string(),
        "-mbd".to_string(),
        "rd".to_string(),
        "-trellis".to_string(),
        "2".to_string(),
        "-cmp".to_string(),
        "2".to_string(),
        "-subcmp".to_string(),
        "2".to_string(),
        "-aspect".to_string(),
        aspect_label(video.aspect).to_string(),
    ];
    if video.interlaced {
        args.extend([
            "-flags".to_string(),
            "+ildct+ilme".to_string(),
            "-top".to_string(),
            if video.top_field_first { "1" } else { "0" }.to_string(),
        ]);
    }
    args
}

fn format_seconds(frames: u64, fps: FrameRate) -> String {
    format!("{:.6}", frames as f64 / fps.as_f64())
}

fn display_canvas(standard: TvStandard, aspect: AspectRatio) -> (u16, u16) {
    match (standard, aspect) {
        (TvStandard::Ntsc, AspectRatio::FourThree) => (640, 480),
        (TvStandard::Ntsc, AspectRatio::SixteenNine) => (854, 480),
        (TvStandard::Pal, AspectRatio::FourThree) => (768, 576),
        (TvStandard::Pal, AspectRatio::SixteenNine) => (1024, 576),
    }
}

fn target_canvas(standard: TvStandard, profile: DvdProfile) -> (u16, u16) {
    match (standard, profile) {
        (TvStandard::Ntsc, DvdProfile::HalfD1) => (352, 480),
        (TvStandard::Pal, DvdProfile::HalfD1) => (352, 576),
        (TvStandard::Ntsc, DvdProfile::D1 | DvdProfile::Auto) => (720, 480),
        (TvStandard::Pal, DvdProfile::D1 | DvdProfile::Auto) => (720, 576),
    }
}

fn target_sar(standard: TvStandard, aspect: AspectRatio, profile: DvdProfile) -> &'static str {
    match (standard, aspect, profile) {
        (TvStandard::Ntsc, AspectRatio::FourThree, DvdProfile::D1 | DvdProfile::Auto) => "8/9",
        (TvStandard::Ntsc, AspectRatio::SixteenNine, DvdProfile::D1 | DvdProfile::Auto) => "32/27",
        (TvStandard::Pal, AspectRatio::FourThree, DvdProfile::D1 | DvdProfile::Auto) => "16/15",
        (TvStandard::Pal, AspectRatio::SixteenNine, DvdProfile::D1 | DvdProfile::Auto) => "64/45",
        (TvStandard::Ntsc, AspectRatio::FourThree, DvdProfile::HalfD1) => "20/11",
        (TvStandard::Ntsc, AspectRatio::SixteenNine, DvdProfile::HalfD1) => "80/33",
        (TvStandard::Pal, AspectRatio::FourThree, DvdProfile::HalfD1) => "24/11",
        (TvStandard::Pal, AspectRatio::SixteenNine, DvdProfile::HalfD1) => "32/11",
    }
}

fn aspect_label(aspect: AspectRatio) -> &'static str {
    match aspect {
        AspectRatio::FourThree => "4:3",
        AspectRatio::SixteenNine => "16:9",
    }
}

pub fn max_video_kbps_for_audio(total_audio_kbps: u32) -> u32 {
    DVD_MAX_VIDEO_KBPS.min(DVD_MAX_TOTAL_KBPS.saturating_sub(total_audio_kbps + 100))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(profile: DvdProfile, standard: TvStandard, aspect: AspectRatio) -> VideoEncodeConfig {
        VideoEncodeConfig {
            video_kbps: 4_500,
            maxrate_kbps: 9_000,
            profile,
            standard,
            aspect,
            gop_frames: 12,
            b_frames: 2,
            interlaced: false,
            top_field_first: true,
            scale_mode: VideoScaleMode::Letterbox,
        }
    }

    fn request() -> EncodeRequest {
        EncodeRequest {
            input: PathBuf::from("/tmp/input.mp4"),
            output_dir: PathBuf::from("/tmp/out"),
            stem: "episode-01".to_string(),
            video: video(DvdProfile::D1, TvStandard::Ntsc, AspectRatio::FourThree),
            audio_tracks: vec![AudioEncodeConfig {
                source_stream: 1,
                bitrate_kbps: 192,
                channels: 2,
                sample_rate: 48_000,
                delay_ms: 0,
            }],
            trim: None,
            trim_start_ms: None,
            trim_end_ms: None,
            fps: FrameRate::NTSC,
        }
    }

    #[test]
    fn d1_ntsc_4_3_filter_scales_and_pads() {
        let filter = build_scale_filter(&request().video);
        assert!(filter.contains("scale=640:480"));
        assert!(filter.contains("pad=640:480"));
        assert!(filter.contains("scale=720:480"));
        assert!(filter.contains("setsar=8/9"));
        assert!(filter.contains("format=yuv420p"));
    }

    #[test]
    fn half_d1_pal_16_9_filter_uses_352x576() {
        let filter = build_scale_filter(&video(
            DvdProfile::HalfD1,
            TvStandard::Pal,
            AspectRatio::SixteenNine,
        ));
        assert!(filter.contains("scale=1024:576"));
        assert!(filter.contains("scale=352:576"));
        assert!(filter.contains("setsar=32/11"));
    }

    #[test]
    fn pass1_args_include_two_pass_and_motion_flags() {
        let args = build_pass1_args(&request());
        assert!(has_pair(&args, "-pass", "1"));
        assert!(has_pair(&args, "-mbd", "rd"));
        assert!(has_pair(&args, "-trellis", "2"));
        assert!(has_pair(&args, "-cmp", "2"));
        assert!(has_pair(&args, "-subcmp", "2"));
        assert!(has_pair(&args, "-f", "null"));
        assert!(args.contains(&"-passlogfile".to_string()));
    }

    #[test]
    fn pass2_outputs_elementary_mpeg2() {
        let args = build_pass2_args(&request());
        assert!(has_pair(&args, "-pass", "2"));
        assert!(has_pair(&args, "-f", "mpeg2video"));
        assert!(args.iter().any(|arg| arg.ends_with("episode-01.m2v")));
    }

    #[test]
    fn interlaced_source_sets_top_field_and_flags() {
        let mut req = request();
        req.video.interlaced = true;
        req.video.top_field_first = true;
        let args = build_pass1_args(&req);
        assert!(has_pair(&args, "-flags", "+ildct+ilme"));
        assert!(has_pair(&args, "-top", "1"));
        assert!(build_scale_filter(&req.video).contains("interl=1"));
    }

    #[test]
    fn audio_args_are_ac3_stereo_48k() {
        let args = build_audio_args(&request(), 0);
        assert!(has_pair(&args, "-map", "0:1"));
        assert!(has_pair(&args, "-c:a", "ac3"));
        assert!(has_pair(&args, "-b:a", "192k"));
        assert!(has_pair(&args, "-ar", "48000"));
        assert!(has_pair(&args, "-ac", "2"));
        assert!(has_pair(&args, "-f", "ac3"));
    }

    #[test]
    fn audio_delay_uses_adelay_or_atrim() {
        let mut req = request();
        req.audio_tracks[0].delay_ms = 750;
        let positive = build_audio_args(&req, 0);
        assert!(has_pair(&positive, "-af", "adelay=750:all=1"));
        req.audio_tracks[0].delay_ms = -500;
        let negative = build_audio_args(&req, 0);
        assert!(has_pair(
            &negative,
            "-af",
            "atrim=start=0.500,asetpts=PTS-STARTPTS"
        ));
    }

    #[test]
    fn mux_args_use_dvd_program_stream_and_all_audio_tracks() {
        let mut req = request();
        req.audio_tracks.push(AudioEncodeConfig {
            source_stream: 2,
            bitrate_kbps: 448,
            channels: 6,
            sample_rate: 48_000,
            delay_ms: 0,
        });
        let args = build_mux_args(&req);
        assert!(has_pair(&args, "-map", "0:v:0"));
        assert!(has_pair(&args, "-map", "1:a:0"));
        assert!(has_pair(&args, "-map", "2:a:0"));
        assert!(has_pair(&args, "-c", "copy"));
        assert!(has_pair(&args, "-f", "dvd"));
        assert!(args.iter().any(|arg| arg.ends_with("episode-01.mpg")));
        assert!(args.iter().any(|arg| arg.ends_with("episode-01.a2.ac3")));
    }

    #[test]
    fn trim_ms_args_seek_before_input_and_limit_effective_duration() {
        let mut req = request();
        req.trim_start_ms = Some(12_500);
        req.trim_end_ms = Some(74_500);
        let args = build_pass1_args(&req);
        let seek = args.iter().position(|arg| arg == "-ss").expect("seek");
        let input = args.iter().position(|arg| arg == "-i").expect("input");
        assert!(seek < input);
        assert_eq!(args[seek + 1], "12.500");
        assert!(has_pair(&args, "-t", "62.000"));
        let audio = build_audio_args(&req, 0);
        assert!(
            audio.iter().position(|arg| arg == "-ss").expect("seek")
                < audio.iter().position(|arg| arg == "-i").expect("input")
        );
        assert!(has_pair(&audio, "-t", "62.000"));
    }

    #[test]
    fn video_scale_modes_use_letterbox_cropfill_or_stretch() {
        let letterbox = build_scale_filter(&video(
            DvdProfile::D1,
            TvStandard::Ntsc,
            AspectRatio::SixteenNine,
        ));
        assert!(letterbox.contains("force_original_aspect_ratio=decrease"));
        assert!(letterbox.contains("pad=854:480"));

        let mut crop = video(DvdProfile::D1, TvStandard::Ntsc, AspectRatio::SixteenNine);
        crop.scale_mode = VideoScaleMode::CropFill;
        let crop_filter = build_scale_filter(&crop);
        assert!(crop_filter.contains("force_original_aspect_ratio=increase"));
        assert!(crop_filter.contains("crop=720:480"));
        assert!(!crop_filter.contains("pad="));

        let mut stretch = video(DvdProfile::D1, TvStandard::Ntsc, AspectRatio::SixteenNine);
        stretch.scale_mode = VideoScaleMode::Stretch;
        let stretch_filter = build_scale_filter(&stretch);
        assert!(stretch_filter.contains("scale=720:480"));
        assert!(!stretch_filter.contains("force_original_aspect_ratio"));
        assert!(!stretch_filter.contains("pad="));
    }

    #[test]
    fn trim_is_applied_to_input_and_output_duration() {
        let mut req = request();
        req.trim = Some(FrameRange { start: 25, end: 50 });
        req.fps = FrameRate::PAL;
        let args = build_pass1_args(&req);
        assert!(has_pair(&args, "-ss", "1.000000"));
        assert!(has_pair(&args, "-t", "1.000000"));
    }

    #[test]
    fn invalid_encoder_config_is_rejected() {
        let mut req = request();
        req.video.maxrate_kbps = 1_000;
        assert!(matches!(req.validate(), Err(EncodeError::InvalidConfig(_))));
    }

    fn motion_button(index: usize, shape: ButtonShapeMask) -> MotionButtonEncode {
        MotionButtonEncode {
            input: PathBuf::from(format!("/tmp/button-{index}.mp4")),
            mask_png: (shape != ButtonShapeMask::Rectangle)
                .then(|| PathBuf::from(format!("/tmp/button-{index}-mask.png"))),
            source_start_ms: 1_000,
            loop_duration_ms: 5_000,
            x: 40 + index as u16 * 24,
            y: 120,
            width: 160,
            height: 90,
            shape_mask: shape,
        }
    }

    fn motion_request(button_count: usize, shape: ButtonShapeMask) -> MotionMenuEncodeRequest {
        MotionMenuEncodeRequest {
            background: PathBuf::from("/tmp/menu-background.png"),
            background_is_video: false,
            labels_overlay: PathBuf::from("/tmp/labels.png"),
            buttons: (0..button_count)
                .map(|index| motion_button(index, shape))
                .collect(),
            audio_ac3: Some(PathBuf::from("/tmp/menu.ac3")),
            output_mpg: PathBuf::from("/tmp/menu-motion.mpg"),
            duration_ms: 15_000,
            standard: TvStandard::Ntsc,
            aspect: AspectRatio::FourThree,
            fps: FrameRate::NTSC,
            video_kbps: 6_000,
            maxrate_kbps: 8_000,
            dim_percent: 0,
            blur_px: 0,
            scale_mode: MenuScaleMode::Cover,
        }
    }

    fn filter_complex(args: &[String]) -> &str {
        let index = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .expect("filter_complex");
        &args[index + 1]
    }

    #[test]
    fn motion_menu_args_handle_one_two_and_four_buttons() {
        for count in [1usize, 2, 4] {
            let request = motion_request(count, ButtonShapeMask::Rectangle);
            let args = build_motion_menu_args(&request);
            let input_count = args.iter().filter(|arg| arg.as_str() == "-i").count();
            assert_eq!(input_count, count + 3, "inputs for {count} buttons");
            let filter = filter_complex(&args);
            assert_eq!(filter.matches("overlay=").count(), count + 1);
            assert!(filter.contains("trim=start=1.000:duration=5.000"));
            assert!(filter.contains("loop=loop=-1:size=150"));
            assert!(has_pair(&args, "-target", "ntsc-dvd"));
            assert!(has_pair(&args, "-b:v", "6000k"));
            assert!(has_pair(&args, "-maxrate", "8000k"));
            assert!(has_pair(&args, "-bufsize", "1835k"));
            assert!(has_pair(&args, "-g", "15"));
            assert!(has_pair(&args, "-c:a", "ac3"));
            assert!(has_pair(&args, "-b:a", "192k"));
            assert!(has_pair(&args, "-t", "15.000"));
        }
    }

    #[test]
    fn motion_menu_args_add_one_mask_input_per_shaped_button() {
        let request = motion_request(2, ButtonShapeMask::Circle);
        let args = build_motion_menu_args(&request);
        assert_eq!(args.iter().filter(|arg| arg.as_str() == "-i").count(), 7);
        let filter = filter_complex(&args);
        assert_eq!(filter.matches("alphamerge").count(), 2);
        assert!(filter.contains("[4:v]format=gray[mask0]"));
        assert!(filter.contains("[5:v]format=gray[mask1]"));
        assert!(filter.contains("[6:a]atrim="));
        assert!(filter.contains("alphamerge[btn0]"));
    }

    #[test]
    fn motion_menu_args_apply_pal_scale_dim_and_blur() {
        let mut request = motion_request(1, ButtonShapeMask::Rectangle);
        request.standard = TvStandard::Pal;
        request.fps = FrameRate::PAL;
        request.scale_mode = MenuScaleMode::Contain;
        request.dim_percent = 30;
        request.blur_px = 5;
        let args = build_motion_menu_args(&request);
        assert!(has_pair(&args, "-target", "pal-dvd"));
        let filter = filter_complex(&args);
        assert!(filter.contains("scale=720:576:force_original_aspect_ratio=decrease"));
        assert!(filter.contains("pad=720:576"));
        assert!(filter.contains("boxblur=5:1"));
        assert!(filter.contains("drawbox=x=0:y=0:w=iw:h=ih:color=black@0.30:t=fill"));
    }

    #[test]
    fn motion_menu_args_generate_silence_when_no_menu_music() {
        let mut request = motion_request(1, ButtonShapeMask::Rectangle);
        request.audio_ac3 = None;
        let args = build_motion_menu_args(&request);
        assert!(args.iter().any(|arg| arg == "anullsrc=r=48000:cl=stereo"));
        assert!(filter_complex(&args).contains("[3:a]atrim="));
    }

    #[test]
    fn menu_args_use_silent_audio_and_dvd_mux() {
        let args = build_menu_encode_args(&MenuEncodeRequest {
            background_png: PathBuf::from("/tmp/menu.png"),
            audio_ac3: None,
            output_mpg: PathBuf::from("/tmp/menu.mpg"),
            duration_ms: 30_000,
            standard: TvStandard::Ntsc,
            aspect: AspectRatio::FourThree,
            video_kbps: 5_000,
            maxrate_kbps: 8_000,
            fps: FrameRate::NTSC,
        });
        assert!(has_pair(&args, "-loop", "1"));
        assert!(args.iter().any(|arg| arg == "anullsrc=r=48000:cl=stereo"));
        assert!(has_pair(&args, "-t", "30.000"));
        assert!(has_pair(&args, "-f", "dvd"));
        assert!(args.iter().any(|arg| arg.ends_with("menu.mpg")));
    }

    fn has_pair(args: &[String], key: &str, value: &str) -> bool {
        args.windows(2)
            .any(|window| window[0] == key && window[1] == value)
    }
}
