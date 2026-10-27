//! Reusable end-to-end DVD-Video render pipeline shared by the CLI and Tauri.

use dvdforge_avchd::{
    concat_mkv, write_meta, AvchdAudioConfig, AvchdEncodeRequest, AvchdEncoder, AvchdMetaRequest,
    AvchdOutputs, TsMuxer,
};
use dvdforge_domain::{
    normalize_rect, AssetId, AssetSource, AuthoringMode, ButtonAction, ButtonId, ButtonShapeMask,
    Diagnostic, DiscCapacity, DvdProfile, Episode, EpisodeId, FrameRate, JobId, MediaKind,
    MediaProbe, MenuBackground, MenuId, MenuScaleMode, PixelRect, Project, TvStandard, VideoConfig,
    VideoScaleMode,
};
use dvdforge_dvd::{
    build_dvdauthor_xml, AudioEncodeConfig, AuthoredAudioTrack, AuthoredButton,
    AuthoredSubtitleTrack, AuthoringRequest, DvdAuthor, DvdEncoder, EncodeOutputs, EncodeProgress,
    EncodeRequest, EpisodeFile, MenuEncodeRequest, MotionButtonEncode, MotionMenuEncodeRequest,
    RenderedMenu, VideoEncodeConfig,
};
use dvdforge_hdmv::{
    append_menu_movie_object, compile_ig_m2ts, compile_ig_pes, compile_ig_pes_packets,
    dummy_pgs_for_slots, frame_rate_code, inject_ig_into_menu_m2ts, jump_object, jump_title,
    movie_object_count, patch_menu_clpi, patch_menu_mpls, pes_transport_packet_count,
    set_audio_stream, set_gpr, set_index_menu_object, HdmvButton, HdmvButtonState, HdmvCommand,
    HdmvMenu, RgbaBitmap,
};
use dvdforge_iso::{BdmvIsoBuilder, Xorriso};
use dvdforge_jobs::{ArtifactKind, JobEventKind, JobStage};
use dvdforge_media::Ffprobe;
use dvdforge_menu::{
    generate_menu_layers, shift_srt, write_button_shape_mask, write_spumux_xml, write_textsub_xml,
    CompiledButton, MenuCompileRequest, MenuLayerRequest, Spumux, TextSubtitleSpec,
};
use dvdforge_plan::{calculate_bitrate_plan, BitratePlan};
use dvdforge_store::{ArtifactKey, Store};
use dvdforge_toolchain::{resolve_binary, MEDIA_PROTOCOL_WHITELIST};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

pub const CRATE_NAME: &str = "dvdforge-render";
pub type EventSink = mpsc::Sender<JobEventKind>;

/// tsMuxeR/BDAV timelines start at 600 seconds. Menu IG packets are aligned to
/// that origin so the interactive composition is valid on a real playlist.
pub const HDMV_MENU_PTS_90K: u64 = 54_000_000;

/// Upper bound for a single auxiliary FFmpeg invocation (menu clip, menu audio
/// or subtitle extraction). Encoding jobs have their own progress timeouts.
const AUX_FFMPEG_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Adds a local file input with an explicit protocol whitelist.
fn push_file_input(command: &mut Command, path: &Path) {
    command
        .arg("-protocol_whitelist")
        .arg(MEDIA_PROTOCOL_WHITELIST)
        .arg("-i")
        .arg(path);
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RenderOutcome {
    pub job_id: JobId,
    pub iso_path: String,
    pub elapsed_ms: u64,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("no se pudo sondear los medios: {0}")]
    Media(String),
    #[error("el plan de bitrate no es viable: {0}")]
    Plan(String),
    #[error("error de codificación: {0}")]
    Encode(String),
    #[error("error de menú: {0}")]
    Menu(String),
    #[error("error de authoring: {0}")]
    Author(String),
    #[error("error de ISO: {0}")]
    Iso(String),
    #[error("error de imagen: {0}")]
    Image(#[from] image::ImageError),
    #[error("render cancelado")]
    Cancelled,
}

/// Renders a project to a DVD-Video or AVCHD ISO. Progress is emitted as
/// `JobEventKind` values through `events`.
/// Options shared by the desktop and command-line render entry points.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Re-probe source files with FFprobe instead of trusting stored probes.
    pub probe_assets: bool,
    /// Rasterized Konva layers exported by the desktop editor.
    pub composited_backgrounds: Option<PathBuf>,
    /// Cooperative cancellation shared with the UDF writer. Child processes
    /// are killed on drop; this flag stops work between stages as well.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            probe_assets: true,
            composited_backgrounds: None,
            cancel: None,
        }
    }
}

pub async fn render_project(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    render_project_with_id(project, project_dir, output_dir, JobId::new(), events).await
}

/// Igual que [`render_project`] pero conserva el identificador generado por el
/// llamador, de modo que todos los `JobEvent` compartan el mismo `JobId`.
pub async fn render_project_with_id(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    job_id: JobId,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    render_project_with_options(
        project,
        project_dir,
        output_dir,
        job_id,
        RenderOptions::default(),
        events,
    )
    .await
}

/// Unified render entry point used by the CLI and the desktop app.
pub async fn render_project_with_options(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    job_id: JobId,
    options: RenderOptions,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    ensure_not_cancelled(&options.cancel)?;
    match project.disc.mode {
        AuthoringMode::DvdVideo => {
            render_dvd_project(project, project_dir, output_dir, job_id, options, events).await
        }
        AuthoringMode::Avchd => {
            render_avchd_project(project, project_dir, output_dir, job_id, options, events).await
        }
    }
}

fn ensure_not_cancelled(cancel: &Option<Arc<AtomicBool>>) -> Result<(), RenderError> {
    if cancel
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
    {
        Err(RenderError::Cancelled)
    } else {
        Ok(())
    }
}

/// Variante usada por la UI: permite sustituir el fondo de cada menú por la
/// composición rasterizada por el editor Konva (fondos, textos y miniaturas).
/// Las máscaras de subpicture se siguen generando con la paleta del proyecto.
pub async fn render_project_with_layers(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    job_id: JobId,
    composited_backgrounds: Option<PathBuf>,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    render_project_with_options(
        project,
        project_dir,
        output_dir,
        job_id,
        RenderOptions {
            composited_backgrounds,
            ..RenderOptions::default()
        },
        events,
    )
    .await
}

async fn render_dvd_project(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    job_id: JobId,
    options: RenderOptions,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    let started = Instant::now();
    let composited_backgrounds = options.composited_backgrounds.clone();
    let cancel = options.cancel.clone();

    stage(&events, JobStage::Probe, "Sondeando medios").await;
    let probes = collect_probes(&project, &project_dir, options.probe_assets).await;
    progress(&events, JobStage::Probe, 1000).await;

    stage(&events, JobStage::Plan, "Calculando bitrate").await;
    let plan = calculate_bitrate_plan(&project, &probes)
        .map_err(|error| RenderError::Plan(error.to_string()))?;
    for warning in &plan.warnings {
        let _ = events
            .send(JobEventKind::Warning {
                code: warning.code.clone(),
                message: warning.message.clone(),
            })
            .await;
    }
    if !plan.feasible {
        return Err(RenderError::Plan(
            "el plan no es viable; revisa las advertencias".to_string(),
        ));
    }
    progress(&events, JobStage::Plan, 1000).await;
    ensure_not_cancelled(&cancel)?;

    prepare_output(&output_dir)?;
    let encoder = DvdEncoder::default();

    stage(&events, JobStage::EncodeIntro, "Codificando intro").await;
    let intro = encode_intro(&project, &probes, &project_dir, &output_dir, &encoder).await?;
    progress(&events, JobStage::EncodeIntro, 1000).await;

    stage(&events, JobStage::EncodeEpisodes, "Codificando episodios").await;
    let episodes = encode_episodes(
        &project,
        &probes,
        &project_dir,
        &output_dir,
        &encoder,
        &plan,
        &events,
    )
    .await?;

    stage(
        &events,
        JobStage::EncodeMenus,
        "Compilando menús con spumux",
    )
    .await;
    let menus = render_menus(
        &project,
        &project_dir,
        &output_dir,
        &encoder,
        &events,
        composited_backgrounds.as_deref(),
    )
    .await?;

    ensure_not_cancelled(&cancel)?;
    stage(&events, JobStage::Author, "Compilando DVD con dvdauthor").await;
    let xml_path = output_dir.join("xml").join("dvdauthor.xml");
    let authoring = AuthoringRequest {
        project: &project,
        intro: intro.as_deref(),
        menus: &menus,
        episodes: &episodes,
    };
    let xml =
        build_dvdauthor_xml(&authoring).map_err(|error| RenderError::Author(error.to_string()))?;
    std::fs::create_dir_all(xml_path.parent().unwrap_or(&output_dir))?;
    std::fs::write(&xml_path, xml)?;
    let dvd_dir = output_dir.join("dvd");
    DvdAuthor::default()
        .author(&xml_path, &dvd_dir)
        .await
        .map_err(|error| RenderError::Author(error.to_string()))?;
    dvdforge_dvd::apply_region_mask(&dvd_dir, project.disc.region_mask)
        .map_err(|error| RenderError::Author(error.to_string()))?;
    std::fs::create_dir_all(dvd_dir.join("AUDIO_TS"))?;
    progress(&events, JobStage::Author, 1000).await;

    ensure_not_cancelled(&cancel)?;
    stage(&events, JobStage::Iso, "Construyendo ISO").await;
    let iso_path = output_dir.join("disc.iso");
    let iso = Xorriso::default()
        .build_dvd_iso(&dvd_dir, &iso_path, &project.disc.volume_label)
        .await
        .map_err(|error| RenderError::Iso(error.to_string()))?;
    let _ = events
        .send(JobEventKind::Artifact {
            kind: ArtifactKind::Iso,
            path: iso.iso_path.display().to_string(),
        })
        .await;
    progress(&events, JobStage::Iso, 1000).await;

    let warnings = plan.warnings.clone();
    let _ = events
        .send(JobEventKind::Completed {
            summary: format!(
                "ISO generado: {} ({} bytes)",
                iso.iso_path.display(),
                iso.size_bytes
            ),
        })
        .await;
    Ok(RenderOutcome {
        job_id,
        iso_path: iso.iso_path.display().to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        warnings,
    })
}

async fn collect_probes(
    project: &Project,
    project_dir: &Path,
    refresh: bool,
) -> HashMap<AssetId, MediaProbe> {
    let mut probes = HashMap::new();
    for asset in &project.assets {
        probes.insert(asset.id, asset.probe.clone());
        if !refresh || asset.kind != MediaKind::Video {
            continue;
        }
        let Some(path) = asset_path(project_dir, asset) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        if let Ok(probe) = Ffprobe::default().probe(&path).await {
            probes.insert(asset.id, probe);
        }
    }
    probes
}

async fn sha256_file(path: &Path) -> Result<String, RenderError> {
    let path = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || -> Result<String, std::io::Error> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(|error| RenderError::Encode(format!("hash cancelado: {error}")))??;
    Ok(hash)
}

fn audio_params_hash(episode: &Episode) -> String {
    let encoded = serde_json::to_vec(&episode.audio).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(encoded);
    hex::encode(hasher.finalize())
}

fn cached_artifact_path(
    store: &Store,
    key: &ArtifactKey,
    extension: &str,
    source: &Path,
    duration_ms: u64,
) -> Result<PathBuf, RenderError> {
    let target = store.artifact_blob_path(key, extension);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, &target)?;
    store
        .put_artifact(key, &target, duration_ms)
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    Ok(target)
}

fn prepare_output(output: &Path) -> Result<(), RenderError> {
    for directory in ["dvd", "intro", "episodes", "menus", "xml"] {
        let path = output.join(directory);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir_all(&path)?;
    }
    let iso = output.join("disc.iso");
    if iso.exists() {
        std::fs::remove_file(iso)?;
    }
    Ok(())
}

