//! Pure domain models and algorithms for DVD Forge.
//!
//! This crate deliberately contains no I/O and no process execution. Geometry,
//! navigation, project validation and capacity budgeting live here so they can
//! be tested without FFmpeg, dvdauthor or a display server.

pub mod budget;
pub mod diagnostic;
pub mod geometry;
pub mod ids;
pub mod language;
pub mod menu;
pub mod navigation;
pub mod project;

pub use budget::{AvchdPlanDetails, BitratePlan, BudgetPolicy, EpisodeBitrate, PlanFallback};
pub use diagnostic::{Diagnostic, Severity};
pub use geometry::{
    even_ceil, even_floor, has_gap, normalize_rect, validate_layout, AspectRatio, CanvasSpec,
    FrameRange, FrameRate, LogicalRect, PixelRect, Point, TvStandard,
};
pub use ids::{AssetId, ButtonId, ChapterId, EpisodeId, JobId, MenuId, PlanId, ProjectId};
pub use language::{LanguageCode, LanguageInfo};
pub use menu::{
    ButtonAction, ButtonMotionConfig, ButtonShapeMask, ButtonStyleKind, ImageItem, Menu, MenuAudio,
    MenuBackground, MenuButton, MenuItem, MenuPlacement, MenuRole, MenuScaleMode, MenuTiming,
    MotionMenuConfig, NavOverride, ShapeItem, SubpictureColor, SubpicturePalette, SubpictureSpec,
    TextAlign, TextItem, ThumbnailSpec, VideoThumbnailItem,
};
pub use navigation::{
    resolve_navigation, EdgeSource, NavDirection, NavEdge, NavNode, NavPolicy, NavigationGraph,
};
pub use project::{
    AssetSource, AudioCodec, AudioDefaults, AudioFormat, AudioMastering, AudioStreamInfo,
    AuthoringMode, AvchdResolution, AvchdVideoConfig, BurnPreferences, ChannelMode, Chapter,
    DiscCapacity, DiscConfig, DvdProfile, DvdVideoConfig, EncodeOverride, EncodingPriority,
    Episode, EpisodeAudio, FirstPlay, H264Level, LayerBreakMode, LoudnessTarget, MediaAsset,
    MediaChapter, MediaKind, MediaProbe, PlayAllConfig, PostAction, Project, ProxyInfo,
    SubtitleStreamInfo, SubtitleTrack, VideoConfig, VideoScaleMode, VideoStreamInfo,
};
