use crate::geometry::{normalize_rect, CanvasSpec, FrameRate, LogicalRect, PixelRect};
use crate::ids::{AssetId, ButtonId, EpisodeId, MenuId};
use crate::navigation::{resolve_navigation, NavPolicy, NavigationGraph};
use serde::{Deserialize, Serialize};

/// Where a menu lives in the DVD-Video virtual machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MenuPlacement {
    Vmgm,
    Vtsm { titleset: u8 },
}

/// Functional role of a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MenuRole {
    Root,
    EpisodeSelect,
    ChapterSelect,
    Submenu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum MenuScaleMode {
    #[default]
    Cover,
    Contain,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ButtonStyleKind {
    TextOnly,
    Hotspot,
    ThumbnailCard,
    GlassPill,
    #[default]
    ClassicFrame,
    PlayBadge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ButtonShapeMask {
    #[default]
    Rectangle,
    RoundedRect,
    Circle,
    Star,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ButtonMotionConfig {
    #[serde(default)]
    pub source_start_ms: u64,
    #[serde(default = "default_motion_loop_ms")]
    pub loop_duration_ms: u64,
}

fn default_motion_loop_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MotionMenuConfig {
    #[serde(default = "default_motion_duration_ms")]
    pub duration_ms: u64,
    #[serde(default = "default_true")]
    pub loop_enabled: bool,
    #[serde(default)]
    pub fade_in_ms: u64,
    #[serde(default)]
    pub fade_out_ms: u64,
    #[serde(default = "default_audio_volume")]
    pub audio_volume: f32,
}

fn default_motion_duration_ms() -> u64 {
    15_000
}

fn default_audio_volume() -> f32 {
    1.0
}

impl Default for MotionMenuConfig {
    fn default() -> Self {
        Self {
            duration_ms: default_motion_duration_ms(),
            loop_enabled: true,
            fade_in_ms: 500,
            fade_out_ms: 500,
            audio_volume: 1.0,
        }
    }
}

/// Visual menu definition. Rasterized layers are produced by the frontend and
/// sent to the backend separately in `MenuCompileRequest`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Menu {
    pub id: MenuId,
    pub name: String,
    pub role: MenuRole,
    pub placement: MenuPlacement,
    pub canvas: CanvasSpec,
    pub background: MenuBackground,
    #[serde(default)]
    pub scale_mode: MenuScaleMode,
    #[serde(default)]
    pub dim_percent: u8,
    #[serde(default)]
    pub blur_px: u8,
    #[serde(default)]
    pub audio: Option<MenuAudio>,
    #[serde(default)]
    pub motion_config: Option<MotionMenuConfig>,
    #[serde(default)]
    pub items: Vec<MenuItem>,
    #[serde(default)]
    pub buttons: Vec<MenuButton>,
    #[serde(default)]
    pub subpicture: SubpictureSpec,
    #[serde(default)]
    pub timing: MenuTiming,
}

impl Menu {
    pub fn normalized_button_rects(&self) -> Result<Vec<(ButtonId, PixelRect)>, crate::Diagnostic> {
        self.buttons
            .iter()
            .map(|button| {
                normalize_rect(button.rect, self.canvas, 8)
                    .map(|rect| (button.id, rect))
                    .map_err(|diagnostic| diagnostic.with_entity(format!("button:{}", button.id)))
            })
            .collect()
    }

    pub fn navigation(&self, default_focus: Option<ButtonId>) -> NavigationGraph {
        resolve_navigation(
            self.id,
            &self.canvas,
            &self.buttons,
            &NavPolicy::default(),
            default_focus,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MenuBackground {
    Static {
        asset: AssetId,
    },
    VideoLoop {
        asset: AssetId,
        #[serde(default)]
        in_frame: u64,
        #[serde(default)]
        out_frame: u64,
        #[serde(default)]
        loop_forever: bool,
        #[serde(default)]
        mute_source: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MenuAudio {
    pub asset: AssetId,
    #[serde(default)]
    pub gain_db: f32,
    #[serde(default)]
    pub fade_in_ms: u32,
    #[serde(default)]
    pub fade_out_ms: u32,
    #[serde(default)]
    pub loop_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MenuItem {
    Text(TextItem),
    Image(ImageItem),
    VideoThumbnail(VideoThumbnailItem),
    Shape(ShapeItem),
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TextItem {
    pub id: String,
    pub text: String,
    pub rect: LogicalRect,
    pub font_family: String,
    pub font_size: f32,
    pub color: String,
    #[serde(default)]
    pub align: TextAlign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImageItem {
    pub id: String,
    pub asset: AssetId,
    pub rect: LogicalRect,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct VideoThumbnailItem {
    pub id: String,
    pub asset: AssetId,
    pub rect: LogicalRect,
    #[serde(default)]
    pub in_frame: u64,
    #[serde(default)]
    pub duration_frames: u64,
    #[serde(default)]
    pub fps: FrameRate,
    #[serde(default)]
    pub loop_forever: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ShapeItem {
    pub id: String,
    pub rect: LogicalRect,
    pub fill: String,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default)]
    pub stroke_width: f32,
}

fn default_opacity() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MenuButton {
    pub id: ButtonId,
    pub name: String,
    pub rect: LogicalRect,
    #[serde(default)]
    pub z: i32,
    pub action: ButtonAction,
    #[serde(default)]
    pub thumbnail: Option<ThumbnailSpec>,
    #[serde(default)]
    pub label: Option<TextItem>,
    #[serde(default)]
    pub style_kind: ButtonStyleKind,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: u16,
    #[serde(default = "default_font_weight")]
    pub font_weight: String,
    #[serde(default = "default_text_color")]
    pub text_color: String,
    #[serde(default = "default_shadow_color")]
    pub shadow_color: String,
    #[serde(default = "default_fill_color")]
    pub fill_color: String,
    #[serde(default)]
    pub shape_mask: ButtonShapeMask,
    #[serde(default)]
    pub motion_preview: Option<ButtonMotionConfig>,
    #[serde(default)]
    pub nav_override: NavOverride,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_font_family() -> String {
    "sans-serif".to_string()
}

fn default_font_size() -> u16 {
    22
}

fn default_font_weight() -> String {
    "normal".to_string()
}

fn default_text_color() -> String {
    "#FFFFFF".to_string()
}

fn default_shadow_color() -> String {
    "#000000".to_string()
}

fn default_fill_color() -> String {
    "#0F172A".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailSpec {
    pub asset: AssetId,
    #[serde(default)]
    pub in_frame: u64,
    #[serde(default)]
    pub duration_frames: u64,
    #[serde(default)]
    pub fps: FrameRate,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NavOverride {
    #[serde(default, deserialize_with = "crate::ids::deserialize_optional_id")]
    #[specta(type = Option<ButtonId>)]
    pub up: Option<ButtonId>,
    #[serde(default, deserialize_with = "crate::ids::deserialize_optional_id")]
    #[specta(type = Option<ButtonId>)]
    pub down: Option<ButtonId>,
    #[serde(default, deserialize_with = "crate::ids::deserialize_optional_id")]
    #[specta(type = Option<ButtonId>)]
    pub left: Option<ButtonId>,
    #[serde(default, deserialize_with = "crate::ids::deserialize_optional_id")]
    #[specta(type = Option<ButtonId>)]
    pub right: Option<ButtonId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, specta::Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ButtonAction {
    PlayEpisode {
        episode: EpisodeId,
    },
    PlayAll,
    GotoMenu {
        menu: MenuId,
    },
    PlayFirstPlay,
    JumpTitleChapter {
        episode: EpisodeId,
        chapter: u32,
    },
    SetAudioTrack {
        track: u8,
    },
    Resume,
    #[default]
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubpictureSpec {
    #[serde(default)]
    pub palette: SubpicturePalette,
    #[serde(default = "default_true")]
    pub force: bool,
    #[serde(default)]
    pub auto_outline: bool,
    #[serde(default = "default_alpha")]
    pub highlight_opacity: u8,
    #[serde(default = "default_alpha")]
    pub select_opacity: u8,
}

fn default_alpha() -> u8 {
    15
}

impl Default for SubpictureSpec {
    fn default() -> Self {
        Self {
            palette: SubpicturePalette::default(),
            force: true,
            auto_outline: false,
            highlight_opacity: 15,
            select_opacity: 15,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubpicturePalette {
    pub colors: [SubpictureColor; 4],
}

impl Default for SubpicturePalette {
    fn default() -> Self {
        Self {
            colors: [
                SubpictureColor::new([0, 0, 0], 0),
                SubpictureColor::new([255, 255, 255], 15),
                SubpictureColor::new([255, 220, 0], 15),
                SubpictureColor::new([0, 120, 255], 15),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubpictureColor {
    pub rgb: [u8; 3],
    pub alpha: u8,
}

impl SubpictureColor {
    pub const fn new(rgb: [u8; 3], alpha: u8) -> Self {
        Self { rgb, alpha }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MenuTiming {
    pub duration_ms: u64,
    pub loop_playback: bool,
}

impl Default for MenuTiming {
    fn default() -> Self {
        Self {
            duration_ms: 30_000,
            loop_playback: true,
        }
    }
}