fn prepare_avchd_output(output: &Path) -> Result<(), RenderError> {
    for directory in ["dvd", "intro", "episodes", "menus", "xml", "meta", "avchd"] {
        let path = output.join(directory);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
    }
    std::fs::create_dir_all(output.join("episodes"))?;
    std::fs::create_dir_all(output.join("meta"))?;
    std::fs::create_dir_all(output.join("menus"))?;
    let iso = output.join("disc.iso");
    if iso.exists() {
        std::fs::remove_file(iso)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct HdMenuLayout {
    width: u16,
    height: u16,
    pts_90k: u64,
}

fn compile_hd_menu(
    project: &Project,
    project_dir: &Path,
    menu: &dvdforge_domain::Menu,
    composited_backgrounds: Option<&Path>,
    layout: HdMenuLayout,
    context: &HdMenuContext<'_>,
) -> Result<CompiledHdMenu, RenderError> {
    let width = layout.width;
    let height = layout.height;
    let pts_90k = layout.pts_90k;
    let has_motion_previews = menu
        .buttons
        .iter()
        .any(|button| button.motion_preview.is_some() && button.thumbnail.is_some());
    let video_background = matches!(menu.background, MenuBackground::VideoLoop { .. });
    // Video backgrounds and motion buttons keep their artwork in a transparent
    // labels overlay so the underlying menu video remains visible. Static
    // menus bake the complete artwork into the IG background.
    let overlay_only = video_background || has_motion_previews;
    let static_asset = match &menu.background {
        MenuBackground::Static { asset } => project
            .asset(*asset)
            .and_then(|asset| asset_path(project_dir, asset))
            .filter(|path| path.is_file()),
        MenuBackground::VideoLoop { .. } => None,
    };
    let background_path = if overlay_only {
        composited_backgrounds
            .map(|directory| directory.join(format!("labels-{}.png", menu.id)))
            .filter(|path| path.is_file())
    } else {
        composited_backgrounds
            .map(|directory| directory.join(format!("{}.png", menu.id)))
            .filter(|path| path.is_file())
            .or(static_asset)
    };
    let mut background_image = match &background_path {
        Some(path) => {
            let image = image::open(path)?.to_rgba8();
            image::imageops::resize(
                &image,
                width as u32,
                height as u32,
                image::imageops::FilterType::Triangle,
            )
        }
        None if overlay_only => {
            image::RgbaImage::from_pixel(width as u32, height as u32, image::Rgba([0, 0, 0, 0]))
        }
        None => image::RgbaImage::from_pixel(
            width as u32,
            height as u32,
            image::Rgba([11, 18, 32, 255]),
        ),
    };
    let labels_path = if overlay_only {
        None
    } else {
        composited_backgrounds
            .map(|directory| directory.join(format!("labels-{}.png", menu.id)))
            .filter(|path| path.is_file())
    };
    if let Some(labels_path) = labels_path {
        let labels = image::open(labels_path)?.to_rgba8();
        let labels = image::imageops::resize(
            &labels,
            width as u32,
            height as u32,
            image::imageops::FilterType::Triangle,
        );
        image::imageops::overlay(&mut background_image, &labels, 0, 0);
    }
    let has_background = overlay_only || background_path.is_some();
    let background =
        Some(RgbaBitmap::new(width, height, background_image.into_raw()).map_err(menu_error)?);
    let navigation = menu.navigation(None);
    let mut buttons = Vec::with_capacity(menu.buttons.len());
    for (position, button) in menu.buttons.iter().enumerate() {
        let rect = normalize_rect(button.rect, menu.canvas, 8)
            .map_err(|error| RenderError::Menu(error.to_string()))?;
        let x0 = scale_coord(rect.x0, menu.canvas.width, width);
        let y0 = scale_coord(rect.y0, menu.canvas.height, height);
        let x1 = scale_coord(rect.x1, menu.canvas.width, width).max(x0 + 2);
        let y1 = scale_coord(rect.y1, menu.canvas.height, height).max(y0 + 2);
        let button_width = x1.saturating_sub(x0).max(2);
        let button_height = y1.saturating_sub(y0).max(2);
        let fill = parse_hex_color(&button.fill_color, 255);
        let normal = if has_background {
            RgbaBitmap::solid(button_width, button_height, [0, 0, 0, 0])
        } else {
            RgbaBitmap::solid(button_width, button_height, fill)
        }
        .map_err(menu_error)?;
        let selected = RgbaBitmap::solid(button_width, button_height, [255, 255, 255, 76])
            .map_err(menu_error)?;
        let activated = RgbaBitmap::solid(button_width, button_height, [34, 211, 238, 112])
            .map_err(menu_error)?;
        let hmv_id = (position + 1) as u16;
        let target = |direction: dvdforge_domain::NavDirection| {
            navigation.target(button.id, direction).and_then(|id| {
                menu.buttons
                    .iter()
                    .position(|candidate| candidate.id == id)
                    .map(|index| (index + 1) as u16)
            })
        };
        buttons.push(HdmvButton {
            id: hmv_id,
            x: x0,
            y: y0,
            up: target(dvdforge_domain::NavDirection::Up),
            down: target(dvdforge_domain::NavDirection::Down),
            left: target(dvdforge_domain::NavDirection::Left),
            right: target(dvdforge_domain::NavDirection::Right),
            commands: hdmv_commands(&button.action, context),
            state: HdmvButtonState {
                normal,
                selected,
                activated,
            },
        });
    }
    let fps = match &project.disc.video {
        VideoConfig::Avchd(config) => config.frame_rate.as_f64(),
        VideoConfig::Dvd(_) => project.disc.standard.frame_rate().as_f64(),
    };
    let menu = HdmvMenu {
        width,
        height,
        frame_rate_code: frame_rate_code(fps),
        background,
        buttons,
        default_selected: Some(1),
    };
    let pes_packets = compile_ig_pes_packets(&menu, pts_90k).map_err(menu_error)?;
    let pes = compile_ig_pes(&menu, pts_90k).map_err(menu_error)?;
    let m2ts = compile_ig_m2ts(&menu, pts_90k).map_err(menu_error)?;
    Ok(CompiledHdMenu {
        pes,
        m2ts,
        pes_packets,
    })
}

fn hdmv_commands(action: &ButtonAction, context: &HdMenuContext<'_>) -> Vec<HdmvCommand> {
    match action {
        ButtonAction::PlayEpisode { episode } => context
            .episode_marks
            .get(episode)
            .copied()
            .map(play_episode_at)
            .unwrap_or_else(|| vec![set_gpr(0, 0), jump_title(1)]),
        ButtonAction::PlayAll => vec![set_gpr(0, 0), jump_title(1)],
        ButtonAction::GotoMenu { menu } => context
            .menu_objects
            .get(menu)
            .copied()
            .map(|object| vec![jump_object(object as u32)])
            .unwrap_or_else(|| vec![jump_title(0)]),
        ButtonAction::PlayFirstPlay => vec![set_gpr(0, 0), jump_title(1)],
        ButtonAction::JumpTitleChapter { episode, chapter } => context
            .chapter_marks
            .get(&(*episode, *chapter as u16))
            .copied()
            .map(play_episode_at)
            .unwrap_or_else(|| {
                context
                    .episode_marks
                    .get(episode)
                    .copied()
                    .map(play_episode_at)
                    .unwrap_or_else(|| vec![set_gpr(0, 0), jump_title(1)])
            }),
        ButtonAction::SetAudioTrack { track } => vec![set_audio_stream(*track)],
        ButtonAction::Resume | ButtonAction::Noop => {
            vec![HdmvCommand::new(dvdforge_hdmv::OP_NOP, 0, 0)]
        }
    }
}

fn play_episode_at(mark: u16) -> Vec<HdmvCommand> {
    // tsMuxeR's movie object copies GPR0 to GPR10 and then runs
    // PlayPL_PM on playlist 0. Setting GPR0 here starts the movie at the
    // requested chapter mark without changing the playlist inside IG mode.
    vec![set_gpr(0, mark as u32), jump_title(1)]
}

fn scale_coord(value: u16, source: u16, target: u16) -> u16 {
    if source == 0 {
        0
    } else {
        ((value as u32 * target as u32) / source as u32).min(u16::MAX as u32) as u16
    }
}

fn parse_hex_color(value: &str, alpha: u8) -> [u8; 4] {
    let trimmed = value.trim_start_matches('#');
    if trimmed.len() != 6 {
        return [15, 23, 42, alpha];
    }
    let parse =
        |range: std::ops::Range<usize>| u8::from_str_radix(&trimmed[range], 16).unwrap_or(15);
    [parse(0..2), parse(2..4), parse(4..6), alpha]
}

fn menu_error(error: dvdforge_hdmv::HdmvError) -> RenderError {
    RenderError::Menu(error.to_string())
}

async fn render_avchd_project(
    project: Project,
    project_dir: PathBuf,
    output_dir: PathBuf,
    job_id: JobId,
    options: RenderOptions,
    events: EventSink,
) -> Result<RenderOutcome, RenderError> {
    let started = Instant::now();
    let composited_backgrounds = options.composited_backgrounds.clone();
    let cancel = options.cancel.clone();
    let config = match &project.disc.video {
        VideoConfig::Avchd(config) => config.clone(),
        VideoConfig::Dvd(_) => {
            return Err(RenderError::Encode(
                "el proyecto AVCHD no tiene configuración AVCHD".to_string(),
            ));
        }
    };

    stage(&events, JobStage::Probe, "Sondeando medios").await;
    let probes = collect_probes(&project, &project_dir, options.probe_assets).await;
    progress(&events, JobStage::Probe, 1000).await;

    stage(&events, JobStage::Plan, "Calculando bitrate AVCHD").await;
    let plan = calculate_bitrate_plan(&project, &probes)
        .map_err(|error| RenderError::Plan(error.to_string()))?;
    for warning in &plan.warnings {
        let _ = events
            .send(JobEventKind::Warning {
                code: warning.code.clone(),
                message: warning.message.clone(),
            })
            .await;
    }
    if !plan.feasible {
        return Err(RenderError::Plan(
            "el plan AVCHD no es viable; revisa las advertencias".to_string(),
        ));
    }
    progress(&events, JobStage::Plan, 1000).await;
    ensure_not_cancelled(&cancel)?;

    prepare_avchd_output(&output_dir)?;
    let encoder = AvchdEncoder::default();
    let episodes_dir = output_dir.join("episodes");
    let total = project.episodes.len().max(1);
    let mut stream_inputs = Vec::with_capacity(project.episodes.len());
    let mut order: Vec<&Episode> = project.episodes.iter().collect();
    order.sort_by_key(|episode| episode.number);
    let mut chapters = Vec::new();
    let mut episode_start_ms: HashMap<EpisodeId, u64> = HashMap::new();
    let mut episode_fps: HashMap<EpisodeId, FrameRate> = HashMap::new();
    let mut timeline_ms = 0u64;
    let mut language = "und".to_string();
    let cache = Store::open_default().ok();

    stage(
        &events,
        JobStage::EncodeEpisodes,
        "Codificando episodios AVCHD",
    )
    .await;
    for (index, episode) in order.iter().enumerate() {
        ensure_not_cancelled(&cancel)?;
        let asset = project
            .asset(episode.video)
            .ok_or_else(|| RenderError::Encode("episodio AVCHD sin asset".to_string()))?;
        let probe = probes.get(&asset.id).unwrap_or(&asset.probe);
        let input = asset_path(&project_dir, asset)
            .ok_or_else(|| RenderError::Encode("ruta de episodio AVCHD inválida".to_string()))?;
        let source_fps = probe
            .video
            .as_ref()
            .map(|video| video.frame_rate)
            .unwrap_or_else(|| config.frame_rate);
        let duration_ms = plan
            .per_episode
            .iter()
            .find(|entry| entry.episode == episode.id)
            .map(|entry| entry.duration_ms)
            .or(probe.duration_ms)
            .unwrap_or(1);
        let max_video_cap = if project.disc.capacity.is_blu_ray() {
            dvdforge_avchd::BLURAY_MAX_VIDEO_KBPS
        } else {
            dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
        };
        let planned_kbps = plan
            .per_episode
            .iter()
            .find(|entry| entry.episode == episode.id)
            .map(|entry| entry.video_kbps)
            .unwrap_or(plan.video_kbps)
            .clamp(1, max_video_cap);
        let audio = episode
            .audio
            .first()
            .map(|audio| AvchdAudioConfig {
                source_stream: audio.source_stream,
                bitrate_kbps: audio.ac3_kbps,
                channels: project.disc.audio.channels.clamp(1, 6),
                sample_rate: 48_000,
            })
            .or_else(|| {
                probe.audio.first().map(|audio| AvchdAudioConfig {
                    source_stream: audio.index,
                    bitrate_kbps: project.disc.audio.ac3_kbps,
                    channels: project.disc.audio.channels.clamp(1, 6),
                    sample_rate: 48_000,
                })
            });
        if language == "und" {
            if let Some(audio) = episode.audio.first() {
                language = audio.language.clone();
            } else if let Some(audio) = probe.audio.first() {
                if let Some(value) = &audio.language {
                    language = value.clone();
                }
            }
        }
        let cache_key = if cache.is_some() {
            Some(ArtifactKey {
                source_sha256: sha256_file(&input).await?,
                trim_in_ms: episode.trim_start_ms.unwrap_or(0),
                trim_out_ms: episode.trim_end_ms.unwrap_or(duration_ms),
                target_profile: format!(
                    "hdmv:{:?}:{:.3}:{:?}:{}",
                    config.resolution,
                    config.frame_rate.as_f64(),
                    episode.scale_mode,
                    project.disc.capacity.label()
                ),
                target_bitrate: planned_kbps,
                audio_params_hash: audio_params_hash(episode),
            })
        } else {
            None
        };
        let cached = cache
            .as_ref()
            .zip(cache_key.as_ref())
            .and_then(|(store, key)| store.lookup_artifact(key).ok().flatten())
            .filter(|artifact| Path::new(&artifact.artifact_path).is_file());
        if let Some(artifact) = &cached {
            log(
                &events,
                format!(
                    "Episodio AVCHD {}/{}: reutilizando caché SHA-256 {}",
                    index + 1,
                    total,
                    artifact.artifact_path
                ),
            )
            .await;
        } else {
            log(
                &events,
                format!(
                    "Codificando episodio AVCHD {}/{}: {} ({} kbps)",
                    index + 1,
                    total,
                    episode.title,
                    planned_kbps
                ),
            )
            .await;
        }
        let request = AvchdEncodeRequest {
            input,
            output_dir: episodes_dir.clone(),
            stem: format!("episode-{:02}", episode.number),
            config: config.clone(),
            disc_capacity: project.disc.capacity,
            aspect: project.disc.aspect,
            video_kbps: planned_kbps,
            maxrate_kbps: if project.disc.capacity.is_blu_ray() {
                config
                    .vbv_maxrate_kbps
                    .min(dvdforge_avchd::BLURAY_MAX_VIDEO_KBPS)
                    .max(planned_kbps)
            } else {
                dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
            },
            vbv_bufsize_kbps: if project.disc.capacity.is_blu_ray() {
                config.vbv_bufsize_kbps.clamp(
                    dvdforge_avchd::BLURAY_VBV_MIN_KBITS,
                    dvdforge_avchd::BLURAY_VBV_MAX_KBITS,
                )
            } else {
                dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
            },
            audio,
            trim: episode.trim,
            trim_start_ms: episode.trim_start_ms,
            trim_end_ms: episode.trim_end_ms,
            scale_mode: episode.scale_mode,
            fps: source_fps,
            duration_ms,
        };
        let stream_mkv = if let Some(artifact) = cached {
            PathBuf::from(artifact.artifact_path)
        } else {
            let encoded = encode_avchd_episode_with_progress(
                &encoder,
                &request,
                duration_ms,
                &events,
                index,
                total,
            )
            .await?;
            if let (Some(store), Some(key)) = (&cache, &cache_key) {
                cached_artifact_path(store, key, "mkv", &encoded.stream_mkv, duration_ms)
                    .unwrap_or(encoded.stream_mkv)
            } else {
                encoded.stream_mkv
            }
        };
        stream_inputs.push(stream_mkv);
        chapters.push(timeline_ms);
        episode_start_ms.insert(episode.id, timeline_ms);
        episode_fps.insert(episode.id, source_fps);
        for chapter in &episode.chapters {
            let offset_ms = if source_fps.is_valid() {
                (chapter.at_frame as f64 * 1000.0 / source_fps.as_f64()).round() as u64
            } else {
                0
            };
            if offset_ms < duration_ms {
                chapters.push(timeline_ms + offset_ms);
            }
        }
        timeline_ms = timeline_ms.saturating_add(duration_ms);
        progress(
            &events,
            JobStage::EncodeEpisodes,
            (((index + 1) as f64 / total as f64) * 1000.0) as u16,
        )
        .await;
    }

    ensure_not_cancelled(&cancel)?;
    stage(&events, JobStage::Author, "Multiplexando AVCHD con tsMuxeR").await;
    let combined_mkv = episodes_dir.join("combined.mkv");
    concat_mkv(&encoder.ffmpeg, &stream_inputs, &combined_mkv)
        .await
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    chapters.sort_unstable();
    chapters.dedup();

    let mut episode_marks = HashMap::new();
    let mut chapter_marks = HashMap::new();
    for (episode_id, start_ms) in &episode_start_ms {
        if let Some(index) = chapters.iter().position(|chapter| chapter == start_ms) {
            episode_marks.insert(*episode_id, index as u16);
        }
    }
    for episode in &order {
        let start_ms = episode_start_ms
            .get(&episode.id)
            .copied()
            .unwrap_or_default();
        let fps = episode_fps
            .get(&episode.id)
            .copied()
            .unwrap_or(config.frame_rate);
        for (chapter_index, chapter) in episode.chapters.iter().enumerate() {
            let offset_ms = if fps.is_valid() {
                (chapter.at_frame as f64 * 1000.0 / fps.as_f64()).round() as u64
            } else {
                0
            };
            let mark_ms = start_ms.saturating_add(offset_ms);
            if let Some(index) = chapters.iter().position(|value| *value == mark_ms) {
                chapter_marks.insert((episode.id, chapter_index as u16), index as u16);
            }
        }
    }

    let mut warnings = plan.warnings.clone();
    let (menu_width, menu_height) = config.resolution.dimensions();
    let root_menu = project
        .menu(project.root_menu)
        .filter(|menu| !menu.buttons.is_empty());
    let has_menus = root_menu.is_some();

    let maxrate_kbps = if project.disc.capacity.is_blu_ray() {
        config
            .vbv_maxrate_kbps
            .clamp(plan.video_kbps, dvdforge_avchd::BLURAY_MAX_VIDEO_KBPS)
    } else {
        dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
    };
    let vbv_bufsize_kbps = if project.disc.capacity.is_blu_ray() {
        config.vbv_bufsize_kbps.clamp(
            dvdforge_avchd::BLURAY_VBV_MIN_KBITS,
            dvdforge_avchd::BLURAY_VBV_MAX_KBITS,
        )
    } else {
        dvdforge_avchd::AVCHD_MAX_VIDEO_KBPS
    };
    let meta_path = output_dir.join("meta").join("avchd.meta");
    write_meta(
        &meta_path,
        &AvchdMetaRequest {
            video: "combined.mkv",
            audio: Some("combined.mkv"),
            video_track: 1,
            audio_track: 2,
            fps: config.frame_rate,
            chapters: &chapters,
            language: &language,
            capacity: project.disc.capacity,
            maxrate_kbps,
            vbv_bufsize_kbps,
            interactive_graphics: None,
        },
    )
    .map_err(|error| RenderError::Author(error.to_string()))?;
    let iso_path = output_dir.join("disc.iso");
    let muxer = TsMuxer::default();
    let (final_iso_path, final_iso_size) = if has_menus {
        stage(
            &events,
            JobStage::Author,
            "Multiplexando BDMV con menús HDMV IG",
        )
        .await;
        let bdmv_dir = output_dir.join("avchd");
        muxer
            .mux_folder(&meta_path, &bdmv_dir, &episodes_dir)
            .await
            .map_err(|error| RenderError::Author(error.to_string()))?;
        let bdmv_root = bdmv_dir.join("BDMV");
        let movie_object = read_bdmv_file(&bdmv_root.join("MovieObject.bdmv"))?;
        let base_object = movie_object_count(&movie_object)
            .map_err(|error| RenderError::Menu(error.to_string()))?;
        let root = root_menu.ok_or_else(|| {
            RenderError::Menu("no hay un menú raíz con botones para autorar".to_string())
        })?;
        let mut menu_list: Vec<&dvdforge_domain::Menu> = vec![root];
        for menu in &project.menus {
            if menu.id != root.id && !menu.buttons.is_empty() {
                menu_list.push(menu);
            }
        }
        let menu_objects: HashMap<MenuId, u16> = menu_list
            .iter()
            .enumerate()
            .map(|(index, menu)| (menu.id, base_object.saturating_add(index as u16)))
            .collect();
        let width = menu_width.min(u16::MAX as u32) as u16;
        let height = menu_height.min(u16::MAX as u32) as u16;
        let context = HdMenuContext {
            episode_marks: &episode_marks,
            chapter_marks: &chapter_marks,
            menu_objects: &menu_objects,
        };
        let mut compiled_menus: Vec<(MenuId, String, CompiledHdMenu)> = Vec::new();
        let mut menu_ready = true;
        for (index, menu) in menu_list.iter().enumerate() {
            ensure_not_cancelled(&cancel)?;
            match compile_hd_menu(
                &project,
                &project_dir,
                menu,
                composited_backgrounds.as_deref(),
                HdMenuLayout {
                    width,
                    height,
                    pts_90k: HDMV_MENU_PTS_90K,
                },
                &context,
            ) {
                Ok(compiled) => {
                    let stem = format!("{index:02}-{}", menu.id);
                    let igs = output_dir.join("menus").join(format!("{stem}.igs"));
                    let m2ts = output_dir.join("menus").join(format!("{stem}.m2ts"));
                    std::fs::write(&igs, &compiled.pes)?;
                    std::fs::write(&m2ts, &compiled.m2ts)?;
                    if index == 0 {
                        std::fs::write(output_dir.join("menus").join("root.igs"), &compiled.pes)?;
                        std::fs::write(output_dir.join("menus").join("root.m2ts"), &compiled.m2ts)?;
                    }
                    log(
                        &events,
                        format!(
                            "Menú HDMV IG compilado: {} ({} bytes, {} PES, objeto {})",
                            menu.name,
                            compiled.pes.len(),
                            compiled.pes_packets.len(),
                            base_object.saturating_add(index as u16)
                        ),
                    )
                    .await;
                    compiled_menus.push((menu.id, menu.name.clone(), compiled));
                }
                Err(error) => {
                    let message = format!(
                        "No se pudo compilar el menú HDMV IG «{}»: {error}",
                        menu.name
                    );
                    let _ = events
                        .send(JobEventKind::Warning {
                            code: "W_HDMV_IG_COMPILE".to_string(),
                            message: message.clone(),
                        })
                        .await;
                    warnings.push(Diagnostic::warning("W_HDMV_IG_COMPILE", message));
                    menu_ready = false;
                    break;
                }
            }
        }

        if menu_ready {
            let mut attached = true;
            for (index, (menu_id, menu_name, compiled)) in compiled_menus.iter().enumerate() {
                let menu = project
                    .menu(*menu_id)
                    .ok_or_else(|| RenderError::Menu("menú inexistente".to_string()))?;
                let menu_spec = build_menu_clip_spec(
                    &project,
                    &project_dir,
                    menu,
                    composited_backgrounds.as_deref(),
                    MenuClipFormat {
                        width,
                        height,
                        fps: config.frame_rate,
                        interlaced: config.resolution.interlaced(),
                        capacity: project.disc.capacity,
                        maxrate_kbps,
                        vbv_bufsize_kbps,
                    },
                );
                match attach_hdmv_menu(
                    &bdmv_dir,
                    &output_dir.join("menus").join(format!("clip-{index:02}")),
                    &compiled.pes_packets,
                    &menu_spec,
                    base_object.saturating_add(index as u16),
                )
                .await
                {
                    Ok(report) => {
                        log(
                            &events,
                            format!(
                                "Menú HDMV IG «{}» adjuntado: playlist {} (clip {}, objeto {}, {} paquetes IG)",
                                menu_name,
                                report.playlist,
                                report.clip,
                                report.object,
                                report.ig_packets
                            ),
                        )
                        .await;
                    }
                    Err(error) => {
                        let message = format!(
                            "No se pudo adjuntar el menú «{menu_name}» al BDMV; se genera el disco sin menú interactivo: {error}"
                        );
                        let _ = events
                            .send(JobEventKind::Warning {
                                code: "W_HDMV_IG_ATTACH".to_string(),
                                message: message.clone(),
                            })
                            .await;
                        warnings.push(Diagnostic::warning("W_HDMV_IG_ATTACH", message));
                        attached = false;
                        break;
                    }
                }
            }
            if attached {
                let root_object = *menu_objects.get(&project.root_menu).unwrap_or(&base_object);
                let index_path = bdmv_root.join("index.bdmv");
                let index = read_bdmv_file(&index_path)?;
                let index = set_index_menu_object(&index, root_object)
                    .map_err(|error| RenderError::Menu(error.to_string()))?;
                write_bdmv_file(&index_path, &index)?;
                let index_backup = bdmv_root.join("BACKUP/index.bdmv");
                if index_backup.is_file() {
                    write_bdmv_file(&index_backup, &index)?;
                }
                log(
                    &events,
                    format!(
                        "First Play/Top Menu apuntan al objeto raíz {root_object} ({} menús)",
                        compiled_menus.len()
                    ),
                )
                .await;
            }
        }
        ensure_not_cancelled(&cancel)?;
        let iso = match BdmvIsoBuilder::default()
            .build_with_cancel(
                &bdmv_dir,
                &iso_path,
                &project.disc.volume_label,
                cancel.clone(),
            )
            .await
        {
            Ok(report) => report,
            Err(error) => {
                if cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed))
                {
                    return Err(RenderError::Cancelled);
                }
                return Err(RenderError::Iso(error.to_string()));
            }
        };
        (iso.iso_path, iso.size_bytes)
    } else {
        // Even menu-less AVCHD/Blu-ray discs need the same UDF 2.50 metadata
        // partition as the interactive variant. Mux to a BDMV folder first and
        // wrap it with our UDF writer instead of letting tsMuxeR create the ISO.
        let bdmv_dir = output_dir.join("avchd");
        muxer
            .mux_folder(&meta_path, &bdmv_dir, &episodes_dir)
            .await
            .map_err(|error| RenderError::Author(error.to_string()))?;
        ensure_not_cancelled(&cancel)?;
        let iso = match BdmvIsoBuilder::default()
            .build_with_cancel(
                &bdmv_dir,
                &iso_path,
                &project.disc.volume_label,
                cancel.clone(),
            )
            .await
        {
            Ok(report) => report,
            Err(error) => {
                if cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::Relaxed))
                {
                    return Err(RenderError::Cancelled);
                }
                return Err(RenderError::Iso(error.to_string()));
            }
        };
        (iso.iso_path, iso.size_bytes)
    };
    progress(&events, JobStage::Author, 1000).await;

    stage(&events, JobStage::Iso, "Validando ISO AVCHD").await;
    let _ = events
        .send(JobEventKind::Artifact {
            kind: ArtifactKind::Iso,
            path: final_iso_path.display().to_string(),
        })
        .await;
    progress(&events, JobStage::Iso, 1000).await;

    let _ = events
        .send(JobEventKind::Completed {
            summary: format!(
                "ISO AVCHD/Blu-ray generado: {} ({} bytes, UDF {}, {} capítulos)",
                final_iso_path.display(),
                final_iso_size,
                dvdforge_avchd::AVCHD_UDF_REVISION,
                chapters.len()
            ),
        })
        .await;
    Ok(RenderOutcome {
        job_id,
        iso_path: final_iso_path.display().to_string(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        warnings,
    })
}

