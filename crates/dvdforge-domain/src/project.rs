use crate::budget::BudgetPolicy;
use crate::diagnostic::Diagnostic;
use crate::geometry::{
    normalize_rect, validate_layout, AspectRatio, FrameRange, FrameRate, TvStandard,
};
use crate::ids::{AssetId, ChapterId, EpisodeId, MenuId, ProjectId};
use crate::menu::{ButtonAction, Menu, MenuItem, MenuPlacement, MenuRole};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AuthoringMode {
    DvdVideo,
    Avchd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DiscCapacity {
    Dvd5,
    Dvd9,
    Bd25,
    Bd50,
}

impl DiscCapacity {
    /// Nominal capacity in decimal bytes.
    pub const fn nominal_bytes(self) -> u64 {
        match self {
            Self::Dvd5 => 4_700_000_000,
            Self::Dvd9 => 8_540_000_000,
            Self::Bd25 => 25_000_000_000,
            Self::Bd50 => 50_000_000_000,
        }
    }

    /// Capacity in binary GiB, tal como la muestra el sistema operativo.
    pub const fn effective_bytes(self) -> u64 {
        match self {
            Self::Dvd5 => 4_692_251_770,
            Self::Dvd9 => 8_536_247_500,
            Self::Bd25 => 24_996_709_662,
            Self::Bd50 => 49_993_419_325,
        }
    }

    pub const fn is_blu_ray(self) -> bool {
        matches!(self, Self::Bd25 | Self::Bd50)
    }

    pub const fn is_hd(self) -> bool {
        self.is_blu_ray()
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dvd5 => "DVD-5",
            Self::Dvd9 => "DVD-9",
            Self::Bd25 => "BD-25",
            Self::Bd50 => "BD-50",
        }
    }

    pub const fn max_video_kbps(self) -> u32 {
        if self.is_blu_ray() {
            38_000
        } else {
            9_800
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioDefaults {
    pub ac3_kbps: u16,
    pub channels: u8,
    pub sample_rate: u32,
}

impl Default for AudioDefaults {
    fn default() -> Self {
        Self {
            ac3_kbps: 192,
            channels: 2,
            sample_rate: 48_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LayerBreakMode {
    Ptp,
    #[default]
    Otp,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscConfig {
    pub mode: AuthoringMode,
    pub standard: TvStandard,
    pub aspect: AspectRatio,
    pub capacity: DiscCapacity,
    pub volume_label: String,
    /// Máscara RCE de regiones DVD. `0` = libre; bits 0..5 = regiones 1..6.
    #[serde(default)]
    pub region_mask: u8,
    #[serde(default)]
    pub layer_break_mode: LayerBreakMode,
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VideoConfig {
    Dvd(DvdVideoConfig),
    Avchd(AvchdVideoConfig),
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DvdVideoConfig {
    pub profile: DvdProfile,
    pub auto_half_d1_after_ms: u64,
    pub max_video_kbps: u32,
    pub min_video_kbps: u32,
    pub gop_frames: u16,
    pub b_frames: u8,
    pub two_pass: bool,
    #[serde(default = "default_true")]
    pub trellis: bool,
}

impl Default for DvdVideoConfig {
    fn default() -> Self {
        Self {
            profile: DvdProfile::Auto,
            auto_half_d1_after_ms: 9_000_000,
            max_video_kbps: 9_800,
            min_video_kbps: 2_500,
            gop_frames: 12,
            b_frames: 2,
            two_pass: true,
            trellis: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DvdProfile {
    D1,
    HalfD1,
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AvchdVideoConfig {
    pub resolution: AvchdResolution,
    pub frame_rate: FrameRate,
    pub level: H264Level,
    pub bluray_compat: bool,
    pub vbv_maxrate_kbps: u32,
    pub vbv_bufsize_kbps: u32,
    pub keyint_frames: u16,
    pub slices: u8,
}

impl Default for AvchdVideoConfig {
    fn default() -> Self {
        Self {
            resolution: AvchdResolution::R1080p,
            frame_rate: FrameRate {
                num: 24_000,
                den: 1_001,
            },
            level: H264Level::Level41,
            bluray_compat: true,
            vbv_maxrate_kbps: 24_000,
            vbv_bufsize_kbps: 24_000,
            keyint_frames: 24,
            slices: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub enum AvchdResolution {
    #[serde(rename = "1080p")]
    R1080p,
    #[serde(rename = "1080i")]
    R1080i,
    #[serde(rename = "720p")]
    R720p,
    #[serde(rename = "480i")]
    Sd480i,
    #[serde(rename = "576i")]
    Sd576i,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum H264Level {
    Level41,
    Level42,
}

impl AvchdResolution {
    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::R1080p | Self::R1080i => (1920, 1080),
            Self::R720p => (1280, 720),
            Self::Sd480i => (720, 480),
            Self::Sd576i => (720, 576),
        }
    }

    pub const fn interlaced(self) -> bool {
        matches!(self, Self::R1080i | Self::Sd480i | Self::Sd576i)
    }

    pub fn allowed_frame_rates(self) -> &'static [FrameRate] {
        match self {
            Self::R1080p => &[FrameRate::FPS_23_976, FrameRate::FPS_24],
            Self::R1080i => &[FrameRate::FPS_29_97, FrameRate::PAL],
            Self::R720p => &[
                FrameRate::FPS_23_976,
                FrameRate::FPS_50,
                FrameRate::FPS_59_94,
            ],
            Self::Sd480i => &[FrameRate::FPS_29_97],
            Self::Sd576i => &[FrameRate::PAL],
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::R1080p => "1080p",
            Self::R1080i => "1080i",
            Self::R720p => "720p",
            Self::Sd480i => "480i",
            Self::Sd576i => "576i",
        }
    }
}

impl AvchdVideoConfig {
    /// Compatibility rules for AVCHD high-definition streams.
    pub fn validate(&self) -> Vec<Diagnostic> {
        self.validate_for_capacity(DiscCapacity::Dvd5)
    }

    /// Reglas de codificación según el medio final. Los discos Blu-ray
    /// desbloquean hasta 38 Mbps y un VBV de 30.000–40.000 kbits.
    pub fn validate_for_capacity(&self, capacity: DiscCapacity) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let blu_ray = capacity.is_blu_ray();
        let max_rate = if blu_ray { 38_000 } else { 24_000 };
        if !self.frame_rate.is_valid() {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_FRAME_RATE",
                "El framerate de alta definición no es válido.",
            ));
            return diagnostics;
        }
        if !self
            .resolution
            .allowed_frame_rates()
            .contains(&self.frame_rate)
        {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_FPS_INCOMPATIBLE",
                format!(
                    "{} no admite {} fps. Consulta las combinaciones permitidas.",
                    self.resolution.label(),
                    self.frame_rate.as_f64()
                ),
            ));
        }
        let level_ok =
            self.level == H264Level::Level41 || (blu_ray && self.level == H264Level::Level42);
        if !level_ok {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_LEVEL",
                if blu_ray {
                    "Blu-ray admite H.264 High@Level 4.1 o 4.2."
                } else {
                    "AVCHD exige H.264 High@Level 4.1."
                },
            ));
        }
        if self.vbv_maxrate_kbps == 0 || self.vbv_maxrate_kbps > max_rate {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_VBV_MAXRATE",
                format!(
                    "El VBV maxrate debe estar entre 1 y {max_rate} kbps para {}.",
                    capacity.label()
                ),
            ));
        } else if !blu_ray && self.vbv_maxrate_kbps != 24_000 {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_VBV_MAXRATE",
                "Se normalizará el VBV maxrate a 24.000 kbps para compatibilidad AVCHD.",
            ));
        }
        if blu_ray {
            if !(30_000..=40_000).contains(&self.vbv_bufsize_kbps) {
                diagnostics.push(Diagnostic::warning(
                    "W_BD_VBV_BUFSIZE",
                    "Blu-ray recomienda un VBV bufsize entre 30.000 y 40.000 kbits.",
                ));
            }
        } else if self.vbv_bufsize_kbps == 0 || self.vbv_bufsize_kbps > 24_000 {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_VBV_BUFSIZE",
                "El VBV bufsize AVCHD debe estar entre 1 y 24.000 kbps.",
            ));
        } else if self.vbv_bufsize_kbps != 24_000 {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_VBV_BUFSIZE",
                "Se normalizará el VBV bufsize a 24.000 kbps para compatibilidad AVCHD.",
            ));
        }
        if self.slices != 4 {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_SLICES",
                "La codificación se realiza con 4 slices por fotograma.",
            ));
        }
        if self.keyint_frames == 0 {
            diagnostics.push(Diagnostic::error(
                "E_AVCHD_KEYINT",
                "El intervalo de keyframes no puede ser cero.",
            ));
        } else if self.keyint_frames as f64 > self.frame_rate.as_f64().ceil() {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_KEYINT",
                "El GOP debería durar como máximo un segundo.",
            ));
        }
        if !self.bluray_compat {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_BLURAY_COMPAT",
                "Se forzará bluray-compat=1 para la salida de alta definición.",
            ));
        }
        diagnostics
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Subtitle,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AssetSource {
    Project,
    External { absolute_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: AssetId,
    pub display_name: String,
    pub source: AssetSource,
    #[serde(default)]
    pub relative_path: Option<String>,
    pub sha256: String,
    pub bytes: u64,
    pub kind: MediaKind,
    pub probe: MediaProbe,
    #[serde(default)]
    pub proxy: Option<ProxyInfo>,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbe {
    pub format_name: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub bitrate_bps: Option<u64>,
    #[serde(default)]
    pub video: Option<VideoStreamInfo>,
    #[serde(default)]
    pub audio: Vec<AudioStreamInfo>,
    #[serde(default)]
    pub subtitle_streams: u32,
    #[serde(default)]
    pub subtitle_tracks: Vec<SubtitleStreamInfo>,
    #[serde(default)]
    pub chapters: Vec<MediaChapter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MediaChapter {
    pub start_ms: u64,
    #[serde(default)]
    pub end_ms: Option<u64>,
    pub title: String,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStreamInfo {
    pub index: u32,
    pub codec: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
    #[serde(default)]
    pub bitmap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoStreamInfo {
    pub index: u32,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    #[serde(default)]
    pub sample_aspect_ratio: Option<String>,
    #[serde(default)]
    pub display_aspect_ratio: Option<String>,
    #[serde(default)]
    pub pix_fmt: Option<String>,
    #[serde(default)]
    pub field_order: Option<String>,
    #[serde(default)]
    pub bit_depth: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioStreamInfo {
    pub index: u32,
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
    #[serde(default)]
    pub bitrate_bps: Option<u64>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub commentary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum VideoScaleMode {
    #[default]
    Letterbox,
    CropFill,
    Stretch,
    /// Sinónimo explícito de `Letterbox` usado por la UI avanzada.
    Contain,
    /// Sinónimo explícito de `CropFill` usado por la UI avanzada.
    Cover,
}

impl VideoScaleMode {
    pub const fn is_letterbox(self) -> bool {
        matches!(self, Self::Letterbox | Self::Contain)
    }

    pub const fn is_crop_fill(self) -> bool {
        matches!(self, Self::CropFill | Self::Cover)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EncodingPriority {
    Low,
    #[default]
    Medium,
    High,
}

impl EncodingPriority {
    /// Peso relativo aplicado por el planificador VBR.
    pub const fn weight(self) -> f64 {
        match self {
            Self::Low => 0.75,
            Self::Medium => 1.0,
            Self::High => 1.25,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Low => "Baja",
            Self::Medium => "Media",
            Self::High => "Alta",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub id: EpisodeId,
    pub number: u16,
    pub title: String,
    pub video: AssetId,
    #[serde(default)]
    pub trim: Option<FrameRange>,
    #[serde(default)]
    pub trim_start_ms: Option<u64>,
    #[serde(default)]
    pub trim_end_ms: Option<u64>,
    #[serde(default)]
    pub scale_mode: VideoScaleMode,
    #[serde(default)]
    pub thumbnail_time_ms: Option<u64>,
    #[serde(default)]
    pub thumbnail_path: Option<String>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    #[serde(default)]
    pub audio: Vec<EpisodeAudio>,
    #[serde(default)]
    pub subtitles: Vec<SubtitleTrack>,
    #[serde(default)]
    pub encoding_priority: EncodingPriority,
    #[serde(default)]
    pub post_action: PostAction,
    #[serde(default)]
    pub encode_override: Option<EncodeOverride>,
}

impl Episode {
    /// Duración efectiva cuando el editor definió `trim_end_ms`.
    /// Si no hay fin explícito, devuelve 0; el planificador usa la sonda del
    /// medio como fallback para no romper proyectos antiguos.
    pub fn effective_duration_ms(&self) -> u64 {
        let start = self.trim_start_ms.unwrap_or(0);
        let end = self.trim_end_ms.unwrap_or(start);
        end.saturating_sub(start)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub id: ChapterId,
    pub title: String,
    pub at_frame: u64,
    #[serde(default, deserialize_with = "crate::ids::deserialize_optional_id")]
    #[specta(type = Option<AssetId>)]
    pub thumbnail: Option<AssetId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AudioFormat {
    #[default]
    Ac3Stereo,
    Ac3Surround51,
}

impl AudioFormat {
    pub const fn channels(self) -> u8 {
        match self {
            Self::Ac3Stereo => 2,
            Self::Ac3Surround51 => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ChannelMode {
    #[default]
    Source,
    Stereo,
    Surround51,
    Upmix51,
    DownmixStereo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AudioCodec {
    Ac3,
    Dts,
    Lpcm,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LoudnessTarget {
    pub target_lufs: f32,
    pub true_peak_db: f32,
    pub lra: f32,
}

impl Default for LoudnessTarget {
    fn default() -> Self {
        Self {
            target_lufs: -16.0,
            true_peak_db: -1.5,
            lra: 11.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AudioMastering {
    #[serde(default)]
    pub loudness: Option<LoudnessTarget>,
    #[serde(default)]
    pub channel_mode: ChannelMode,
    #[serde(default)]
    pub codec: Option<AudioCodec>,
    #[serde(default)]
    pub passthrough: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeAudio {
    pub source_stream: u32,
    /// Ruta opcional para audio externo; vacío significa usar el stream del vídeo.
    #[serde(default)]
    pub source_path: Option<String>,
    pub language: String,
    pub label: String,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default = "default_ac3_kbps")]
    pub ac3_kbps: u16,
    #[serde(default)]
    pub delay_ms: i32,
    #[serde(default)]
    pub format: AudioFormat,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub commentary: bool,
    #[serde(default)]
    pub mastering: AudioMastering,
}

fn default_ac3_kbps() -> u16 {
    192
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SubtitleAlignment {
    #[default]
    BottomCenter,
    BottomLeft,
    BottomRight,
    TopCenter,
    Center,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleStyle {
    #[serde(default = "default_subtitle_font")]
    pub font_family: String,
    #[serde(default)]
    pub font_path: Option<String>,
    #[serde(default = "default_subtitle_weight")]
    pub font_weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default = "default_subtitle_size")]
    pub font_size_percent: f32,
    #[serde(default = "default_subtitle_outline")]
    pub outline_percent: f32,
    #[serde(default = "default_subtitle_shadow")]
    pub shadow_offset_percent: f32,
    #[serde(default = "default_subtitle_fill")]
    pub fill_color: String,
    #[serde(default = "default_subtitle_outline_color")]
    pub outline_color: String,
    #[serde(default = "default_subtitle_shadow_color")]
    pub shadow_color: String,
    #[serde(default)]
    pub alignment: SubtitleAlignment,
    #[serde(default = "default_subtitle_safe_area")]
    pub safe_area_percent: u8,
    #[serde(default = "default_subtitle_max_lines")]
    pub max_lines: u8,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font_family: default_subtitle_font(),
            font_path: None,
            font_weight: default_subtitle_weight(),
            italic: false,
            font_size_percent: default_subtitle_size(),
            outline_percent: default_subtitle_outline(),
            shadow_offset_percent: default_subtitle_shadow(),
            fill_color: default_subtitle_fill(),
            outline_color: default_subtitle_outline_color(),
            shadow_color: default_subtitle_shadow_color(),
            alignment: SubtitleAlignment::BottomCenter,
            safe_area_percent: default_subtitle_safe_area(),
            max_lines: default_subtitle_max_lines(),
        }
    }
}

fn default_subtitle_font() -> String {
    "sans-serif".to_string()
}
fn default_subtitle_weight() -> u16 {
    400
}
fn default_subtitle_size() -> f32 {
    4.5
}
fn default_subtitle_outline() -> f32 {
    0.18
}
fn default_subtitle_shadow() -> f32 {
    0.12
}
fn default_subtitle_fill() -> String {
    "#FFFFFF".to_string()
}
fn default_subtitle_outline_color() -> String {
    "#000000".to_string()
}
fn default_subtitle_shadow_color() -> String {
    "#000000A0".to_string()
}
fn default_subtitle_safe_area() -> u8 {
    5
}
fn default_subtitle_max_lines() -> u8 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    pub source_stream: u32,
    /// Ruta opcional a un `.srt`/`.ass` externo.
    #[serde(default)]
    pub source_path: Option<String>,
    pub language: String,
    pub label: String,
    #[serde(default)]
    pub delay_ms: i32,
    #[serde(default)]
    pub is_forced: bool,
    #[serde(default)]
    pub is_default: bool,
    /// PGS/VobSub/DVB bitmap subtitles cannot be converted to SRT without OCR.
    #[serde(default)]
    pub bitmap: bool,
    #[serde(default)]
    pub style: SubtitleStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PostAction {
    #[default]
    ReturnToMenu,
    NextEpisode,
    Stop,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EncodeOverride {
    #[serde(default)]
    pub video_kbps: Option<u32>,
    #[serde(default)]
    pub maxrate_kbps: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FirstPlay {
    pub asset: AssetId,
    #[serde(default)]
    pub trim: Option<FrameRange>,
    #[serde(default)]
    pub audio: Option<EpisodeAudio>,
    pub target_menu: MenuId,
    #[serde(default)]
    pub skippable: bool,
    #[serde(default)]
    pub encode_override: Option<EncodeOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PlayAllConfig {
    pub enabled: bool,
    #[serde(default)]
    pub order: Vec<EpisodeId>,
    #[serde(default = "default_mode_gprm")]
    pub mode_gprm: u8,
}

fn default_mode_gprm() -> u8 {
    1
}

impl Default for PlayAllConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            order: Vec::new(),
            mode_gprm: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BurnPreferences {
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub speed_x: Option<u16>,
    #[serde(default = "default_true")]
    pub verify: bool,
    #[serde(default = "default_true")]
    pub eject: bool,
    #[serde(default = "default_true")]
    pub close_session: bool,
}

fn default_true() -> bool {
    true
}

impl Default for BurnPreferences {
    fn default() -> Self {
        Self {
            device: None,
            speed_x: None,
            verify: true,
            eject: true,
            close_session: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub schema_version: u32,
    pub id: ProjectId,
    pub name: String,
    pub created_at: String,
    pub modified_at: String,
    pub app_version: String,
    pub disc: DiscConfig,
    #[serde(default)]
    pub assets: Vec<MediaAsset>,
    #[serde(default)]
    pub first_play: Option<FirstPlay>,
    #[serde(default)]
    pub menus: Vec<Menu>,
    pub root_menu: MenuId,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    #[serde(default)]
    pub play_all: PlayAllConfig,
    #[serde(default)]
    pub budget: BudgetPolicy,
    #[serde(default)]
    pub burn: BurnPreferences,
}

impl Project {
    /// Resolves the effective subtitle codec for a track. Embedded tracks use
    /// the video asset's probe; external `.sup` files are recognized by suffix.
    pub fn subtitle_codec(&self, episode: &Episode, track: &SubtitleTrack) -> Option<&str> {
        if track
            .source_path
            .as_deref()
            .is_some_and(|path| path.to_ascii_lowercase().ends_with(".sup"))
        {
            return Some("hdmv_pgs_subtitle");
        }
        if track.source_path.is_some() {
            return None;
        }
        self.asset(episode.video)?
            .probe
            .subtitle_tracks
            .iter()
            .find(|stream| stream.index == track.source_stream)
            .map(|stream| stream.codec.as_str())
    }

    /// Whether a subtitle track can be stream-copied as Blu-ray PGS.
    pub fn is_pgs_track(&self, episode: &Episode, track: &SubtitleTrack) -> bool {
        self.subtitle_codec(episode, track) == Some("hdmv_pgs_subtitle")
    }

    pub fn menu(&self, id: MenuId) -> Option<&Menu> {
        self.menus.iter().find(|menu| menu.id == id)
    }

    pub fn episode(&self, id: EpisodeId) -> Option<&Episode> {
        self.episodes.iter().find(|episode| episode.id == id)
    }

    pub fn asset(&self, id: AssetId) -> Option<&MediaAsset> {
        self.assets.iter().find(|asset| asset.id == id)
    }

    /// Structural validation. This is the authoritative check used before
    /// planning, rendering or exporting a project.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        if self.schema_version != 1 {
            diagnostics.push(
                Diagnostic::error(
                    "E_SCHEMA_VERSION",
                    format!(
                        "Versión de esquema {} no soportada; se esperaba 1.",
                        self.schema_version
                    ),
                )
                .with_entity(format!("project:{}", self.id)),
            );
        }
        if self.name.trim().is_empty() {
            diagnostics.push(Diagnostic::error(
                "E_PROJECT_NAME",
                "El proyecto no tiene nombre.",
            ));
        }
        if self.disc.volume_label.trim().is_empty() {
            diagnostics.push(Diagnostic::error(
                "E_VOLUME_LABEL_EMPTY",
                "La etiqueta del volumen no puede estar vacía.",
            ));
        } else if self.disc.volume_label.len() > 32 || !self.disc.volume_label.is_ascii() {
            diagnostics.push(Diagnostic::warning(
                "W_VOLUME_LABEL",
                "La etiqueta del volumen debería ser ASCII y tener como máximo 32 caracteres.",
            ));
        }

        let is_avchd = matches!(self.disc.mode, AuthoringMode::Avchd);
        if self.disc.capacity.is_blu_ray() && !is_avchd {
            diagnostics.push(Diagnostic::error(
                "E_BD_REQUIRES_HD",
                "BD-25/BD-50 requieren el modo de authoring AVCHD/Blu-ray HD.",
            ));
        }
        match (&self.disc.mode, &self.disc.video) {
            (AuthoringMode::DvdVideo, VideoConfig::Dvd(_)) => {}
            (AuthoringMode::Avchd, VideoConfig::Avchd(config)) => {
                diagnostics.extend(config.validate_for_capacity(self.disc.capacity));
            }
            _ => diagnostics.push(Diagnostic::error(
                "E_VIDEO_MODE_MISMATCH",
                "El modo de authoring y la configuración de video no coinciden.",
            )),
        }

        let mut asset_ids = HashSet::new();
        for asset in &self.assets {
            if !asset_ids.insert(asset.id) {
                diagnostics.push(
                    Diagnostic::error("E_DUPLICATE_ASSET", "ID de asset duplicado.")
                        .with_entity(format!("asset:{}", asset.id)),
                );
            }
        }

        let mut menu_ids = HashSet::new();
        for menu in &self.menus {
            if !menu_ids.insert(menu.id) {
                diagnostics.push(
                    Diagnostic::error("E_DUPLICATE_MENU", "ID de menú duplicado.")
                        .with_entity(format!("menu:{}", menu.id)),
                );
            }
            validate_menu(self, menu, &mut diagnostics);
        }

        let mut episode_ids = HashSet::new();
        for episode in &self.episodes {
            if !episode_ids.insert(episode.id) {
                diagnostics.push(
                    Diagnostic::error("E_DUPLICATE_EPISODE", "ID de episodio duplicado.")
                        .with_entity(format!("episode:{}", episode.id)),
                );
            }
            if self.asset(episode.video).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_EPISODE_VIDEO_MISSING",
                        "El episodio referencia un asset de video inexistente.",
                    )
                    .with_entity(format!("episode:{}", episode.id)),
                );
            }
            if let Some(trim) = episode.trim {
                if !trim.is_valid() {
                    diagnostics.push(
                        Diagnostic::error(
                            "E_EPISODE_TRIM",
                            "El rango de recorte del episodio no es válido.",
                        )
                        .with_entity(format!("episode:{}", episode.id)),
                    );
                }
            }
            if let (Some(start), Some(end)) = (episode.trim_start_ms, episode.trim_end_ms) {
                if end <= start {
                    diagnostics.push(
                        Diagnostic::error(
                            "E_EPISODE_TRIM_MS",
                            "El fin del recorte debe ser posterior al inicio.",
                        )
                        .with_entity(format!("episode:{}", episode.id)),
                    );
                }
            }
            for (index, chapter) in episode.chapters.iter().enumerate() {
                if chapter.at_frame >= episode.trim.map(|trim| trim.duration()).unwrap_or(u64::MAX)
                {
                    diagnostics.push(
                        Diagnostic::warning(
                            "W_CHAPTER_OUT_OF_RANGE",
                            "El capítulo está fuera del rango del episodio.",
                        )
                        .with_entity(format!("chapter:{index}:{}", chapter.id)),
                    );
                }
            }
            for subtitle in &episode.subtitles {
                let bitmap = subtitle.bitmap || self.is_pgs_track(episode, subtitle);
                let pgs_passthrough = is_avchd && self.is_pgs_track(episode, subtitle);
                if bitmap && subtitle.source_path.is_none() && !pgs_passthrough {
                    diagnostics.push(
                        Diagnostic::warning(
                            "W_BITMAP_SUBTITLE_UNSUPPORTED",
                            "Los subtítulos bitmap (PGS/VobSub/DVB) no se pueden convertir a SRT sin OCR; la pista se omitirá.",
                        )
                        .with_entity(format!("subtitle:{}", subtitle.source_stream)),
                    );
                }
            }
        }

        if !is_avchd && self.menus.is_empty() {
            diagnostics.push(Diagnostic::error(
                "E_NO_MENUS",
                "El proyecto no tiene menús.",
            ));
        }
        if self.episodes.is_empty() {
            diagnostics.push(Diagnostic::error(
                "E_NO_EPISODES",
                "El proyecto no tiene episodios.",
            ));
        }

        if !is_avchd || !self.menus.is_empty() {
            match self.menu(self.root_menu) {
                None => diagnostics.push(
                    Diagnostic::error(
                        "E_ROOT_MENU_MISSING",
                        "El menú raíz no existe en el proyecto.",
                    )
                    .with_entity(format!("menu:{}", self.root_menu)),
                ),
                Some(menu) => {
                    if menu.role != MenuRole::Root {
                        diagnostics.push(
                            Diagnostic::error(
                                "E_ROOT_MENU_ROLE",
                                "El menú raíz debe tener rol root.",
                            )
                            .with_entity(format!("menu:{}", menu.id)),
                        );
                    }
                    if menu.placement != MenuPlacement::Vmgm {
                        diagnostics.push(
                            Diagnostic::error(
                                "E_ROOT_MENU_PLACEMENT",
                                "El menú raíz debe vivir en VMGM.",
                            )
                            .with_entity(format!("menu:{}", menu.id)),
                        );
                    }
                }
            }
        }

        if !is_avchd {
            if let Some(first_play) = &self.first_play {
                if self.asset(first_play.asset).is_none() {
                    diagnostics.push(Diagnostic::error(
                        "E_FIRST_PLAY_ASSET_MISSING",
                        "El First Play referencia un asset inexistente.",
                    ));
                }
                if self.menu(first_play.target_menu).is_none() {
                    diagnostics.push(Diagnostic::error(
                        "E_FIRST_PLAY_MENU_MISSING",
                        "El First Play apunta a un menú inexistente.",
                    ));
                }
            }
        } else if self.first_play.is_some() {
            diagnostics.push(Diagnostic::warning(
                "W_AVCHD_FIRST_PLAY_IGNORED",
                "Blu-ray/AVCHD no usan First Play; el campo se ignora.",
            ));
        }

        if self.play_all.enabled && self.play_all.order.is_empty() {
            diagnostics.push(Diagnostic::warning(
                "W_PLAY_ALL_EMPTY",
                "Reproducir todo está activo pero no tiene episodios en el orden.",
            ));
        }
        for episode in &self.play_all.order {
            if self.episode(*episode).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_PLAY_ALL_EPISODE_MISSING",
                        "El orden de Reproducir todo referencia un episodio inexistente.",
                    )
                    .with_entity(format!("episode:{episode}")),
                );
            }
        }
        if self.play_all.mode_gprm > 15 {
            diagnostics.push(Diagnostic::error(
                "E_GPRM_RANGE",
                "El GPRM de modo debe estar entre 0 y 15.",
            ));
        }

        diagnostics.extend(self.budget.validate());
        diagnostics
    }
}

fn validate_menu(project: &Project, menu: &Menu, diagnostics: &mut Vec<Diagnostic>) {
    let menu_entity = format!("menu:{}", menu.id);
    if menu.name.trim().is_empty() {
        diagnostics.push(
            Diagnostic::error("E_MENU_NAME", "El menú no tiene nombre.")
                .with_entity(menu_entity.clone()),
        );
    }
    if menu.canvas.tv_standard != project.disc.standard {
        diagnostics.push(
            Diagnostic::warning(
                "W_MENU_STANDARD_MISMATCH",
                "El lienzo del menú no coincide con el estándar del disco.",
            )
            .with_entity(menu_entity.clone()),
        );
    }
    if menu.canvas.aspect != project.disc.aspect {
        diagnostics.push(
            Diagnostic::warning(
                "W_MENU_ASPECT_MISMATCH",
                "El aspecto del menú no coincide con el del disco.",
            )
            .with_entity(menu_entity.clone()),
        );
    }

    match (menu.role, menu.placement) {
        (MenuRole::Root, MenuPlacement::Vmgm) => {}
        (
            MenuRole::EpisodeSelect | MenuRole::ChapterSelect | MenuRole::Submenu,
            MenuPlacement::Vtsm { titleset: 0 },
        ) => {
            diagnostics.push(
                Diagnostic::warning("W_TITLESET_ZERO", "El titleset 0 no es válido.")
                    .with_entity(menu_entity.clone()),
            );
        }
        (
            MenuRole::EpisodeSelect | MenuRole::ChapterSelect | MenuRole::Submenu,
            MenuPlacement::Vtsm { .. },
        ) => {}
        (MenuRole::Root, _) => diagnostics.push(
            Diagnostic::error("E_ROOT_MENU_PLACEMENT", "El menú raíz debe estar en VMGM.")
                .with_entity(menu_entity.clone()),
        ),
        (_, MenuPlacement::Vmgm) => diagnostics.push(
            Diagnostic::warning(
                "W_MENU_PLACEMENT",
                "Los menús de selección deberían estar en el VTSM del titleset.",
            )
            .with_entity(menu_entity.clone()),
        ),
    }

    if menu.dim_percent > 80 {
        diagnostics.push(
            Diagnostic::error(
                "E_MENU_DIM_RANGE",
                "El oscurecimiento del fondo debe estar entre 0 y 80 %.",
            )
            .with_entity(menu_entity.clone()),
        );
    }
    if menu.blur_px > 20 {
        diagnostics.push(
            Diagnostic::error(
                "E_MENU_BLUR_RANGE",
                "El desenfoque de fondo debe estar entre 0 y 20 px.",
            )
            .with_entity(menu_entity.clone()),
        );
    }

    if menu.buttons.len() > 36 {
        diagnostics.push(
            Diagnostic::error(
                "E_TOO_MANY_BUTTONS",
                "Un menú DVD no puede tener más de 36 botones.",
            )
            .with_entity(menu_entity.clone()),
        );
    }

    match &menu.background {
        crate::menu::MenuBackground::Static { asset }
        | crate::menu::MenuBackground::VideoLoop { asset, .. } => {
            if project.asset(*asset).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_MENU_BACKGROUND_ASSET",
                        "El fondo del menú referencia un asset inexistente.",
                    )
                    .with_entity(menu_entity.clone()),
                );
            }
        }
    }
    if let Some(audio) = &menu.audio {
        if project.asset(audio.asset).is_none() {
            diagnostics.push(
                Diagnostic::error(
                    "E_MENU_AUDIO_ASSET",
                    "La música del menú referencia un asset inexistente.",
                )
                .with_entity(menu_entity.clone()),
            );
        }
    }
    for item in &menu.items {
        let asset = match item {
            MenuItem::Image(item) => Some(item.asset),
            MenuItem::VideoThumbnail(item) => Some(item.asset),
            MenuItem::Text(_) | MenuItem::Shape(_) => None,
        };
        if let Some(asset) = asset {
            if project.asset(asset).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_MENU_ITEM_ASSET",
                        "Un elemento del menú referencia un asset inexistente.",
                    )
                    .with_entity(menu_entity.clone()),
                );
            }
        }
    }

    let mut button_ids = HashSet::new();
    let mut button_names = HashSet::new();
    let mut rects = Vec::new();
    for button in &menu.buttons {
        let button_entity = format!("button:{}", button.id);
        if !button_ids.insert(button.id) {
            diagnostics.push(
                Diagnostic::error("E_DUPLICATE_BUTTON", "ID de botón duplicado.")
                    .with_entity(button_entity.clone()),
            );
        }
        if button.name.trim().is_empty() {
            diagnostics.push(
                Diagnostic::error("E_BUTTON_NAME", "El botón no tiene nombre.")
                    .with_entity(button_entity.clone()),
            );
        } else if !button_names.insert(button.name.clone()) {
            diagnostics.push(
                Diagnostic::error("E_DUPLICATE_BUTTON_NAME", "Nombre de botón duplicado.")
                    .with_entity(button_entity.clone()),
            );
        }
        if let Some(thumbnail) = &button.thumbnail {
            if project.asset(thumbnail.asset).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_BUTTON_THUMBNAIL_ASSET",
                        "La miniatura del botón referencia un asset inexistente.",
                    )
                    .with_entity(button_entity.clone()),
                );
            }
        }
        if button.font_size == 0 {
            diagnostics.push(
                Diagnostic::error(
                    "E_BUTTON_FONT_SIZE",
                    "El tamaño de fuente debe ser mayor que cero.",
                )
                .with_entity(button_entity.clone()),
            );
        }
        if !matches!(button.font_weight.as_str(), "normal" | "bold") {
            diagnostics.push(
                Diagnostic::warning(
                    "W_BUTTON_FONT_WEIGHT",
                    "El peso de fuente debería ser normal o bold.",
                )
                .with_entity(button_entity.clone()),
            );
        }
        for (label, color) in [
            ("text", &button.text_color),
            ("shadow", &button.shadow_color),
            ("fill", &button.fill_color),
        ] {
            if !is_hex_color(color) {
                diagnostics.push(
                    Diagnostic::warning(
                        "W_BUTTON_COLOR",
                        format!("El color {label} «{color}» no es un hex #RRGGBB válido."),
                    )
                    .with_entity(button_entity.clone()),
                );
            }
        }
        match normalize_rect(button.rect, menu.canvas, 8) {
            Ok(rect) => rects.push(rect),
            Err(diagnostic) => diagnostics.push(diagnostic.with_entity(button_entity.clone())),
        }
        validate_action(project, menu, button, diagnostics, &button_entity);
    }

    diagnostics.extend(validate_layout(&rects, 2, menu.canvas, 36));

    for color in menu.subpicture.palette.colors {
        if color.alpha > 15 {
            diagnostics.push(
                Diagnostic::error(
                    "E_SUBPICTURE_ALPHA",
                    "La opacidad de la paleta debe estar entre 0 y 15.",
                )
                .with_entity(menu_entity.clone()),
            );
        }
    }
    if menu.subpicture.highlight_opacity > 15 || menu.subpicture.select_opacity > 15 {
        diagnostics.push(
            Diagnostic::error(
                "E_SUBPICTURE_OPACITY",
                "La opacidad de highlight/select debe estar entre 0 y 15.",
            )
            .with_entity(menu_entity),
        );
    }
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn validate_action(
    project: &Project,
    menu: &Menu,
    button: &crate::menu::MenuButton,
    diagnostics: &mut Vec<Diagnostic>,
    entity: &str,
) {
    match &button.action {
        ButtonAction::PlayEpisode { episode } => {
            if project.episode(*episode).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_ACTION_EPISODE_MISSING",
                        "El botón referencia un episodio inexistente.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::GotoMenu { menu: target } => {
            if project.menu(*target).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        "E_ACTION_MENU_MISSING",
                        "El botón referencia un menú inexistente.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::JumpTitleChapter { episode, chapter } => {
            match project.episode(*episode) {
                None => diagnostics.push(
                    Diagnostic::error(
                        "E_ACTION_EPISODE_MISSING",
                        "El salto a capítulo referencia un episodio inexistente.",
                    )
                    .with_entity(entity.to_string()),
                ),
                Some(episode) if *chapter as usize >= episode.chapters.len() => {
                    diagnostics.push(
                        Diagnostic::error(
                            "E_ACTION_CHAPTER_MISSING",
                            "El salto a capítulo referencia un capítulo inexistente.",
                        )
                        .with_entity(entity.to_string()),
                    );
                }
                Some(_) => {}
            }
            if !matches!(menu.placement, MenuPlacement::Vtsm { .. })
                && !matches!(project.disc.mode, AuthoringMode::Avchd)
            {
                diagnostics.push(
                    Diagnostic::warning(
                        "W_ACTION_CHAPTER_VMGM",
                        "El salto a capítulo desde VMGM requiere un comando de título completo.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::SetAudioTrack { .. } => {
            if !matches!(menu.placement, MenuPlacement::Vtsm { .. })
                && !matches!(project.disc.mode, AuthoringMode::Avchd)
            {
                diagnostics.push(
                    Diagnostic::warning(
                        "W_ACTION_AUDIO_VMGM",
                        "El cambio de pista de audio desde VMGM puede no aplicarse en todos los reproductores.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::PlayFirstPlay => {
            if project.first_play.is_none() {
                diagnostics.push(
                    Diagnostic::warning(
                        "W_ACTION_NO_FIRST_PLAY",
                        "El botón reproduce el First Play pero el proyecto no tiene intro.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::PlayAll => {
            if !project.play_all.enabled {
                diagnostics.push(
                    Diagnostic::warning(
                        "W_ACTION_PLAY_ALL_DISABLED",
                        "El botón Reproducir todo está activo pero la función está deshabilitada.",
                    )
                    .with_entity(entity.to_string()),
                );
            }
        }
        ButtonAction::Resume | ButtonAction::Noop => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::BudgetPolicy;
    use crate::diagnostic::Severity;
    use crate::geometry::CanvasSpec;
    use crate::menu::{MenuBackground, MenuTiming, SubpictureSpec};

    fn minimal_project() -> Project {
        let asset_id = AssetId::from_u128(2);
        let menu_id = MenuId::from_u128(3);
        let episode_id = EpisodeId::from_u128(4);
        let project_id = ProjectId::from_u128(1);
        Project {
            schema_version: 1,
            id: project_id,
            name: "Test".into(),
            created_at: "2026-10-23T00:00:00Z".into(),
            modified_at: "2026-10-23T00:00:00Z".into(),
            app_version: "0.1.0".into(),
            disc: DiscConfig {
                mode: AuthoringMode::DvdVideo,
                standard: TvStandard::Ntsc,
                aspect: AspectRatio::FourThree,
                capacity: DiscCapacity::Dvd5,
                volume_label: "TEST".into(),
                region_mask: 0,
                layer_break_mode: LayerBreakMode::Otp,
                video: VideoConfig::Dvd(DvdVideoConfig::default()),
                audio: AudioDefaults::default(),
            },
            assets: vec![MediaAsset {
                id: asset_id,
                display_name: "episode.mp4".into(),
                source: AssetSource::Project,
                relative_path: Some("assets/episode.mp4".into()),
                sha256: "00".into(),
                bytes: 1,
                kind: MediaKind::Video,
                probe: MediaProbe {
                    format_name: "mp4".into(),
                    duration_ms: Some(120_000),
                    size_bytes: Some(1),
                    bitrate_bps: None,
                    video: None,
                    audio: vec![],
                    subtitle_streams: 0,
                    subtitle_tracks: Vec::new(),
                    chapters: Vec::new(),
                },
                proxy: None,
                imported_at: "2026-10-23T00:00:00Z".into(),
            }],
            first_play: None,
            menus: vec![Menu {
                id: menu_id,
                name: "Root".into(),
                role: MenuRole::Root,
                placement: MenuPlacement::Vmgm,
                canvas: CanvasSpec::default(),
                background: MenuBackground::Static { asset: asset_id },
                scale_mode: crate::menu::MenuScaleMode::Cover,
                dim_percent: 0,
                blur_px: 0,
                audio: None,
                motion_config: None,
                items: vec![],
                buttons: vec![],
                subpicture: SubpictureSpec::default(),
                timing: MenuTiming::default(),
            }],
            root_menu: menu_id,
            episodes: vec![Episode {
                id: episode_id,
                number: 1,
                title: "Episode".into(),
                video: asset_id,
                trim: None,
                trim_start_ms: None,
                trim_end_ms: None,
                scale_mode: VideoScaleMode::Letterbox,
                thumbnail_time_ms: None,
                thumbnail_path: None,
                chapters: vec![],
                audio: vec![],
                subtitles: vec![],
                encoding_priority: EncodingPriority::Medium,
                post_action: PostAction::ReturnToMenu,
                encode_override: None,
            }],
            play_all: PlayAllConfig::default(),
            budget: BudgetPolicy::default(),
            burn: BurnPreferences::default(),
        }
    }

    fn avchd_project() -> Project {
        let mut project = minimal_project();
        project.disc.mode = AuthoringMode::Avchd;
        project.disc.aspect = AspectRatio::SixteenNine;
        project.disc.video = VideoConfig::Avchd(AvchdVideoConfig::default());
        project.menus.clear();
        project.root_menu = MenuId::new();
        project
    }

    #[test]
    fn avchd_project_does_not_require_menus() {
        let diagnostics = avchd_project().validate();
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    fn pgs_track() -> SubtitleTrack {
        SubtitleTrack {
            source_stream: 2,
            source_path: None,
            language: "spa".into(),
            label: "PGS".into(),
            delay_ms: 0,
            is_forced: false,
            is_default: false,
            bitmap: true,
            style: SubtitleStyle::default(),
        }
    }

    #[test]
    fn blu_ray_pgs_is_accepted_while_dvd_still_warns() {
        let mut project = avchd_project();
        project.disc.capacity = DiscCapacity::Bd25;
        project.assets[0].probe.subtitle_tracks = vec![SubtitleStreamInfo {
            index: 2,
            codec: "hdmv_pgs_subtitle".into(),
            language: Some("spa".into()),
            title: Some("PGS".into()),
            default: false,
            forced: false,
            bitmap: true,
        }];
        project.episodes[0].subtitles = vec![pgs_track()];
        let diagnostics = project.validate();
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "W_BITMAP_SUBTITLE_UNSUPPORTED"),
            "{diagnostics:?}"
        );

        project.disc.mode = AuthoringMode::DvdVideo;
        project.disc.capacity = DiscCapacity::Dvd5;
        project.disc.video = VideoConfig::Dvd(DvdVideoConfig::default());
        let diagnostics = project.validate();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "W_BITMAP_SUBTITLE_UNSUPPORTED"));
    }

    #[test]
    fn avchd_1080p_rejects_2997_fps() {
        let mut project = avchd_project();
        if let VideoConfig::Avchd(config) = &mut project.disc.video {
            config.resolution = AvchdResolution::R1080p;
            config.frame_rate = FrameRate::FPS_29_97;
        }
        let diagnostics = project.validate();
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E_AVCHD_FPS_INCOMPATIBLE"));
    }

    #[test]
    fn avchd_1080i_accepts_25_fps_interlaced() {
        let mut project = avchd_project();
        if let VideoConfig::Avchd(config) = &mut project.disc.video {
            config.resolution = AvchdResolution::R1080i;
            config.frame_rate = FrameRate::PAL;
        }
        let diagnostics = project.validate();
        assert!(diagnostics.iter().all(|diagnostic| !diagnostic.is_error()));
    }

    #[test]
    fn minimal_project_is_valid() {
        let diagnostics = minimal_project().validate();
        let errors: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn blu_ray_capacity_requires_high_definition_mode() {
        let mut project = minimal_project();
        project.disc.capacity = DiscCapacity::Bd25;
        assert!(project
            .validate()
            .iter()
            .any(|diagnostic| diagnostic.code == "E_BD_REQUIRES_HD"));
    }

    #[test]
    fn avchd_project_accepts_hdmv_menus() {
        let mut project = minimal_project();
        project.disc.mode = AuthoringMode::Avchd;
        project.disc.capacity = DiscCapacity::Bd25;
        project.disc.aspect = AspectRatio::SixteenNine;
        project.disc.video = VideoConfig::Avchd(AvchdVideoConfig {
            vbv_maxrate_kbps: 38_000,
            vbv_bufsize_kbps: 35_000,
            ..AvchdVideoConfig::default()
        });
        project.first_play = None;
        let errors: Vec<_> = project
            .validate()
            .into_iter()
            .filter(Diagnostic::is_error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn episode_effective_duration_subtracts_trim() {
        let mut project = minimal_project();
        if let Some(episode) = project.episodes.first_mut() {
            episode.trim_start_ms = Some(5_000);
            episode.trim_end_ms = Some(95_000);
            assert_eq!(episode.effective_duration_ms(), 90_000);
        }
    }

    #[test]
    fn empty_required_and_optional_ids_deserialize_tolerantly() {
        let raw = include_str!("../../../fixtures/projects/minimal.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
        value["id"] = serde_json::json!("");
        value["rootMenu"] = serde_json::json!("");
        value["assets"][0]["id"] = serde_json::json!("");
        value["episodes"][0]["id"] = serde_json::json!("");
        value["episodes"][0]["video"] = serde_json::json!("");
        value["episodes"][0]["chapters"][0]["id"] = serde_json::json!("");
        value["episodes"][0]["chapters"][0]["thumbnail"] = serde_json::json!("");
        value["menus"][0]["id"] = serde_json::json!("");
        value["menus"][0]["background"]["asset"] = serde_json::json!("");
        value["menus"][0]["buttons"][0]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][0]["action"] =
            serde_json::json!({"kind": "playEpisode", "episode": ""});
        value["menus"][0]["buttons"][1]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][1]["action"]["menu"] = serde_json::json!("");
        value["menus"][0]["buttons"][1]["navOverride"] = serde_json::json!({
            "up": "",
            "down": null,
            "left": "  ",
            "right": ""
        });

        let project: Project =
            serde_json::from_value(value).expect("empty IDs must not abort deserialization");
        assert!(!project.id.as_uuid().is_nil());
        assert!(!project.root_menu.as_uuid().is_nil());
        assert!(!project.assets[0].id.as_uuid().is_nil());
        assert!(!project.episodes[0].id.as_uuid().is_nil());
        assert!(!project.episodes[0].video.as_uuid().is_nil());
        assert!(!project.episodes[0].chapters[0].id.as_uuid().is_nil());
        assert!(project.episodes[0].chapters[0].thumbnail.is_none());
        assert!(!project.menus[0].id.as_uuid().is_nil());
        assert!(!project.menus[0]
            .buttons
            .iter()
            .find(|button| button.name == "Episodios")
            .expect("goto menu button")
            .id
            .as_uuid()
            .is_nil());
        let nav = project.menus[0]
            .buttons
            .iter()
            .find(|button| button.name == "Episodios")
            .expect("goto menu button")
            .nav_override
            .clone();
        assert!(nav.up.is_none());
        assert!(nav.down.is_none());
        assert!(nav.left.is_none());
        assert!(nav.right.is_none());
        assert!(matches!(
            project.menus[0].buttons[0].action,
            ButtonAction::PlayEpisode { .. }
        ));
    }

    #[test]
    fn episode_effective_duration_without_end_is_zero() {
        let mut project = minimal_project();
        if let Some(episode) = project.episodes.first_mut() {
            episode.trim_start_ms = Some(5_000);
            episode.trim_end_ms = None;
            assert_eq!(episode.effective_duration_ms(), 0);
        }
    }

    #[test]
    fn chapter_range_uses_trim_duration_not_absolute_end() {
        let mut project = minimal_project();
        let episode = project.episodes.first_mut().expect("episode");
        episode.trim = Some(FrameRange {
            start: 1_000,
            end: 1_200,
        });
        episode.chapters.push(Chapter {
            id: ChapterId::from_u128(99),
            title: "Late".into(),
            at_frame: 250,
            thumbnail: None,
        });
        let diagnostics = project.validate();
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "W_CHAPTER_OUT_OF_RANGE" && diagnostic.severity == Severity::Warning
        }));
    }
}