async fn encode_avchd_episode_with_progress(
    encoder: &AvchdEncoder,
    request: &AvchdEncodeRequest,
    duration_ms: u64,
    events: &EventSink,
    index: usize,
    total: usize,
) -> Result<AvchdOutputs, RenderError> {
    let (sender, mut receiver) = mpsc::channel::<dvdforge_avchd::EncodeProgress>(64);
    let event_sink = events.clone();
    let duration = duration_ms.max(1) as f64;
    let forward = tokio::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            let fraction = (progress.out_time_us as f64 / 1000.0 / duration).min(1.0);
            let episode_fraction = fraction.min(1.0);
            let permille = (((index as f64 + episode_fraction) / total.max(1) as f64) * 1000.0)
                .min(1000.0) as u16;
            let _ = event_sink
                .send(JobEventKind::Progress {
                    stage: JobStage::EncodeEpisodes,
                    permille,
                    fps: progress.fps.map(f64::from),
                    speed: progress.speed_x.map(f64::from),
                    eta_secs: eta_for(progress.out_time_us, duration_ms, progress.speed_x),
                })
                .await;
        }
    });
    let result = encoder
        .encode_with_progress(request, Some(sender))
        .await
        .map_err(|error| RenderError::Encode(error.to_string()));
    let _ = forward.await;
    result
}

async fn encode_intro(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    project_dir: &Path,
    output: &Path,
    encoder: &DvdEncoder,
) -> Result<Option<PathBuf>, RenderError> {
    let Some(first_play) = &project.first_play else {
        return Ok(None);
    };
    let asset = project
        .asset(first_play.asset)
        .ok_or_else(|| RenderError::Encode("la intro no existe".to_string()))?;
    let probe = probes.get(&asset.id).unwrap_or(&asset.probe);
    let input = asset_path(project_dir, asset)
        .ok_or_else(|| RenderError::Encode("ruta de intro inválida".to_string()))?;
    let fps = probe
        .video
        .as_ref()
        .map(|video| video.frame_rate)
        .unwrap_or_else(|| project.disc.standard.frame_rate());
    let request = EncodeRequest {
        input,
        output_dir: output.join("intro"),
        stem: "intro".to_string(),
        video: VideoEncodeConfig {
            video_kbps: 5_000,
            maxrate_kbps: 8_000,
            profile: DvdProfile::D1,
            standard: project.disc.standard,
            aspect: project.disc.aspect,
            gop_frames: gop_for(project.disc.standard),
            b_frames: 2,
            interlaced: is_interlaced(probe),
            top_field_first: is_top_field_first(probe),
            scale_mode: VideoScaleMode::Letterbox,
        },
        audio_tracks: first_play
            .audio
            .as_ref()
            .map(|audio| vec![audio_encode_config(audio)])
            .unwrap_or_default(),
        trim: first_play.trim,
        trim_start_ms: None,
        trim_end_ms: None,
        fps,
    };
    let encoded = encoder
        .encode(&request)
        .await
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    Ok(Some(encoded.muxed_mpg))
}

fn audio_encode_config(audio: &dvdforge_domain::EpisodeAudio) -> AudioEncodeConfig {
    AudioEncodeConfig {
        source_stream: audio.source_stream,
        bitrate_kbps: audio.ac3_kbps,
        channels: audio.format.channels(),
        sample_rate: 48_000,
        delay_ms: audio.delay_ms,
    }
}

struct SubtitleMuxContext<'a> {
    project: &'a Project,
    episode: &'a Episode,
    project_dir: &'a Path,
    output_dir: &'a Path,
    input_mpeg: &'a Path,
    stem: &'a str,
    ffmpeg: &'a Path,
}

async fn mux_episode_subtitles(
    context: SubtitleMuxContext<'_>,
    events: &EventSink,
) -> Result<(PathBuf, Vec<AuthoredSubtitleTrack>), RenderError> {
    let mut current = context.input_mpeg.to_path_buf();
    let mut authored = Vec::new();
    let mut stream_id = 0u8;
    for (index, track) in context.episode.subtitles.iter().enumerate() {
        let text = if let Some(source_path) = &track.source_path {
            let source = resolve_project_path(context.project_dir, source_path);
            std::fs::read_to_string(&source).map_err(|error| {
                RenderError::Menu(format!(
                    "No se pudo leer el subtítulo {}: {error}",
                    source.display()
                ))
            })?
        } else {
            let internal_index = context.episode.subtitles[..index]
                .iter()
                .filter(|candidate| candidate.source_path.is_none())
                .count();
            let asset = context
                .project
                .asset(context.episode.video)
                .ok_or_else(|| RenderError::Menu("episodio sin asset".to_string()))?;
            let video = asset_path(context.project_dir, asset)
                .ok_or_else(|| RenderError::Menu("ruta de vídeo inválida".to_string()))?;
            let extracted =
                context
                    .output_dir
                    .join(format!("{}.s{}.source.srt", context.stem, index + 1));
            let mut command = tokio::process::Command::new(context.ffmpeg);
            command
                .arg("-hide_banner")
                .arg("-y")
                .arg("-protocol_whitelist")
                .arg(MEDIA_PROTOCOL_WHITELIST)
                .arg("-i")
                .arg(&video)
                .arg("-map")
                .arg(format!("0:s:{internal_index}"))
                .arg("-c:s")
                .arg("srt")
                .arg(&extracted)
                .kill_on_drop(true);
            let result = timeout(AUX_FFMPEG_TIMEOUT, command.output())
                .await
                .map_err(|_| {
                    RenderError::Encode("FFmpeg agotó el tiempo al extraer subtítulos".to_string())
                })?
                .map_err(|error| RenderError::Encode(error.to_string()))?;
            if !result.status.success() {
                return Err(RenderError::Menu(format!(
                    "No se pudo extraer el subtítulo interno {}: {}",
                    internal_index,
                    String::from_utf8_lossy(&result.stderr).trim()
                )));
            }
            std::fs::read_to_string(&extracted)?
        };
        let trim_start = context.episode.trim_start_ms.unwrap_or(0) as i64;
        let shifted = shift_srt(&text, track.delay_ms as i64 - trim_start)
            .map_err(|error| RenderError::Menu(error.to_string()))?;
        let srt = context
            .output_dir
            .join(format!("{}.s{}.srt", context.stem, index + 1));
        let xml = context
            .output_dir
            .join(format!("{}.s{}.xml", context.stem, index + 1));
        let mpeg = context
            .output_dir
            .join(format!("{}.s{}.mpg", context.stem, index + 1));
        std::fs::write(&srt, shifted)?;
        write_textsub_xml(
            &xml,
            &TextSubtitleSpec {
                srt: &srt,
                fps: context.project.disc.standard.frame_rate(),
                standard: context.project.disc.standard,
                aspect: context.project.disc.aspect,
                forced: track.is_forced,
            },
        )
        .map_err(|error| RenderError::Menu(error.to_string()))?;
        log(
            events,
            format!(
                "Episodio {}: multiplexando subtítulo {} ({}, {} ms).",
                context.episode.number,
                index + 1,
                track.language,
                track.delay_ms
            ),
        )
        .await;
        Spumux::default()
            .mux_stream(&xml, &current, &mpeg, stream_id)
            .await
            .map_err(|error| RenderError::Menu(error.to_string()))?;
        current = mpeg;
        authored.push(AuthoredSubtitleTrack {
            language: track.language.clone(),
            stream_id,
        });
        stream_id = stream_id.saturating_add(1);
    }
    Ok((current, authored))
}

fn resolve_project_path(project_dir: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        project_dir.join(path)
    }
}

async fn encode_episodes(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    project_dir: &Path,
    output: &Path,
    encoder: &DvdEncoder,
    plan: &BitratePlan,
    events: &EventSink,
) -> Result<Vec<EpisodeFile>, RenderError> {
    let mut files = Vec::with_capacity(project.episodes.len());
    let cache = Store::open_default().ok();
    let total = project.episodes.len().max(1);
    for (index, episode) in project.episodes.iter().enumerate() {
        let asset = project
            .asset(episode.video)
            .ok_or_else(|| RenderError::Encode("episodio sin asset".to_string()))?;
        let probe = probes.get(&asset.id).unwrap_or(&asset.probe);
        let input = asset_path(project_dir, asset)
            .ok_or_else(|| RenderError::Encode("ruta de episodio inválida".to_string()))?;
        let fps = probe
            .video
            .as_ref()
            .map(|video| video.frame_rate)
            .unwrap_or_else(|| project.disc.standard.frame_rate());
        let audio_tracks: Vec<AudioEncodeConfig> =
            episode.audio.iter().map(audio_encode_config).collect();
        let audio_kbps: u32 = audio_tracks
            .iter()
            .map(|audio| audio.bitrate_kbps as u32)
            .sum();
        let cap = 10_080u32
            .saturating_sub(audio_kbps)
            .saturating_sub(100)
            .min(9_800);
        let planned = plan
            .per_episode
            .iter()
            .find(|entry| entry.episode == episode.id)
            .map(|entry| entry.video_kbps)
            .unwrap_or(plan.video_kbps);
        let video_kbps = planned.min(cap).max(1);
        let maxrate_kbps = (video_kbps + 3_000).min(cap).max(video_kbps);
        let duration = plan
            .per_episode
            .iter()
            .find(|entry| entry.episode == episode.id)
            .map(|entry| entry.duration_ms)
            .or(probe.duration_ms)
            .unwrap_or(1);
        let cache_key = if cache.is_some() {
            Some(ArtifactKey {
                source_sha256: sha256_file(&input).await?,
                trim_in_ms: episode.trim_start_ms.unwrap_or(0),
                trim_out_ms: episode.trim_end_ms.unwrap_or(duration),
                target_profile: format!(
                    "dvd:{:?}:{:?}:{:?}:{:?}",
                    plan.profile, project.disc.standard, project.disc.aspect, episode.scale_mode
                ),
                target_bitrate: video_kbps,
                audio_params_hash: audio_params_hash(episode),
            })
        } else {
            None
        };
        let request = EncodeRequest {
            input,
            output_dir: output.join("episodes"),
            stem: format!("episode-{:02}", episode.number),
            video: VideoEncodeConfig {
                video_kbps,
                maxrate_kbps,
                profile: plan.profile,
                standard: project.disc.standard,
                aspect: project.disc.aspect,
                gop_frames: gop_for(project.disc.standard),
                b_frames: 2,
                interlaced: is_interlaced(probe),
                top_field_first: is_top_field_first(probe),
                scale_mode: episode.scale_mode,
            },
            audio_tracks,
            trim: episode.trim,
            trim_start_ms: episode.trim_start_ms,
            trim_end_ms: episode.trim_end_ms,
            fps,
        };
        let cached = cache
            .as_ref()
            .zip(cache_key.as_ref())
            .and_then(|(store, key)| store.lookup_artifact(key).ok().flatten())
            .filter(|artifact| Path::new(&artifact.artifact_path).is_file());
        let base_mpeg = if let Some(artifact) = cached {
            log(
                events,
                format!(
                    "Episodio {}/{}: reutilizando caché SHA-256 {}",
                    index + 1,
                    total,
                    artifact.artifact_path
                ),
            )
            .await;
            PathBuf::from(artifact.artifact_path)
        } else {
            log(
                events,
                format!(
                    "Codificando episodio {}/{}: {}",
                    index + 1,
                    total,
                    episode.title
                ),
            )
            .await;
            let encoded =
                encode_episode_with_progress(encoder, &request, duration, events, index, total)
                    .await?;
            if let (Some(store), Some(key)) = (&cache, &cache_key) {
                match cached_artifact_path(store, key, "mpg", &encoded.muxed_mpg, duration) {
                    Ok(path) => path,
                    Err(_) => encoded.muxed_mpg.clone(),
                }
            } else {
                encoded.muxed_mpg.clone()
            }
        };
        let episodes_dir = output.join("episodes");
        let subtitle_stem = format!("episode-{:02}", episode.number);
        let (path, subtitles) = mux_episode_subtitles(
            SubtitleMuxContext {
                project,
                episode,
                project_dir,
                output_dir: &episodes_dir,
                input_mpeg: &base_mpeg,
                stem: &subtitle_stem,
                ffmpeg: &encoder.ffmpeg,
            },
            events,
        )
        .await?;
        files.push(EpisodeFile {
            episode: episode.id,
            number: episode.number,
            path,
            chapters: episode.chapters.clone(),
            fps,
            audio: episode
                .audio
                .iter()
                .map(|track| AuthoredAudioTrack {
                    language: track.language.clone(),
                    format: "ac3".to_string(),
                    channels: track.format.channels(),
                })
                .collect(),
            subtitles,
        });
        progress(
            events,
            JobStage::EncodeEpisodes,
            (((index + 1) as f64 / total as f64) * 1000.0) as u16,
        )
        .await;
    }
    Ok(files)
}

fn build_motion_buttons(
    project: &Project,
    project_dir: &Path,
    menu: &dvdforge_domain::Menu,
    rects: &[(ButtonId, PixelRect)],
    menu_dir: &Path,
) -> Result<Vec<MotionButtonEncode>, RenderError> {
    let mut buttons = Vec::new();
    for button in &menu.buttons {
        let Some(motion) = &button.motion_preview else {
            continue;
        };
        let Some((_, rect)) = rects.iter().find(|(id, _)| *id == button.id) else {
            continue;
        };
        let action_asset = match &button.action {
            ButtonAction::PlayEpisode { episode }
            | ButtonAction::JumpTitleChapter { episode, .. } => project
                .episodes
                .iter()
                .find(|item| item.id == *episode)
                .map(|item| item.video),
            _ => None,
        };
        let thumbnail_asset = button.thumbnail.as_ref().map(|thumbnail| thumbnail.asset);
        let Some(asset_id) =
            [thumbnail_asset, action_asset]
                .into_iter()
                .flatten()
                .find(|asset_id| {
                    project
                        .asset(*asset_id)
                        .is_some_and(|asset| asset.kind == MediaKind::Video)
                })
        else {
            continue;
        };
        let asset = project
            .asset(asset_id)
            .ok_or_else(|| RenderError::Menu("asset de botón animado inexistente".to_string()))?;
        let Some(input) = asset_path(project_dir, asset) else {
            continue;
        };
        let mask_png = if button.shape_mask != ButtonShapeMask::Rectangle {
            let mask = menu_dir.join("masks").join(format!("{}.png", button.id));
            write_button_shape_mask(
                button.shape_mask,
                rect.width() as u32,
                rect.height() as u32,
                &mask,
            )
            .map_err(|error| RenderError::Menu(error.to_string()))?;
            Some(mask)
        } else {
            None
        };
        buttons.push(MotionButtonEncode {
            input,
            mask_png,
            source_start_ms: motion.source_start_ms,
            loop_duration_ms: motion.loop_duration_ms.max(1),
            x: rect.x0,
            y: rect.y0,
            width: rect.width(),
            height: rect.height(),
            shape_mask: button.shape_mask,
        });
    }
    Ok(buttons)
}

async fn render_menus(
    project: &Project,
    project_dir: &Path,
    output: &Path,
    encoder: &DvdEncoder,
    events: &EventSink,
    composited_backgrounds: Option<&Path>,
) -> Result<Vec<RenderedMenu>, RenderError> {
    let mut rendered = Vec::with_capacity(project.menus.len());
    let total = project.menus.len().max(1);
    for (index, menu) in project.menus.iter().enumerate() {
        let menu_dir = output.join("menus").join(menu.id.to_string());
        let composited = composited_backgrounds
            .map(|directory| directory.join(format!("{}.png", menu.id)))
            .filter(|path| path.is_file());
        let is_composited = composited.is_some();
        let background_source = if let Some(composited) = composited {
            log(
                events,
                format!(
                    "Usando composición del editor para el menú «{}».",
                    menu.name
                ),
            )
            .await;
            composited
        } else {
            let background_asset = match &menu.background {
                MenuBackground::Static { asset } => *asset,
                MenuBackground::VideoLoop { asset, .. } => *asset,
            };
            let background = project
                .asset(background_asset)
                .ok_or_else(|| RenderError::Menu("fondo de menú inexistente".to_string()))?;
            let background_source = asset_path(project_dir, background)
                .ok_or_else(|| RenderError::Menu("ruta de fondo inválida".to_string()))?;
            // Un proyecto recién creado aún no tiene un PNG de fondo en disco.
            // Se usa el mismo color base que el lienzo del editor para que el
            // render no dependa de elegir una imagen antes de probar.
            let video_background = matches!(menu.background, MenuBackground::VideoLoop { .. });
            if background_source.is_file() && !video_background {
                background_source
            } else {
                let fallback = menu_dir.join("background_fallback.png");
                write_placeholder_background(
                    &fallback,
                    menu.canvas.width as u32,
                    menu.canvas.height as u32,
                )?;
                log(
                    events,
                    format!(
                        "Fondo del menú «{}» no encontrado; se usa color base.",
                        menu.name
                    ),
                )
                .await;
                fallback
            }
        };
        log(
            events,
            format!(
                "Compilando menú {}/{}: {} (spumux)",
                index + 1,
                total,
                menu.name
            ),
        )
        .await;
        let rects = menu
            .normalized_button_rects()
            .map_err(|error| RenderError::Menu(error.to_string()))?;
        let compiled: Vec<CompiledButton> = rects
            .iter()
            .map(|(id, rect)| {
                let button = menu
                    .buttons
                    .iter()
                    .find(|button| button.id == *id)
                    .ok_or_else(|| {
                        RenderError::Menu(format!("botón normalizado inexistente: {id}"))
                    })?;
                Ok(CompiledButton {
                    id: *id,
                    name: button.name.clone(),
                    rect: *rect,
                    style_kind: button.style_kind,
                    shape_mask: button.shape_mask,
                })
            })
            .collect::<Result<Vec<_>, RenderError>>()?;
        let motion_buttons = build_motion_buttons(project, project_dir, menu, &rects, &menu_dir)?;
        let navigation = menu.navigation(None);
        if navigation.has_errors() {
            return Err(RenderError::Menu(
                "la navegación del menú contiene errores".to_string(),
            ));
        }
        let layers = generate_menu_layers(&MenuLayerRequest {
            canvas: menu.canvas,
            background_source: background_source.clone(),
            buttons: compiled.clone(),
            palette: menu.subpicture.palette,
            // La composición del editor ya incluye el ajuste, el oscurecido y el
            // desenfoque del fondo. Volver a aplicarlos duplicaría el efecto y
            // recortaría los textos y miniaturas ya rasterizados.
            scale_mode: if is_composited {
                MenuScaleMode::Stretch
            } else {
                menu.scale_mode
            },
            dim_percent: if is_composited { 0 } else { menu.dim_percent },
            blur_px: if is_composited { 0 } else { menu.blur_px },
            output_dir: menu_dir.join("layers"),
        })
        .map_err(|error| RenderError::Menu(error.to_string()))?;
        let request = MenuCompileRequest {
            menu_id: menu.id,
            canvas: menu.canvas,
            background_png: layers.background_png.clone(),
            highlight_png: layers.highlight_png.clone(),
            select_png: layers.select_png.clone(),
            buttons: compiled,
            force: menu.subpicture.force,
        };
        let xml_path = output.join("xml").join(format!("spumux-{}.xml", menu.id));
        write_spumux_xml(&request, &navigation, &xml_path)
            .map_err(|error| RenderError::Menu(error.to_string()))?;

        let audio_ac3 = if let Some(menu_audio) = &menu.audio {
            let asset = project
                .asset(menu_audio.asset)
                .ok_or_else(|| RenderError::Menu("audio de menú inexistente".to_string()))?;
            let input = asset_path(project_dir, asset)
                .ok_or_else(|| RenderError::Menu("ruta de audio inválida".to_string()))?;
            let audio_path = menu_dir.join("menu.ac3");
            encode_menu_audio(
                &input,
                &audio_path,
                menu.timing.duration_ms,
                menu_audio,
                menu.motion_config.as_ref(),
            )
            .await?;
            Some(audio_path)
        } else {
            None
        };

        let base_mpeg = menu_dir.join("menu_base.mpg");
        let video_background = matches!(menu.background, MenuBackground::VideoLoop { .. });
        let mut labels_overlay = composited_backgrounds
            .map(|directory| directory.join(format!("labels-{}.png", menu.id)))
            .filter(|path| path.is_file());
        // The CLI does not rasterize Konva text layers. A transparent overlay
        // still lets the motion encoder composite video backgrounds and
        // moving thumbnails; the subtitle masks provide the button artwork.
        if labels_overlay.is_none() && (menu.motion_config.is_some() || video_background) {
            let fallback = menu_dir.join("labels-transparent.png");
            write_transparent_overlay(
                &fallback,
                menu.canvas.width as u32,
                menu.canvas.height as u32,
            )?;
            labels_overlay = Some(fallback);
        }
        let animated = (menu.motion_config.is_some() || video_background)
            && (video_background || !motion_buttons.is_empty());
        if animated {
            let labels_overlay = labels_overlay.ok_or_else(|| {
                RenderError::Menu(
                    "no se pudo generar la capa de etiquetas del menú animado".to_string(),
                )
            })?;
            let (motion_background, background_is_video) = match &menu.background {
                MenuBackground::VideoLoop { asset, .. } => {
                    let background_asset = project.asset(*asset).ok_or_else(|| {
                        RenderError::Menu("fondo de vídeo inexistente".to_string())
                    })?;
                    let path = asset_path(project_dir, background_asset).ok_or_else(|| {
                        RenderError::Menu("ruta de fondo de vídeo inválida".to_string())
                    })?;
                    (path, true)
                }
                MenuBackground::Static { .. } => (background_source.clone(), false),
            };
            log(
                events,
                format!(
                    "Menú animado {}/{}: {} ({} botones de vídeo)",
                    index + 1,
                    total,
                    menu.name,
                    motion_buttons.len()
                ),
            )
            .await;
            encode_motion_menu_with_progress(
                encoder,
                &MotionMenuEncodeRequest {
                    background: motion_background,
                    background_is_video,
                    labels_overlay,
                    buttons: motion_buttons,
                    audio_ac3: audio_ac3.clone(),
                    output_mpg: base_mpeg.clone(),
                    duration_ms: menu.timing.duration_ms.max(1),
                    standard: project.disc.standard,
                    aspect: project.disc.aspect,
                    fps: project.disc.standard.frame_rate(),
                    video_kbps: 6_000,
                    maxrate_kbps: 8_000,
                    dim_percent: if background_is_video {
                        menu.dim_percent
                    } else {
                        0
                    },
                    blur_px: if background_is_video { menu.blur_px } else { 0 },
                    scale_mode: if background_is_video {
                        menu.scale_mode
                    } else {
                        MenuScaleMode::Stretch
                    },
                },
                menu.timing.duration_ms.max(1),
                events,
                index,
                total,
            )
            .await?;
        } else {
            if menu.motion_config.is_some() && labels_overlay.is_none() {
                log(
                    events,
                    format!(
                        "Menú «{}»: falta labels-<id>.png; se renderiza estático.",
                        menu.name
                    ),
                )
                .await;
            }
            encode_menu_with_progress(
                encoder,
                &MenuEncodeRequest {
                    background_png: layers.background_png,
                    audio_ac3,
                    output_mpg: base_mpeg.clone(),
                    duration_ms: menu.timing.duration_ms.max(1),
                    standard: project.disc.standard,
                    aspect: project.disc.aspect,
                    video_kbps: 5_000,
                    maxrate_kbps: 8_000,
                    fps: project.disc.standard.frame_rate(),
                },
                menu.timing.duration_ms.max(1),
                events,
                index,
                total,
            )
            .await?;
        }

        let sub_mpeg = menu_dir.join("menu_sub.mpg");
        Spumux::default()
            .mux(&xml_path, &base_mpeg, &sub_mpeg)
            .await
            .map_err(|error| RenderError::Menu(error.to_string()))?;

        rendered.push(RenderedMenu {
            menu_id: menu.id,
            placement: menu.placement,
            role: menu.role,
            mpeg: sub_mpeg,
            buttons: menu
                .buttons
                .iter()
                .enumerate()
                .map(|(index, button)| AuthoredButton {
                    name: format!("b{}", index + 1),
                    action: button.action.clone(),
                })
                .collect(),
        });
        progress(
            events,
            JobStage::EncodeMenus,
            (((index + 1) as f64 / total as f64) * 1000.0) as u16,
        )
        .await;
    }
    Ok(rendered)
}

fn eta_for(out_time_us: u64, duration_ms: u64, speed_x: Option<f32>) -> Option<u64> {
    let speed = speed_x.filter(|speed| *speed > 0.01)? as f64;
    let processed = out_time_us as f64 / 1_000_000.0;
    let remaining = (duration_ms as f64 / 1000.0 - processed).max(0.0);
    Some((remaining / speed).ceil() as u64)
}

async fn encode_episode_with_progress(
    encoder: &DvdEncoder,
    request: &EncodeRequest,
    duration_ms: u64,
    events: &EventSink,
    index: usize,
    total: usize,
) -> Result<EncodeOutputs, RenderError> {
    let (sender, mut receiver) = mpsc::channel::<EncodeProgress>(64);
    let event_sink = events.clone();
    let duration = duration_ms.max(1) as f64;
    let forward = tokio::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            let (base, span) = match progress.stage {
                "pass1" => (0.0, 0.4),
                "pass2" => (0.4, 0.4),
                "audio" => (0.8, 0.05),
                "mux" => (0.85, 0.15),
                _ => (0.0, 1.0),
            };
            let fraction = (progress.out_time_us as f64 / 1000.0 / duration).min(1.0);
            let episode_fraction = (base + fraction * span).min(1.0);
            let permille = (((index as f64 + episode_fraction) / total.max(1) as f64) * 1000.0)
                .min(1000.0) as u16;
            let _ = event_sink
                .send(JobEventKind::Progress {
                    stage: JobStage::EncodeEpisodes,
                    permille,
                    fps: progress.fps.map(f64::from),
                    speed: progress.speed_x.map(f64::from),
                    eta_secs: eta_for(progress.out_time_us, duration_ms, progress.speed_x),
                })
                .await;
        }
    });
    let result = encoder
        .encode_with_progress(request, Some(sender))
        .await
        .map_err(|error| RenderError::Encode(error.to_string()));
    let _ = forward.await;
    result
}

async fn encode_menu_with_progress(
    encoder: &DvdEncoder,
    request: &MenuEncodeRequest,
    duration_ms: u64,
    events: &EventSink,
    index: usize,
    total: usize,
) -> Result<PathBuf, RenderError> {
    let (sender, mut receiver) = mpsc::channel::<EncodeProgress>(64);
    let event_sink = events.clone();
    let duration = duration_ms.max(1) as f64;
    let forward = tokio::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            let fraction = (progress.out_time_us as f64 / 1000.0 / duration).min(1.0);
            let permille =
                (((index as f64 + fraction) / total.max(1) as f64) * 1000.0).min(1000.0) as u16;
            let _ = event_sink
                .send(JobEventKind::Progress {
                    stage: JobStage::EncodeMenus,
                    permille,
                    fps: None,
                    speed: progress.speed_x.map(f64::from),
                    eta_secs: None,
                })
                .await;
        }
    });
    let result = encoder
        .encode_menu_with_progress(request, Some(sender))
        .await
        .map_err(|error| RenderError::Encode(error.to_string()));
    let _ = forward.await;
    result
}

async fn encode_motion_menu_with_progress(
    encoder: &DvdEncoder,
    request: &MotionMenuEncodeRequest,
    duration_ms: u64,
    events: &EventSink,
    index: usize,
    total: usize,
) -> Result<PathBuf, RenderError> {
    let (sender, mut receiver) = mpsc::channel::<EncodeProgress>(64);
    let event_sink = events.clone();
    let duration = duration_ms.max(1) as f64;
    let forward = tokio::spawn(async move {
        while let Some(progress) = receiver.recv().await {
            let fraction = (progress.out_time_us as f64 / 1000.0 / duration).min(1.0);
            let permille =
                (((index as f64 + fraction) / total.max(1) as f64) * 1000.0).min(1000.0) as u16;
            let _ = event_sink
                .send(JobEventKind::Progress {
                    stage: JobStage::EncodeMenus,
                    permille,
                    fps: None,
                    speed: progress.speed_x.map(f64::from),
                    eta_secs: None,
                })
                .await;
        }
    });
    let result = encoder
        .encode_motion_menu_with_progress(request, Some(sender))
        .await
        .map_err(|error| RenderError::Encode(error.to_string()));
    let _ = forward.await;
    result
}

async fn encode_menu_audio(
    input: &Path,
    output: &Path,
    duration_ms: u64,
    audio: &dvdforge_domain::MenuAudio,
    motion: Option<&dvdforge_domain::MotionMenuConfig>,
) -> Result<(), RenderError> {
    let duration = format!("{:.3}", duration_ms as f64 / 1000.0);
    let duration_secs = duration_ms as f64 / 1000.0;
    let fade_in_ms = motion
        .map(|config| config.fade_in_ms)
        .unwrap_or(audio.fade_in_ms as u64);
    let fade_out_ms = motion
        .map(|config| config.fade_out_ms)
        .unwrap_or(audio.fade_out_ms as u64);
    let volume = motion
        .map(|config| config.audio_volume.clamp(0.0, 1.0))
        .unwrap_or(1.0);
    let mut filters = Vec::new();
    if (volume - 1.0).abs() > 0.01 {
        filters.push(format!("volume={volume:.3}"));
    }
    if audio.gain_db.abs() > 0.01 {
        filters.push(format!("volume={:.3}dB", audio.gain_db));
    }
    if fade_in_ms > 0 {
        filters.push(format!(
            "afade=t=in:st=0:d={:.3}",
            fade_in_ms as f64 / 1000.0
        ));
    }
    if fade_out_ms > 0 {
        let start = (duration_secs - fade_out_ms as f64 / 1000.0).max(0.0);
        filters.push(format!(
            "afade=t=out:st={start:.3}:d={:.3}",
            fade_out_ms as f64 / 1000.0
        ));
    }
    let mut command = tokio::process::Command::new(resolve_binary("ffmpeg"));
    command.kill_on_drop(true);
    command.arg("-hide_banner").arg("-y");
    let loop_audio = audio.loop_audio || motion.map(|config| config.loop_enabled).unwrap_or(false);
    if loop_audio {
        command.arg("-stream_loop").arg("-1");
    }
    push_file_input(&mut command, input);
    command.arg("-t").arg(&duration);
    if !filters.is_empty() {
        command.arg("-af").arg(filters.join(","));
    }
    let result = timeout(
        AUX_FFMPEG_TIMEOUT,
        command
            .arg("-c:a")
            .arg("ac3")
            .arg("-b:a")
            .arg("192k")
            .arg("-ar")
            .arg("48000")
            .arg("-ac")
            .arg("2")
            .arg("-f")
            .arg("ac3")
            .arg(output)
            .output(),
    )
    .await
    .map_err(|_| RenderError::Encode("FFmpeg agotó el tiempo del audio de menú".to_string()))?
    .map_err(|error| RenderError::Encode(error.to_string()))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(RenderError::Encode(
            String::from_utf8_lossy(&result.stderr).trim().to_string(),
        ))
    }
}

fn asset_path(project_dir: &Path, asset: &dvdforge_domain::MediaAsset) -> Option<PathBuf> {
    match &asset.source {
        AssetSource::Project => asset
            .relative_path
            .as_ref()
            .map(|relative| project_dir.join(relative)),
        AssetSource::External { absolute_path } => Some(PathBuf::from(absolute_path)),
    }
}

fn is_interlaced(probe: &MediaProbe) -> bool {
    probe
        .video
        .as_ref()
        .and_then(|video| video.field_order.as_deref())
        .map(|order| order != "progressive")
        .unwrap_or(false)
}

fn is_top_field_first(probe: &MediaProbe) -> bool {
    probe
        .video
        .as_ref()
        .and_then(|video| video.field_order.as_deref())
        .map(|order| order != "bb")
        .unwrap_or(true)
}

fn gop_for(standard: TvStandard) -> u16 {
    match standard {
        TvStandard::Ntsc => 12,
        TvStandard::Pal => 15,
    }
}

#[derive(Debug, Clone)]
struct CompiledHdMenu {
    pes: Vec<u8>,
    m2ts: Vec<u8>,
    pes_packets: Vec<Vec<u8>>,
}

struct HdMenuContext<'a> {
    episode_marks: &'a HashMap<EpisodeId, u16>,
    chapter_marks: &'a HashMap<(EpisodeId, u16), u16>,
    menu_objects: &'a HashMap<dvdforge_domain::MenuId, u16>,
}

#[derive(Debug, Clone)]
struct MenuClipSpec {
    width: u16,
    height: u16,
    fps: FrameRate,
    interlaced: bool,
    capacity: DiscCapacity,
    maxrate_kbps: u32,
    vbv_bufsize_kbps: u32,
    video: MenuClipVideo,
    audio: Option<MenuClipAudio>,
    overlays: Vec<MenuClipOverlay>,
}

#[derive(Debug, Clone)]
struct MenuClipOverlay {
    path: PathBuf,
    source_start_ms: u64,
    loop_duration_ms: u64,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

#[derive(Debug, Clone, Default)]
enum MenuClipVideo {
    #[default]
    Black,
    Image(PathBuf),
    Video {
        path: PathBuf,
        in_ms: u64,
        out_ms: Option<u64>,
    },
}

#[derive(Debug, Clone)]
struct MenuClipAudio {
    path: PathBuf,
    gain_db: f32,
    fade_in_ms: u32,
    fade_out_ms: u32,
    loop_audio: bool,
}

#[derive(Debug, Clone)]
struct MenuAttachReport {
    playlist: u16,
    clip: u16,
    object: u16,
    ig_packets: usize,
}

#[derive(Debug, Clone, Copy)]
struct MenuClipFormat {
    width: u16,
    height: u16,
    fps: FrameRate,
    interlaced: bool,
    capacity: DiscCapacity,
    maxrate_kbps: u32,
    vbv_bufsize_kbps: u32,
}

fn build_menu_clip_spec(
    project: &Project,
    project_dir: &Path,
    menu: &dvdforge_domain::Menu,
    composited_backgrounds: Option<&Path>,
    format: MenuClipFormat,
) -> MenuClipSpec {
    let video = match &menu.background {
        MenuBackground::VideoLoop {
            asset,
            in_frame,
            out_frame,
            ..
        } => project
            .asset(*asset)
            .and_then(|asset| asset_path(project_dir, asset))
            .map(|path| {
                let source_fps = project
                    .asset(*asset)
                    .and_then(|asset| asset.probe.video.as_ref())
                    .map(|video| video.frame_rate)
                    .filter(|rate| rate.is_valid())
                    .unwrap_or(FrameRate::FPS_23_976);
                let in_ms = if source_fps.is_valid() {
                    (*in_frame as f64 * 1000.0 / source_fps.as_f64()).round() as u64
                } else {
                    0
                };
                let out_ms = if *out_frame > *in_frame && source_fps.is_valid() {
                    Some((*out_frame as f64 * 1000.0 / source_fps.as_f64()).round() as u64)
                } else {
                    None
                };
                MenuClipVideo::Video {
                    path,
                    in_ms,
                    out_ms,
                }
            })
            .unwrap_or_default(),
        MenuBackground::Static { .. } => {
            let has_motion = menu
                .buttons
                .iter()
                .any(|button| button.motion_preview.is_some() && button.thumbnail.is_some());
            if has_motion {
                let composited = composited_backgrounds
                    .map(|directory| directory.join(format!("{}.png", menu.id)))
                    .filter(|path| path.is_file());
                let asset_image = match &menu.background {
                    MenuBackground::Static { asset } => project
                        .asset(*asset)
                        .and_then(|asset| asset_path(project_dir, asset))
                        .filter(|path| path.is_file()),
                    MenuBackground::VideoLoop { .. } => None,
                };
                composited
                    .or(asset_image)
                    .map(MenuClipVideo::Image)
                    .unwrap_or_default()
            } else {
                MenuClipVideo::Black
            }
        }
    };
    let audio = menu.audio.as_ref().and_then(|audio| {
        project
            .asset(audio.asset)
            .and_then(|asset| asset_path(project_dir, asset))
            .map(|path| MenuClipAudio {
                path,
                gain_db: audio.gain_db,
                fade_in_ms: audio.fade_in_ms,
                fade_out_ms: audio.fade_out_ms,
                loop_audio: audio.loop_audio,
            })
    });
    let overlays = menu
        .buttons
        .iter()
        .filter_map(|button| {
            let thumbnail = button.thumbnail.as_ref()?;
            let preview = button.motion_preview.as_ref()?;
            let path = project
                .asset(thumbnail.asset)
                .and_then(|asset| asset_path(project_dir, asset))?;
            let rect = normalize_rect(button.rect, menu.canvas, 8).ok()?;
            let x0 = scale_coord(rect.x0, menu.canvas.width, format.width);
            let y0 = scale_coord(rect.y0, menu.canvas.height, format.height);
            let x1 = scale_coord(rect.x1, menu.canvas.width, format.width).max(x0 + 2);
            let y1 = scale_coord(rect.y1, menu.canvas.height, format.height).max(y0 + 2);
            Some(MenuClipOverlay {
                path,
                source_start_ms: preview.source_start_ms,
                loop_duration_ms: preview.loop_duration_ms.max(1),
                x: x0,
                y: y0,
                width: x1.saturating_sub(x0).max(2),
                height: y1.saturating_sub(y0).max(2),
            })
        })
        .collect();
    MenuClipSpec {
        width: format.width,
        height: format.height,
        fps: format.fps,
        interlaced: format.interlaced,
        capacity: format.capacity,
        maxrate_kbps: format.maxrate_kbps,
        vbv_bufsize_kbps: format.vbv_bufsize_kbps,
        video,
        audio,
        overlays,
    }
}

async fn attach_hdmv_menu(
    bdmv_dir: &Path,
    work_dir: &Path,
    ig_packets: &[Vec<u8>],
    spec: &MenuClipSpec,
    expected_object: u16,
) -> Result<MenuAttachReport, RenderError> {
    if work_dir.exists() {
        std::fs::remove_dir_all(work_dir)?;
    }
    std::fs::create_dir_all(work_dir)?;

    let needed = pes_transport_packet_count(ig_packets);
    // A finite clip loops through the MovieObject. Keeping it long enough
    // leaves the remote responsive without relying on decoder-specific still
    // input handling.
    let duration_secs = 60.0;
    let dummy_path = work_dir.join("menu-pgs.sup");
    let dummy_pgs = dummy_pgs_for_slots(spec.width, spec.height, needed)
        .map_err(|error| RenderError::Menu(error.to_string()))?;
    std::fs::write(&dummy_path, dummy_pgs)?;

    let video_path = work_dir.join("menu.h264");
    encode_menu_video(&video_path, spec, duration_secs).await?;
    let audio_path = work_dir.join("menu.ac3");
    encode_hd_menu_audio(&audio_path, spec, duration_secs).await?;

    let meta_path = work_dir.join("menu.meta");
    let mux_mode = if spec.capacity.is_blu_ray() {
        "--blu-ray"
    } else {
        "--avchd"
    };
    // NOTE: raw H.264 takes no `track=` selector. Passing track=1 makes
    // tsMuxeR 2.7 skip every video frame and emit an empty playlist.
    let menu_meta = format!(
        "MUXOPT {mux_mode} --vbv-bufsize={} --vbv-maxrate={}\n\
         V_MPEG4/ISO/AVC, menu.h264, fps={:.3}, insertSEI, contSPS\n\
         A_AC3, menu.ac3, lang=und\n\
         S_HDMV/PGS, {}, track=1, lang=und\n",
        spec.vbv_bufsize_kbps,
        spec.maxrate_kbps,
        spec.fps.as_f64(),
        dummy_path.display()
    );
    std::fs::write(&meta_path, menu_meta)?;

    let menu_bdmv = work_dir.join("bdmv");
    TsMuxer::default()
        .mux_folder(&meta_path, &menu_bdmv, work_dir)
        .await
        .map_err(|error| RenderError::Author(error.to_string()))?;

    // tsMuxeR 2.7 may return before its asynchronous writer has flushed the
    // last playlist/clip files. Wait briefly for the complete tree.
    let menu_root = menu_bdmv.join("BDMV");
    for _ in 0..50 {
        if menu_root.join("STREAM/00000.m2ts").is_file()
            && menu_root.join("CLIPINF/00000.clpi").is_file()
            && menu_root.join("PLAYLIST/00000.mpls").is_file()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let menu_stream = read_bdmv_file(&menu_root.join("STREAM/00000.m2ts"))?;
    let menu_clpi = read_bdmv_file(&menu_root.join("CLIPINF/00000.clpi"))?;
    let menu_mpls = read_bdmv_file(&menu_root.join("PLAYLIST/00000.mpls"))?;

    let patched_stream = inject_ig_into_menu_m2ts(&menu_stream, ig_packets, dvdforge_hdmv::IG_PID)
        .map_err(|error| RenderError::Menu(error.to_string()))?;
    let source_packets = (patched_stream.len() / dvdforge_hdmv::bdmv::M2TS_PACKET_SIZE) as u32;
    let patched_clpi = patch_menu_clpi(&menu_clpi, source_packets)
        .map_err(|error| RenderError::Menu(error.to_string()))?;

    let bdmv_root = bdmv_dir.join("BDMV");
    let playlist = next_bdmv_number(&bdmv_root.join("PLAYLIST"), "mpls")?;
    let stream_number = next_bdmv_number(&bdmv_root.join("STREAM"), "m2ts")?;
    let clip_number = next_bdmv_number(&bdmv_root.join("CLIPINF"), "clpi")?;
    let clip = stream_number.max(clip_number);
    let patched_mpls = patch_menu_mpls(&menu_mpls, clip, None)
        .map_err(|error| RenderError::Menu(error.to_string()))?;

    write_bdmv_file(
        &bdmv_root.join("STREAM").join(format!("{clip:05}.m2ts")),
        &patched_stream,
    )?;
    write_bdmv_file(
        &bdmv_root.join("CLIPINF").join(format!("{clip:05}.clpi")),
        &patched_clpi,
    )?;
    write_bdmv_file(
        &bdmv_root
            .join("PLAYLIST")
            .join(format!("{playlist:05}.mpls")),
        &patched_mpls,
    )?;

    let movie_object_path = bdmv_root.join("MovieObject.bdmv");
    let movie_object = read_bdmv_file(&movie_object_path)?;
    let (movie_object, object_index) = append_menu_movie_object(&movie_object, playlist)
        .map_err(|error| RenderError::Menu(error.to_string()))?;
    if object_index != expected_object {
        return Err(RenderError::Menu(format!(
            "el objeto de menú esperado era {expected_object}, pero se creó {object_index}"
        )));
    }
    write_bdmv_file(&movie_object_path, &movie_object)?;
    let movie_object_backup = bdmv_root.join("BACKUP/MovieObject.bdmv");
    if movie_object_backup.is_file() {
        write_bdmv_file(&movie_object_backup, &movie_object)?;
    }

    Ok(MenuAttachReport {
        playlist,
        clip,
        object: object_index,
        ig_packets: needed,
    })
}

async fn encode_menu_video(
    path: &Path,
    spec: &MenuClipSpec,
    duration_secs: f64,
) -> Result<(), RenderError> {
    let rate = format!("{}/{}", spec.fps.num, spec.fps.den);
    let level = if spec.capacity.is_blu_ray() {
        "4.2"
    } else {
        "4.1"
    };
    let keyint = (spec.fps.as_f64().round() as u32).max(24);
    // x264 downgrades all-black ultrafast output to Constrained Baseline unless
    // the profile is pinned in x264 itself. HDMV expects High profile.
    let x264_params = format!(
        "profile=high:keyint={keyint}:min-keyint={keyint}:scenecut=0:open-gop=0:bluray-compat=1"
    );
    let mut command = Command::new(resolve_binary("ffmpeg"));
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    match &spec.video {
        MenuClipVideo::Black => {
            let filter = format!("color=c=black:s={}x{}:r={rate}", spec.width, spec.height);
            command.args(["-f", "lavfi", "-i", &filter]);
        }
        MenuClipVideo::Image(image) => {
            command.args(["-loop", "1"]);
            push_file_input(&mut command, image);
        }
        MenuClipVideo::Video {
            path: source,
            in_ms,
            out_ms,
        } => {
            let _ = out_ms;
            command.args(["-stream_loop", "-1"]);
            if *in_ms > 0 {
                command.args(["-ss", &format!("{:.3}", *in_ms as f64 / 1000.0)]);
            }
            push_file_input(&mut command, source);
        }
    }
    if spec.overlays.is_empty() {
        command.args(["-t", &format!("{duration_secs:.3}")]);
        if !matches!(spec.video, MenuClipVideo::Black) {
            let video_filter = format!(
                "scale={}:{}:force_original_aspect_ratio=decrease,                 pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,fps={rate}",
                spec.width, spec.height, spec.width, spec.height
            );
            command.args(["-vf", &video_filter]);
        }
    } else {
        let mut filter_graph = vec![format!(
            "[0:v]scale={}:{}:force_original_aspect_ratio=decrease,             pad={}:{}:(ow-iw)/2:(oh-ih)/2:color=black,fps={rate},             format=yuv420p,trim=duration={duration_secs:.3},             setpts=PTS-STARTPTS[base]",
            spec.width, spec.height, spec.width, spec.height
        )];
        let mut current = "base".to_string();
        for (index, overlay) in spec.overlays.iter().enumerate() {
            let input = index + 1;
            let _ = overlay.loop_duration_ms;
            command.args(["-stream_loop", "-1"]);
            if overlay.source_start_ms > 0 {
                command.args([
                    "-ss",
                    &format!("{:.3}", overlay.source_start_ms as f64 / 1000.0),
                ]);
            }
            push_file_input(&mut command, &overlay.path);
            filter_graph.push(format!(
                "[{input}:v]scale={}:{}:force_original_aspect_ratio=decrease,                 pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,setpts=PTS-STARTPTS[ov{index}]",
                overlay.width, overlay.height, overlay.width, overlay.height
            ));
            filter_graph.push(format!(
                "[{current}][ov{index}]overlay={}:{}:eof_action=repeat:shortest=0[v{index}]",
                overlay.x, overlay.y
            ));
            current = format!("v{index}");
        }
        filter_graph.push(format!("[{current}]null[out]"));
        command.args(["-filter_complex", &filter_graph.join(";")]);
        command.args(["-map", "[out]", "-t", &format!("{duration_secs:.3}")]);
    }
    command.args([
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-profile:v",
        "high",
        "-level",
        level,
        "-pix_fmt",
        "yuv420p",
        "-x264-params",
        &x264_params,
    ]);
    if spec.interlaced {
        command.args(["-flags", "+ildct+ilme", "-top", "1"]);
    }
    command.args(["-f", "h264"]).arg(path);
    run_menu_ffmpeg(command, path, "video").await
}

async fn encode_hd_menu_audio(
    path: &Path,
    spec: &MenuClipSpec,
    duration_secs: f64,
) -> Result<(), RenderError> {
    let mut command = Command::new(resolve_binary("ffmpeg"));
    command.args(["-hide_banner", "-loglevel", "error", "-y"]);
    let mut filters = Vec::new();
    if let Some(audio) = &spec.audio {
        if audio.loop_audio {
            command.args(["-stream_loop", "-1"]);
        }
        push_file_input(&mut command, &audio.path);
        command.args(["-t", &format!("{duration_secs:.3}")]);
        if audio.gain_db != 0.0 {
            filters.push(format!("volume={}dB", audio.gain_db));
        }
        if audio.fade_in_ms > 0 {
            filters.push(format!(
                "afade=t=in:st=0:d={:.3}",
                audio.fade_in_ms as f64 / 1000.0
            ));
        }
        if audio.fade_out_ms > 0 {
            let start = (duration_secs - audio.fade_out_ms as f64 / 1000.0).max(0.0);
            filters.push(format!(
                "afade=t=out:st={start:.3}:d={:.3}",
                audio.fade_out_ms as f64 / 1000.0
            ));
        }
        if !audio.loop_audio {
            // Keep the AC-3 track aligned with the menu clip even when the
            // configured music is shorter than the menu duration.
            filters.push("apad".to_string());
        }
    } else {
        command.args([
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=48000:cl=stereo",
            "-t",
            &format!("{duration_secs:.3}"),
        ]);
    }
    command.args(["-vn"]);
    if !filters.is_empty() {
        command.args(["-af", &filters.join(",")]);
    }
    command.args([
        "-c:a",
        "ac3",
        "-b:a",
        "192k",
        "-ar",
        "48000",
        "-ac",
        "2",
        path.to_str().unwrap_or("menu.ac3"),
    ]);
    run_menu_ffmpeg(command, path, "audio").await
}

async fn run_menu_ffmpeg(
    mut command: Command,
    path: &Path,
    label: &str,
) -> Result<(), RenderError> {
    command.kill_on_drop(true);
    let output = timeout(AUX_FFMPEG_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            RenderError::Encode(format!(
                "FFmpeg agotó el tiempo al generar el {label} del menú HDMV"
            ))
        })??;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(RenderError::Encode(format!(
            "FFmpeg no pudo generar el {label} del menú HDMV: {}",
            stderr.trim()
        )));
    }
    if !path.is_file() || std::fs::metadata(path)?.len() == 0 {
        return Err(RenderError::Encode(format!(
            "FFmpeg no generó el {label} del menú HDMV"
        )));
    }
    Ok(())
}

fn read_bdmv_file(path: &Path) -> Result<Vec<u8>, RenderError> {
    std::fs::read(path)
        .map_err(|error| RenderError::Menu(format!("no se pudo leer {}: {error}", path.display())))
}

fn write_bdmv_file(path: &Path, bytes: &[u8]) -> Result<(), RenderError> {
    std::fs::write(path, bytes).map_err(|error| {
        RenderError::Menu(format!("no se pudo escribir {}: {error}", path.display()))
    })
}

fn next_bdmv_number(directory: &Path, extension: &str) -> Result<u16, RenderError> {
    let mut max = None;
    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some(extension) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if stem.len() != 5 || !stem.chars().all(|character| character.is_ascii_digit()) {
                continue;
            }
            if let Ok(number) = stem.parse::<u16>() {
                max = Some(max.map_or(number, |current: u16| current.max(number)));
            }
        }
    }
    let next = max.map_or(0, |value| value.saturating_add(1));
    if next == u16::MAX {
        return Err(RenderError::Menu(
            "se agotaron los números de playlist/clip BDMV".to_string(),
        ));
    }
    Ok(next)
}

fn write_placeholder_background(path: &Path, width: u32, height: u32) -> Result<(), RenderError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::RgbaImage::from_pixel(width, height, image::Rgba([16, 24, 40, 255]))
        .save(path)
        .map_err(RenderError::from)?;
    Ok(())
}

fn write_transparent_overlay(path: &Path, width: u32, height: u32) -> Result<(), RenderError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 0]))
        .save(path)
        .map_err(RenderError::from)?;
    Ok(())
}

async fn log(events: &EventSink, message: String) {
    let _ = events
        .send(JobEventKind::Log {
            level: dvdforge_jobs::LogLevel::Info,
            message,
        })
        .await;
}

async fn stage(events: &EventSink, stage: JobStage, label: &str) {
    let _ = events
        .send(JobEventKind::StageStarted {
            stage,
            label: label.to_string(),
        })
        .await;
}

async fn progress(events: &EventSink, stage: JobStage, permille: u16) {
    let _ = events
        .send(JobEventKind::Progress {
            stage,
            permille,
            fps: None,
            speed: None,
            eta_secs: None,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_buttons_use_chapter_marks_before_jumping_to_the_movie() {
        let episode = EpisodeId::from_u128(7);
        let mut episode_marks = HashMap::new();
        episode_marks.insert(episode, 3);
        let chapter_marks = HashMap::new();
        let menu_objects = HashMap::new();
        let context = HdMenuContext {
            episode_marks: &episode_marks,
            chapter_marks: &chapter_marks,
            menu_objects: &menu_objects,
        };
        let commands = hdmv_commands(&ButtonAction::PlayEpisode { episode }, &context);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].dst, 0);
        assert_eq!(commands[0].src, 3);
        assert_eq!(commands[0].opcode(), dvdforge_hdmv::OP_SET_GPR);
        assert_eq!(commands[1].opcode(), dvdforge_hdmv::OP_JUMP_TITLE);
        assert_eq!(commands[1].dst, 1);
    }

    #[tokio::test]
    async fn cancelled_render_stops_before_encoding() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/minimal.json");
        let project = dvdforge_project::load_project(path).expect("fixture");
        let project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects");
        let output = std::env::temp_dir().join(format!(
            "dvdforge-render-cancel-test-{}",
            std::process::id()
        ));
        let cancel = Arc::new(AtomicBool::new(true));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let result = render_project_with_options(
            project,
            project_dir,
            output.clone(),
            JobId::new(),
            RenderOptions {
                cancel: Some(cancel),
                ..RenderOptions::default()
            },
            sender,
        )
        .await;
        while receiver.recv().await.is_some() {}
        assert!(matches!(result, Err(RenderError::Cancelled)));
        let _ = std::fs::remove_dir_all(output);
    }

    #[test]
    fn compiles_a_menu_to_an_hdmv_ig_pes_stream() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/minimal.json");
        let project = dvdforge_project::load_project(path).expect("fixture");
        let menu = project.menu(project.root_menu).expect("root menu");
        let episode_marks = HashMap::new();
        let chapter_marks = HashMap::new();
        let menu_objects = HashMap::new();
        let compiled = compile_hd_menu(
            &project,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects"),
            menu,
            None,
            HdMenuLayout {
                width: 1280,
                height: 720,
                pts_90k: HDMV_MENU_PTS_90K,
            },
            &HdMenuContext {
                episode_marks: &episode_marks,
                chapter_marks: &chapter_marks,
                menu_objects: &menu_objects,
            },
        )
        .expect("ig stream");
        assert_eq!(&compiled.pes[..4], &[0x00, 0x00, 0x01, 0xBD]);
        assert_eq!(compiled.m2ts[4], 0x47);
        assert!(compiled.pes.len() > 200);
        assert!(compiled
            .pes
            .windows(3)
            .any(|window| window == [0x00, 0x00, 0x01]));
        assert!(!compiled.pes_packets.is_empty());
    }
}
