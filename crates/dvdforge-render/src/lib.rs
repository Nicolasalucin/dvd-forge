//! Reusable end-to-end DVD-Video render pipeline shared by the CLI and Tauri.

use dvdforge_avchd::{
    concat_mkv, write_meta, AvchdAudioConfig, AvchdEncodeRequest, AvchdEncoder, AvchdMetaRequest,
    AvchdOutputs, TsMuxer,
};
use dvdforge_domain::{
    normalize_rect, AspectRatio, AssetId, AssetSource, AuthoringMode, ButtonAction, ButtonId,
    ButtonShapeMask, Diagnostic, DiscCapacity, DvdProfile, Episode, EpisodeId, FrameRate, JobId,
    MediaKind, MediaProbe, MenuBackground, MenuId, MenuScaleMode, PixelRect, Project, TvStandard,
    VideoConfig, VideoScaleMode,
};
use dvdforge_dvd::{
    build_dvdauthor_xml, AudioEncodeConfig, AuthoredAudioTrack, AuthoredButton,
    AuthoredSubtitleTrack, AuthoringRequest, DvdAuthor, DvdEncoder, EncodeOutputs, EncodeProgress,
    EncodeRequest, EpisodeFile, MenuEncodeRequest, MotionButtonEncode, MotionMenuEncodeRequest,
    RenderedMenu, VideoEncodeConfig,
};
use dvdforge_hdmv::{
    append_menu_movie_object, compile_ig_pes_packets, dummy_pgs_for_slots, frame_rate_code,
    inject_ig_into_menu_m2ts, jump_object, jump_title, m2ts_from_pes_packets, movie_object_count,
    patch_menu_clpi, patch_menu_mpls, pes_transport_packet_count, set_audio_stream, set_gpr,
    set_index_menu_object, HdmvButton, HdmvButtonState, HdmvCommand, HdmvMenu, RgbaBitmap,
};
use dvdforge_iso::{BdmvIsoBuilder, Xorriso};
use dvdforge_jobs::{ArtifactKind, JobEventKind, JobStage};
use dvdforge_media::Ffprobe;
use dvdforge_menu::{
    generate_menu_layers, shift_srt, write_button_shape_mask, write_spumux_xml, write_textsub_xml,
    CompiledButton, MenuCompileRequest, MenuLayerRequest, Spumux, TextSubtitleSpec,
};
use dvdforge_plan::{calculate_bitrate_plan, BitratePlan};
use dvdforge_store::{ArtifactKey, ArtifactKind as CacheArtifactKind, CacheLookup, Store};
use dvdforge_toolchain::{
    detect_hardware_encoders, resolve_binary, resolve_hardware_encoder, EncoderPreference,
    MEDIA_PROTOCOL_WHITELIST,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
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

/// Bump when the FFmpeg arguments or the encoded artifact layout change so
/// existing cache entries are invalidated.
const ENCODER_REV: u32 = 2;

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
    pub verification: Option<dvdforge_verify::report::VerifyReport>,
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
    /// Hardware encoder preference for AVCHD/Blu-ray H.264. DVD-Video always
    /// stays on the CPU MPEG-2 path.
    pub hardware_encoding: EncoderPreference,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            probe_assets: true,
            composited_backgrounds: None,
            cancel: None,
            hardware_encoding: EncoderPreference::Auto,
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
    let probe_report = collect_probes(&project, &project_dir, options.probe_assets).await;
    let probes = probe_report.probes;
    let mut warnings = probe_report.warnings;
    progress(&events, JobStage::Probe, 1000).await;

    stage(&events, JobStage::Plan, "Calculando bitrate").await;
    let plan = calculate_bitrate_plan(&project, &probes)
        .map_err(|error| RenderError::Plan(error.to_string()))?;
    warnings.extend(plan.warnings.clone());
    emit_warnings(&events, &warnings).await;
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
    let intro = encode_intro(
        &project,
        &probes,
        &project_dir,
        &output_dir,
        &encoder,
        &events,
    )
    .await?;
    progress(&events, JobStage::EncodeIntro, 1000).await;

    stage(
        &events,
        JobStage::EncodeEpisodes,
        "Codificando episodios y compilando menús",
    )
    .await;
    // Episodes and menus are independent: overlap them instead of adding the
    // whole menu encode to the episode encode. If one side fails, try_join!
    // drops the other future and its children are killed on drop.
    let (episodes, menus) = tokio::try_join!(
        encode_episodes(
            &project,
            &probes,
            &project_dir,
            &output_dir,
            &encoder,
            &plan,
            &events,
        ),
        render_menus(
            &project,
            &project_dir,
            &output_dir,
            &encoder,
            &events,
            composited_backgrounds.as_deref(),
        ),
    )?;

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
    if let Some(sector) = plan.layer_break_sector {
        let report =
            dvdforge_dvd::apply_layer_break(&dvd_dir, sector, project.disc.layer_break_mode)
                .map_err(|error| RenderError::Author(error.to_string()))?;
        if !report.exact {
            warn(
                &events,
                "W_LAYER_BREAK_APPROXIMATE",
                format!(
                    "El layer break solicitado ({}) se aplicó a la celda {} ({}–{}), que no contiene el sector exacto.",
                    report.requested_sector,
                    report.cell_index,
                    report.cell_start_sector,
                    report.cell_end_sector
                ),
            )
            .await;
        }
    }
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

    let verification = verify_artifact(&iso.iso_path, &events).await;
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
        verification,
    })
}

struct ProbeReport {
    probes: HashMap<AssetId, MediaProbe>,
    warnings: Vec<Diagnostic>,
}

async fn collect_probes(project: &Project, project_dir: &Path, refresh: bool) -> ProbeReport {
    let mut probes: HashMap<AssetId, MediaProbe> = HashMap::new();
    let mut warnings = Vec::new();
    if !refresh {
        for asset in &project.assets {
            probes.insert(asset.id, asset.probe.clone());
        }
        return ProbeReport { probes, warnings };
    }
    let ffprobe = Ffprobe::default();
    let permits = Arc::new(tokio::sync::Semaphore::new(4));
    let mut set = tokio::task::JoinSet::new();
    for asset in &project.assets {
        if asset.kind != MediaKind::Video {
            probes.insert(asset.id, asset.probe.clone());
            continue;
        }
        let Some(path) = asset_path(project_dir, asset) else {
            warnings.push(Diagnostic::warning(
                "W_PROBE_REFRESH_FAILED",
                "No se pudo resolver la ruta del asset; se usa la sonda guardada.",
            ));
            probes.insert(asset.id, asset.probe.clone());
            continue;
        };
        if !path.is_file() {
            warnings.push(Diagnostic::warning(
                "W_PROBE_REFRESH_FAILED",
                format!(
                    "El archivo del asset no existe ({}); se usa la sonda guardada.",
                    path.display()
                ),
            ));
            probes.insert(asset.id, asset.probe.clone());
            continue;
        }
        let id = asset.id;
        let ffprobe = ffprobe.clone();
        let permits = Arc::clone(&permits);
        set.spawn(async move {
            let result = async {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|error| format!("semáforo de sondeo cerrado: {error}"))?;
                ffprobe
                    .probe(&path)
                    .await
                    .map_err(|error| error.to_string())
            }
            .await;
            (id, result)
        });
    }
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((id, Ok(probe))) => {
                probes.insert(id, probe);
            }
            Ok((id, Err(error))) => {
                warnings.push(Diagnostic::warning(
                    "W_PROBE_REFRESH_FAILED",
                    format!("No se pudo refrescar la sonda del asset: {error}"),
                ));
                if let Some(asset) = project.assets.iter().find(|asset| asset.id == id) {
                    probes.insert(id, asset.probe.clone());
                }
            }
            Err(error) => warnings.push(Diagnostic::warning(
                "W_PROBE_REFRESH_FAILED",
                format!("La tarea de sondeo terminó inesperadamente: {error}"),
            )),
        }
    }
    ProbeReport { probes, warnings }
}

async fn emit_warnings(events: &EventSink, warnings: &[Diagnostic]) {
    for warning in warnings {
        let _ = events
            .send(JobEventKind::Warning {
                code: warning.code.clone(),
                message: warning.message.clone(),
            })
            .await;
    }
}

async fn verify_artifact(
    path: &Path,
    events: &EventSink,
) -> Option<dvdforge_verify::report::VerifyReport> {
    let path = path.to_path_buf();
    match tokio::task::spawn_blocking(move || dvdforge_verify::verify_iso(&path, false)).await {
        Ok(Ok(report)) => {
            if report.verdict == dvdforge_verify::report::VerifyVerdict::Fail {
                warn(
                    events,
                    "W_STRUCTURE_VERIFY_FAILED",
                    "La verificación estructural de la ISO encontró errores bloqueantes."
                        .to_string(),
                )
                .await;
            } else if report.verdict == dvdforge_verify::report::VerifyVerdict::Warn {
                warn(
                    events,
                    "W_STRUCTURE_VERIFY_WARN",
                    "La verificación estructural de la ISO encontró avisos.".to_string(),
                )
                .await;
            }
            Some(report)
        }
        Ok(Err(error)) => {
            warn(
                events,
                "W_STRUCTURE_VERIFY_FAILED",
                format!("No se pudo verificar la ISO generada: {error}"),
            )
            .await;
            None
        }
        Err(error) => {
            warn(
                events,
                "W_STRUCTURE_VERIFY_FAILED",
                format!("La verificación estructural terminó inesperadamente: {error}"),
            )
            .await;
            None
        }
    }
}

/// Full-content fingerprint used by the render cache.
///
/// Hashing a multi-gigabyte source costs one sequential I/O pass, but a
/// size/mtime/edge sample cannot detect an edit in the middle of a file. The
/// asset SHA-256 recorded at import time is reused when it is a real hash;
/// otherwise this function computes it and returns the `fp2:` form.
async fn source_fingerprint(path: &Path) -> Result<String, RenderError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        let mut file = std::fs::File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("fp2:{}", hex::encode(hasher.finalize())))
    })
    .await
    .map_err(|error| RenderError::Encode(format!("huella cancelada: {error}")))?
    .map_err(RenderError::Io)
}

async fn source_fingerprint_for_asset(
    path: &Path,
    asset: &dvdforge_domain::MediaAsset,
) -> Result<String, RenderError> {
    let sha = asset.sha256.trim();
    if sha.len() == 64
        && sha != "0".repeat(64)
        && sha.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Ok(format!("fp2:{sha}"));
    }
    source_fingerprint(path).await
}

fn audio_params_hash(episode: &Episode) -> String {
    let mut hasher = Sha256::new();
    for audio in &episode.audio {
        hasher.update(audio.source_stream.to_be_bytes());
        hasher.update(audio.language.as_bytes());
        hasher.update([0]);
        hasher.update(audio.label.as_bytes());
        hasher.update([0]);
        if let Some(path) = &audio.source_path {
            hasher.update(path.as_bytes());
        }
        hasher.update([0]);
        hasher.update(audio.gain_db.to_bits().to_be_bytes());
        hasher.update(audio.ac3_kbps.to_be_bytes());
        hasher.update(audio.delay_ms.to_be_bytes());
        hasher.update([
            match audio.format {
                dvdforge_domain::AudioFormat::Ac3Stereo => 1,
                dvdforge_domain::AudioFormat::Ac3Surround51 => 2,
            },
            u8::from(audio.default),
            u8::from(audio.commentary),
        ]);
    }
    hex::encode(hasher.finalize())
}

#[derive(Serialize)]
struct EpisodeRenderInputs<'a> {
    kind: &'a str,
    source_fingerprint: &'a str,
    trim_in_ms: u64,
    trim_out_ms: u64,
    target_bitrate: u32,
    audio_params_hash: &'a str,
    fps_num: u32,
    fps_den: u32,
    scale_mode: VideoScaleMode,
    video_kbps: u32,
    maxrate_kbps: u32,
    profile: String,
    gop_frames: u16,
    b_frames: u8,
    b_strategy: u8,
    interlaced: bool,
    top_field_first: bool,
    dc_precision: u8,
    resolution: Option<String>,
    standard: TvStandard,
    aspect: AspectRatio,
    subtitles_hash: String,
}

#[derive(Serialize)]
struct MenuRenderInputs<'a> {
    menu: &'a dvdforge_domain::Menu,
    background_sha: String,
    audio_sha: Option<String>,
    composited_labels_sha: Option<String>,
}

fn canonical_hash<T: Serialize>(value: &T) -> Result<String, RenderError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        RenderError::Encode(format!("no se pudo serializar la clave de caché: {error}"))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

fn subtitle_inputs_hash(episode: &Episode, project_dir: &Path) -> String {
    let mut hasher = Sha256::new();
    for track in &episode.subtitles {
        hasher.update(track.source_stream.to_be_bytes());
        hasher.update(track.language.as_bytes());
        hasher.update([0]);
        hasher.update(track.label.as_bytes());
        hasher.update([0]);
        hasher.update(track.delay_ms.to_be_bytes());
        hasher.update([u8::from(track.is_forced), u8::from(track.is_default)]);
        if let Some(source_path) = &track.source_path {
            let source = resolve_project_path(project_dir, source_path);
            match std::fs::read(&source) {
                Ok(bytes) => hasher.update(bytes),
                Err(error) => hasher.update(error.to_string().as_bytes()),
            }
        } else {
            hasher.update(b"embedded");
        }
        hasher.update([0xff]);
    }
    hex::encode(hasher.finalize())
}

fn dc_precision_for_kbps(kbps: u32) -> u8 {
    if kbps >= 5_000 {
        10
    } else if kbps >= 3_200 {
        9
    } else {
        8
    }
}

fn toolchain_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let ffmpeg = resolve_binary("ffmpeg");
        let mut hasher = Sha256::new();
        hasher.update(ffmpeg.display().to_string().as_bytes());
        if let Ok(output) = std::process::Command::new(&ffmpeg).arg("-version").output() {
            hasher.update(&output.stdout);
        }
        format!("tc:{}", hex::encode(hasher.finalize()))
    })
}

fn menu_background_sha(project: &Project, menu: &dvdforge_domain::Menu) -> String {
    let asset_id = match &menu.background {
        MenuBackground::Static { asset } | MenuBackground::VideoLoop { asset, .. } => *asset,
    };
    project
        .asset(asset_id)
        .map(|asset| asset.sha256.clone())
        .unwrap_or_else(|| "missing".to_string())
}

fn menu_audio_sha(project: &Project, menu: &dvdforge_domain::Menu) -> Option<String> {
    menu.audio
        .as_ref()
        .and_then(|audio| project.asset(audio.asset))
        .map(|asset| {
            format!(
                "{}:{}",
                asset.sha256,
                menu.audio.as_ref().map(|a| a.gain_db).unwrap_or(0.0)
            )
        })
}

fn composited_labels_sha(composited_backgrounds: Option<&Path>, menu_id: MenuId) -> Option<String> {
    let path = composited_backgrounds
        .map(|directory| directory.join(format!("labels-{menu_id}.png")))
        .filter(|path| path.is_file())?;
    std::fs::read(path)
        .ok()
        .map(|bytes| hex::encode(Sha256::digest(bytes)))
}

fn menu_inputs_hash(
    project: &Project,
    menu: &dvdforge_domain::Menu,
    composited_backgrounds: Option<&Path>,
) -> Result<String, RenderError> {
    canonical_hash(&MenuRenderInputs {
        menu,
        background_sha: menu_background_sha(project, menu),
        audio_sha: menu_audio_sha(project, menu),
        composited_labels_sha: composited_labels_sha(composited_backgrounds, menu.id),
    })
}

async fn cached_artifact_path(
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
    let (from, to) = (source.to_path_buf(), target.clone());
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let _ = std::fs::remove_file(&to);
        // Prefer a hard link: the artifact is immutable once published and the
        // store normally lives on the same filesystem as the render output.
        std::fs::hard_link(&from, &to).or_else(|_| {
            // A failed cross-volume copy must not leave an orphan partial blob.
            std::fs::copy(&from, &to).map(|_| ()).inspect_err(|_| {
                let _ = std::fs::remove_file(&to);
            })
        })
    })
    .await
    .map_err(|error| RenderError::Encode(format!("copia de caché cancelada: {error}")))??;
    if let Err(error) = store.put_artifact(key, &target, duration_ms) {
        // Unregistered blobs are never looked up or evicted again.
        let _ = std::fs::remove_file(&target);
        return Err(RenderError::Encode(error.to_string()));
    }
    Ok(target)
}

async fn materialize_cached_artifact(source: &Path, target: &Path) -> Result<PathBuf, RenderError> {
    let (from, to) = (source.to_path_buf(), target.to_path_buf());
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&to);
        std::fs::hard_link(&from, &to).or_else(|_| {
            std::fs::copy(&from, &to).map(|_| ()).inspect_err(|_| {
                let _ = std::fs::remove_file(&to);
            })
        })
    })
    .await
    .map_err(|error| RenderError::Encode(format!("copia de caché cancelada: {error}")))??;
    Ok(target.to_path_buf())
}

fn prepare_output(output: &Path) -> Result<(), RenderError> {
    for directory in ["dvd", "intro", "episodes", "menus", "xml"] {
        let path = output.join(directory);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
        std::fs::create_dir_all(&path)?;
    }
    // Do not delete the previous disc.iso here: the ISO builders write a
    // temporary sibling and atomically rename it over the target only after
    // validation, so a failed or cancelled render keeps the last good image.
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
    // Keep the previous disc.iso until BdmvIsoBuilder atomically replaces it.
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
    // Compile once; the concatenated PES and the debug M2TS derive from the packets.
    let pes_packets = compile_ig_pes_packets(&menu, pts_90k).map_err(menu_error)?;
    let pes = pes_packets.concat();
    let m2ts = m2ts_from_pes_packets(&pes_packets);
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
            // Button-less menus get no MovieObject; JumpTitle(0) is not a
            // valid HDMV title (1..=999), so keep the button inert instead.
            .unwrap_or_else(|| vec![HdmvCommand::new(dvdforge_hdmv::OP_NOP, 0, 0)]),
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
    let probe_report = collect_probes(&project, &project_dir, options.probe_assets).await;
    let probes = probe_report.probes;
    let mut probe_warnings = probe_report.warnings;
    progress(&events, JobStage::Probe, 1000).await;

    stage(&events, JobStage::Plan, "Calculando bitrate AVCHD").await;
    let plan = calculate_bitrate_plan(&project, &probes)
        .map_err(|error| RenderError::Plan(error.to_string()))?;
    probe_warnings.extend(plan.warnings.clone());
    emit_warnings(&events, &probe_warnings).await;
    if !plan.feasible {
        return Err(RenderError::Plan(
            "el plan AVCHD no es viable; revisa las advertencias".to_string(),
        ));
    }
    progress(&events, JobStage::Plan, 1000).await;
    ensure_not_cancelled(&cancel)?;

    prepare_avchd_output(&output_dir)?;
    let encoder = AvchdEncoder::default();
    let hardware_capabilities = detect_hardware_encoders(&encoder.ffmpeg);
    let hardware_encoder =
        resolve_hardware_encoder(options.hardware_encoding, &hardware_capabilities);
    if let Some(encoder) = hardware_encoder {
        log(
            &events,
            format!(
                "Codificación AVCHD con aceleración hardware: {}",
                encoder.label()
            ),
        )
        .await;
    }
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
    let cache = match Store::open_default() {
        Ok(store) => Some(store),
        Err(error) => {
            warn(
                &events,
                "W_CACHE_DISABLED",
                format!("La caché de artefactos está desactivada: {error}"),
            )
            .await;
            None
        }
    };

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
            let source_fp = source_fingerprint_for_asset(&input, asset).await?;
            let audio_hash = audio_params_hash(episode);
            let subtitles_hash = subtitle_inputs_hash(episode, &project_dir);
            let inputs_hash = canonical_hash(&EpisodeRenderInputs {
                kind: "avchd",
                source_fingerprint: &source_fp,
                trim_in_ms: episode.trim_start_ms.unwrap_or(0),
                trim_out_ms: episode.trim_end_ms.unwrap_or(duration_ms),
                target_bitrate: planned_kbps,
                audio_params_hash: &audio_hash,
                fps_num: config.frame_rate.num,
                fps_den: config.frame_rate.den,
                scale_mode: episode.scale_mode,
                video_kbps: planned_kbps,
                maxrate_kbps: config.vbv_maxrate_kbps,
                profile: format!("{:?}:{:?}", config.resolution, hardware_encoder),
                gop_frames: config.keyint_frames,
                b_frames: 3,
                b_strategy: 2,
                interlaced: config.resolution.interlaced(),
                top_field_first: true,
                dc_precision: 8,
                resolution: Some(format!(
                    "{}x{}",
                    config.resolution.dimensions().0,
                    config.resolution.dimensions().1
                )),
                standard: project.disc.standard,
                aspect: project.disc.aspect,
                subtitles_hash,
            })?;
            Some(ArtifactKey {
                kind: CacheArtifactKind::Episode,
                source_fingerprint: source_fp,
                inputs_hash,
                target_profile: format!(
                    "hdmv:{:?}:{:.3}:{:?}:{}:{:?}:{}:{}:{}:{}",
                    config.resolution,
                    config.frame_rate.as_f64(),
                    episode.scale_mode,
                    project.disc.capacity.label(),
                    project.disc.aspect,
                    config.vbv_maxrate_kbps,
                    config.vbv_bufsize_kbps,
                    project.disc.audio.channels,
                    project.disc.audio.ac3_kbps
                ),
                target_bitrate: planned_kbps,
                audio_params_hash: audio_hash,
                encoder_rev: ENCODER_REV,
                toolchain_hash: toolchain_hash().to_string(),
            })
        } else {
            None
        };
        let cached = match cache.as_ref().zip(cache_key.as_ref()) {
            Some((store, key)) => match store.lookup_artifact(key) {
                CacheLookup::Hit(artifact) if Path::new(&artifact.artifact_path).is_file() => {
                    Some(artifact)
                }
                CacheLookup::Hit(_) => None,
                CacheLookup::Stale(reason) => {
                    warn(
                        &events,
                        "W_CACHE_STALE",
                        format!("La caché de artefactos quedó obsoleta: {reason}"),
                    )
                    .await;
                    None
                }
                CacheLookup::Miss => None,
                CacheLookup::Error(error) => {
                    warn(
                        &events,
                        "W_CACHE_READ_FAILED",
                        format!("No se pudo leer la caché de artefactos: {error}"),
                    )
                    .await;
                    None
                }
            },
            None => None,
        };
        if let Some(artifact) = &cached {
            log(
                &events,
                format!(
                    "Episodio AVCHD {}/{}: reutilizando caché de artefactos {}",
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
            hardware_encoder,
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
                match cached_artifact_path(store, key, "mkv", &encoded.stream_mkv, duration_ms)
                    .await
                {
                    Ok(path) => path,
                    Err(error) => {
                        warn(
                            &events,
                            "W_CACHE_WRITE_FAILED",
                            format!("No se pudo publicar el artefacto AVCHD en caché: {error}"),
                        )
                        .await;
                        encoded.stream_mkv
                    }
                }
            } else {
                encoded.stream_mkv
            }
        };
        stream_inputs.push(stream_mkv);
        chapters.push(timeline_ms);
        episode_start_ms.insert(episode.id, timeline_ms);
        // Chapter::at_frame is authored in disc-rate frames (the editor's rate,
        // which the encoder conforms to with `fps=`), not in source frames.
        episode_fps.insert(episode.id, config.frame_rate);
        for chapter in &episode.chapters {
            let offset_ms = if config.frame_rate.is_valid() {
                (chapter.at_frame as f64 * 1000.0 / config.frame_rate.as_f64()).round() as u64
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

    let mut warnings = probe_warnings;
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

    let verification = verify_artifact(&final_iso_path, &events).await;
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
        verification,
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
    let result = match encoder.encode_with_progress(request, Some(sender)).await {
        Ok(output) if request.hardware_encoder.is_some() => {
            match validate_h264_output(&output.stream_mkv).await {
                Ok(()) => Ok(output),
                Err(error) => {
                    warn(
                        events,
                        "W_HW_ENCODER_FALLBACK",
                        format!(
                            "La salida hardware no cumple el perfil AVCHD/Blu-ray ({error}); se reintenta en CPU/libx264."
                        ),
                    )
                    .await;
                    let mut fallback = request.clone();
                    fallback.hardware_encoder = None;
                    encoder
                        .encode(&fallback)
                        .await
                        .map_err(|error| RenderError::Encode(error.to_string()))
                }
            }
        }
        Ok(output) => Ok(output),
        Err(error) if request.hardware_encoder.is_some() => {
            warn(
                events,
                "W_HW_ENCODER_FALLBACK",
                format!("El encoder hardware falló; se reintenta en CPU/libx264: {error}"),
            )
            .await;
            let mut fallback = request.clone();
            fallback.hardware_encoder = None;
            encoder
                .encode(&fallback)
                .await
                .map_err(|error| RenderError::Encode(error.to_string()))
        }
        Err(error) => Err(RenderError::Encode(error.to_string())),
    };
    let _ = forward.await;
    result
}

async fn validate_h264_output(path: &Path) -> Result<(), String> {
    let output = tokio::process::Command::from(dvdforge_toolchain::background_command(
        resolve_binary("ffprobe"),
    ))
    .args([
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=codec_name,profile,level,pix_fmt",
        "-of",
        "json",
    ])
    .arg(path)
    .kill_on_drop(true)
    .output()
    .await
    .map_err(|error| format!("no se pudo ejecutar ffprobe: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("ffprobe devolvió JSON inválido: {error}"))?;
    let stream = value
        .get("streams")
        .and_then(|streams| streams.get(0))
        .ok_or_else(|| "ffprobe no devolvió un stream de vídeo".to_string())?;
    let codec = stream
        .get("codec_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let profile = stream
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let pix_fmt = stream
        .get("pix_fmt")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if codec != "h264" {
        return Err(format!("códec {codec}, se esperaba h264"));
    }
    if !profile.eq_ignore_ascii_case("high") {
        return Err(format!("perfil {profile}, se esperaba High"));
    }
    if pix_fmt != "yuv420p" {
        return Err(format!("pix_fmt {pix_fmt}, se esperaba yuv420p"));
    }
    Ok(())
}

async fn encode_intro(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    project_dir: &Path,
    output: &Path,
    encoder: &DvdEncoder,
    events: &EventSink,
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
    let cache = match Store::open_default() {
        Ok(store) => Some(store),
        Err(error) => {
            warn(
                events,
                "W_CACHE_DISABLED",
                format!("La caché de artefactos está desactivada: {error}"),
            )
            .await;
            None
        }
    };
    let cache_key = if cache.is_some() {
        let source_fp = source_fingerprint_for_asset(&input, asset).await?;
        let audio_hash = canonical_hash(&first_play.audio)?;
        let inputs_hash = canonical_hash(&EpisodeRenderInputs {
            kind: "intro",
            source_fingerprint: &source_fp,
            trim_in_ms: 0,
            trim_out_ms: first_play
                .trim
                .map(|trim| (trim.duration() as f64 * 1000.0 / fps.as_f64()) as u64)
                .unwrap_or(0),
            target_bitrate: 5_000,
            audio_params_hash: &audio_hash,
            fps_num: fps.num,
            fps_den: fps.den,
            scale_mode: VideoScaleMode::Letterbox,
            video_kbps: 5_000,
            maxrate_kbps: 8_000,
            profile: "D1".to_string(),
            gop_frames: gop_for(project.disc.standard),
            b_frames: 2,
            b_strategy: 2,
            interlaced: is_interlaced(probe),
            top_field_first: is_top_field_first(probe),
            dc_precision: dc_precision_for_kbps(5_000),
            resolution: None,
            standard: project.disc.standard,
            aspect: project.disc.aspect,
            subtitles_hash: String::new(),
        })?;
        Some(ArtifactKey {
            kind: CacheArtifactKind::Intro,
            source_fingerprint: source_fp,
            inputs_hash,
            target_profile: format!(
                "dvd:intro:{:?}:{:?}:{:?}",
                project.disc.standard, project.disc.aspect, project.disc.capacity
            ),
            target_bitrate: 5_000,
            audio_params_hash: audio_hash,
            encoder_rev: ENCODER_REV,
            toolchain_hash: toolchain_hash().to_string(),
        })
    } else {
        None
    };
    if let (Some(store), Some(key)) = (&cache, &cache_key) {
        match store.lookup_artifact(key) {
            CacheLookup::Hit(artifact) if Path::new(&artifact.artifact_path).is_file() => {
                log(
                    events,
                    format!("Intro: reutilizando caché {}", artifact.artifact_path),
                )
                .await;
                return Ok(Some(PathBuf::from(artifact.artifact_path)));
            }
            CacheLookup::Stale(reason) => {
                warn(
                    events,
                    "W_CACHE_STALE",
                    format!("La caché de intro quedó obsoleta: {reason}"),
                )
                .await;
            }
            CacheLookup::Error(error) => {
                warn(
                    events,
                    "W_CACHE_READ_FAILED",
                    format!("No se pudo leer la caché de intro: {error}"),
                )
                .await;
            }
            CacheLookup::Hit(_) | CacheLookup::Miss => {}
        }
    }
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
        passlog: output.join("intro").join("intro.passlog"),
    };
    let encoded = encoder
        .encode(&request)
        .await
        .map_err(|error| RenderError::Encode(error.to_string()))?;
    if let (Some(store), Some(key)) = (&cache, &cache_key) {
        match cached_artifact_path(store, key, "mpg", &encoded.muxed_mpg, 0).await {
            Ok(path) => return Ok(Some(path)),
            Err(error) => {
                warn(
                    events,
                    "W_CACHE_WRITE_FAILED",
                    format!("No se pudo publicar la intro en caché: {error}"),
                )
                .await
            }
        }
    }
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
    let extracted_path = |index: usize| {
        context
            .output_dir
            .join(format!("{}.s{}.source.srt", context.stem, index + 1))
    };
    // Extract every embedded track in ONE demux pass: extra `-map` outputs are
    // free, while one FFmpeg run per track re-reads the whole source file.
    let embedded: Vec<(usize, usize)> = context
        .episode
        .subtitles
        .iter()
        .enumerate()
        .filter(|(_, track)| track.source_path.is_none() && !track.bitmap)
        .map(|(index, track)| (index, track.source_stream as usize))
        .collect();
    if !embedded.is_empty() {
        let asset = context
            .project
            .asset(context.episode.video)
            .ok_or_else(|| RenderError::Menu("episodio sin asset".to_string()))?;
        let video = asset_path(context.project_dir, asset)
            .ok_or_else(|| RenderError::Menu("ruta de vídeo inválida".to_string()))?;
        let mut command =
            tokio::process::Command::from(dvdforge_toolchain::background_command(context.ffmpeg));
        command
            .arg("-hide_banner")
            .arg("-y")
            .arg("-protocol_whitelist")
            .arg(MEDIA_PROTOCOL_WHITELIST)
            .arg("-i")
            .arg(&video)
            .kill_on_drop(true);
        for &(index, stream_index) in &embedded {
            command
                .arg("-map")
                .arg(format!("0:{stream_index}"))
                .arg("-c:s")
                .arg("srt")
                .arg(extracted_path(index));
        }
        let result = timeout(AUX_FFMPEG_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                RenderError::Encode("FFmpeg agotó el tiempo al extraer subtítulos".to_string())
            })?
            .map_err(|error| RenderError::Encode(error.to_string()))?;
        if !result.status.success() {
            return Err(RenderError::Menu(format!(
                "No se pudieron extraer los subtítulos internos: {}",
                String::from_utf8_lossy(&result.stderr).trim()
            )));
        }
    }
    for (index, track) in context.episode.subtitles.iter().enumerate() {
        if track.bitmap {
            warn(
                events,
                "W_BITMAP_SUBTITLE_UNSUPPORTED",
                format!(
                    "Episodio {}: la pista de subtítulos bitmap «{}» ({}) se omite; requiere OCR.",
                    context.episode.number, track.label, track.language
                ),
            )
            .await;
            continue;
        }
        let text = if let Some(source_path) = &track.source_path {
            let source = resolve_project_path(context.project_dir, source_path);
            std::fs::read_to_string(&source).map_err(|error| {
                RenderError::Menu(format!(
                    "No se pudo leer el subtítulo {}: {error}",
                    source.display()
                ))
            })?
        } else {
            std::fs::read_to_string(extracted_path(index))?
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
    let cache = match Store::open_default() {
        Ok(store) => Some(store),
        Err(error) => {
            warn(
                events,
                "W_CACHE_DISABLED",
                format!("La caché de artefactos está desactivada: {error}"),
            )
            .await;
            None
        }
    };
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
            let source_fp = source_fingerprint_for_asset(&input, asset).await?;
            let audio_hash = audio_params_hash(episode);
            let subtitles_hash = subtitle_inputs_hash(episode, project_dir);
            let inputs_hash = canonical_hash(&EpisodeRenderInputs {
                kind: "dvd",
                source_fingerprint: &source_fp,
                trim_in_ms: episode.trim_start_ms.unwrap_or(0),
                trim_out_ms: episode.trim_end_ms.unwrap_or(duration),
                target_bitrate: video_kbps,
                audio_params_hash: &audio_hash,
                fps_num: fps.num,
                fps_den: fps.den,
                scale_mode: episode.scale_mode,
                video_kbps,
                maxrate_kbps,
                profile: format!("{:?}", plan.profile),
                gop_frames: gop_for(project.disc.standard),
                b_frames: 2,
                b_strategy: 2,
                interlaced: is_interlaced(probe),
                top_field_first: is_top_field_first(probe),
                dc_precision: dc_precision_for_kbps(video_kbps),
                resolution: None,
                standard: project.disc.standard,
                aspect: project.disc.aspect,
                subtitles_hash,
            })?;
            Some(ArtifactKey {
                kind: CacheArtifactKind::Episode,
                source_fingerprint: source_fp,
                inputs_hash,
                target_profile: format!(
                    "dvd:{:?}:{:?}:{:?}:{:?}",
                    plan.profile, project.disc.standard, project.disc.aspect, episode.scale_mode
                ),
                target_bitrate: video_kbps,
                audio_params_hash: audio_hash,
                encoder_rev: ENCODER_REV,
                toolchain_hash: toolchain_hash().to_string(),
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
            passlog: output
                .join("episodes")
                .join(format!("episode-{:02}.passlog", episode.number)),
        };
        let cached = match cache.as_ref().zip(cache_key.as_ref()) {
            Some((store, key)) => match store.lookup_artifact(key) {
                CacheLookup::Hit(artifact) if Path::new(&artifact.artifact_path).is_file() => {
                    Some(artifact)
                }
                CacheLookup::Hit(_) => None,
                CacheLookup::Stale(reason) => {
                    warn(
                        events,
                        "W_CACHE_STALE",
                        format!("La caché de artefactos quedó obsoleta: {reason}"),
                    )
                    .await;
                    None
                }
                CacheLookup::Miss => None,
                CacheLookup::Error(error) => {
                    warn(
                        events,
                        "W_CACHE_READ_FAILED",
                        format!("No se pudo leer la caché de artefactos: {error}"),
                    )
                    .await;
                    None
                }
            },
            None => None,
        };
        let base_mpeg = if let Some(artifact) = cached {
            log(
                events,
                format!(
                    "Episodio {}/{}: reutilizando caché de artefactos {}",
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
            // Only the muxed program stream is used from here on; the elementary
            // streams and the pass log would double the scratch footprint.
            let _ = std::fs::remove_file(&encoded.video_m2v);
            for ac3 in &encoded.audio_ac3 {
                let _ = std::fs::remove_file(ac3);
            }
            let _ = std::fs::remove_file(encoded.passlog.with_extension("passlog-0.log"));
            if let (Some(store), Some(key)) = (&cache, &cache_key) {
                match cached_artifact_path(store, key, "mpg", &encoded.muxed_mpg, duration).await {
                    Ok(path) => path,
                    Err(error) => {
                        warn(
                            events,
                            "W_CACHE_WRITE_FAILED",
                            format!("No se pudo publicar el artefacto en caché: {error}"),
                        )
                        .await;
                        encoded.muxed_mpg.clone()
                    }
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
    let cache = match Store::open_default() {
        Ok(store) => Some(store),
        Err(error) => {
            warn(
                events,
                "W_CACHE_DISABLED",
                format!("La caché de menús está desactivada: {error}"),
            )
            .await;
            None
        }
    };
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

        let mut base_mpeg = menu_dir.join("menu_base.mpg");
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
        let menu_video_kbps = if animated { 6_000 } else { 5_000 };
        let cache_key = if cache.is_some() {
            let inputs_hash = menu_inputs_hash(project, menu, composited_backgrounds)?;
            Some(ArtifactKey {
                kind: CacheArtifactKind::MenuVideo,
                source_fingerprint: format!("fp2:menu:{inputs_hash}"),
                inputs_hash,
                target_profile: format!(
                    "menu:{:?}:{}x{}:{:.3}:{}",
                    project.disc.standard,
                    menu.canvas.width,
                    menu.canvas.height,
                    menu.timing.duration_ms as f64 / 1000.0,
                    menu_video_kbps
                ),
                target_bitrate: menu_video_kbps,
                audio_params_hash: menu_audio_sha(project, menu).unwrap_or_default(),
                encoder_rev: ENCODER_REV,
                toolchain_hash: toolchain_hash().to_string(),
            })
        } else {
            None
        };
        let mut cache_hit = false;
        if let (Some(store), Some(key)) = (&cache, &cache_key) {
            match store.lookup_artifact(key) {
                CacheLookup::Hit(artifact) if Path::new(&artifact.artifact_path).is_file() => {
                    log(
                        events,
                        format!(
                            "Menú {}/{}: reutilizando caché {}",
                            index + 1,
                            total,
                            artifact.artifact_path
                        ),
                    )
                    .await;
                    let local = menu_dir.join("menu_base.mpg");
                    match materialize_cached_artifact(Path::new(&artifact.artifact_path), &local)
                        .await
                    {
                        Ok(path) => base_mpeg = path,
                        Err(error) => {
                            warn(
                                events,
                                "W_CACHE_MATERIALIZE_FAILED",
                                format!(
                                    "No se pudo materializar el menú cacheado; se usa el blob: {error}"
                                ),
                            )
                            .await;
                            base_mpeg = PathBuf::from(artifact.artifact_path);
                        }
                    }
                    cache_hit = true;
                }
                CacheLookup::Stale(reason) => {
                    warn(
                        events,
                        "W_CACHE_STALE",
                        format!("La caché del menú quedó obsoleta: {reason}"),
                    )
                    .await;
                }
                CacheLookup::Error(error) => {
                    warn(
                        events,
                        "W_CACHE_READ_FAILED",
                        format!("No se pudo leer la caché del menú: {error}"),
                    )
                    .await;
                }
                CacheLookup::Hit(_) | CacheLookup::Miss => {}
            }
        }
        if !cache_hit {
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
            if let (Some(store), Some(key)) = (&cache, &cache_key) {
                match cached_artifact_path(
                    store,
                    key,
                    "mpg",
                    &base_mpeg,
                    menu.timing.duration_ms.max(1),
                )
                .await
                {
                    Ok(path) => base_mpeg = path,
                    Err(error) => {
                        warn(
                            events,
                            "W_CACHE_WRITE_FAILED",
                            format!("No se pudo publicar el menú en caché: {error}"),
                        )
                        .await;
                    }
                }
            }
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
    let mut command = tokio::process::Command::from(dvdforge_toolchain::background_command(
        resolve_binary("ffmpeg"),
    ));
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
        TvStandard::Ntsc => 18,
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

    // Keep the backup tree consistent with the main one. Players and strict
    // validators may fall back to BACKUP/ when the primary files are damaged.
    for (directory, name, bytes) in [
        ("PLAYLIST", format!("{playlist:05}.mpls"), &patched_mpls),
        ("CLIPINF", format!("{clip:05}.clpi"), &patched_clpi),
    ] {
        let backup = bdmv_root.join("BACKUP").join(directory);
        if backup.is_dir() {
            write_bdmv_file(&backup.join(name), bytes)?;
        }
    }

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

fn menu_video_clip_duration(in_ms: u64, out_ms: Option<u64>) -> Option<f64> {
    out_ms
        .filter(|out| *out > in_ms)
        .map(|out| (out - in_ms) as f64 / 1000.0)
}

fn menu_overlay_filter(input: usize, overlay: &MenuClipOverlay, rate: &str) -> String {
    let loop_duration = format!("{:.3}", overlay.loop_duration_ms.max(1) as f64 / 1000.0);
    let loop_frames = ((overlay.loop_duration_ms.max(1) as f64
        * rate
            .split_once('/')
            .and_then(|(num, den)| {
                let num = num.parse::<f64>().ok()?;
                let den = den.parse::<f64>().ok()?;
                (den > 0.0).then_some(num / den)
            })
            .unwrap_or(25.0)
        / 1000.0)
        .ceil() as u64)
        .max(1);
    format!(
        "[{input}:v]trim=duration={loop_duration},setpts=PTS-STARTPTS,fps={rate},\
loop=loop=-1:size={loop_frames},setpts=N/FRAME_RATE/TB,\
scale={}:{}:force_original_aspect_ratio=decrease,\
pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1[ov{}]",
        overlay.width,
        overlay.height,
        overlay.width,
        overlay.height,
        input - 1
    )
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
    let mut command = Command::from(dvdforge_toolchain::background_command(resolve_binary(
        "ffmpeg",
    )));
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
            command.args(["-stream_loop", "-1"]);
            if *in_ms > 0 {
                command.args(["-ss", &format!("{:.3}", *in_ms as f64 / 1000.0)]);
            }
            push_file_input(&mut command, source);
            if let Some(duration) = menu_video_clip_duration(*in_ms, *out_ms) {
                command.args(["-t", &format!("{duration:.3}")]);
            }
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
            command.args(["-stream_loop", "-1"]);
            if overlay.source_start_ms > 0 {
                command.args([
                    "-ss",
                    &format!("{:.3}", overlay.source_start_ms as f64 / 1000.0),
                ]);
            }
            push_file_input(&mut command, &overlay.path);
            filter_graph.push(menu_overlay_filter(input, overlay, &rate));
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
    let mut command = Command::from(dvdforge_toolchain::background_command(resolve_binary(
        "ffmpeg",
    )));
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
    command.args(["-c:a", "ac3", "-b:a", "192k", "-ar", "48000", "-ac", "2"]);
    // Pass the OsStr path itself; never fall back to a CWD-relative name.
    command.arg(path);
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

async fn warn(events: &EventSink, code: &str, message: String) {
    let _ = events
        .send(JobEventKind::Warning {
            code: code.to_string(),
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

    fn test_directory(label: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "dvdforge-render-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("test directory");
        path
    }

    #[test]
    fn menu_clip_out_point_and_overlay_loop_are_applied_to_filters() {
        assert_eq!(menu_video_clip_duration(1_000, Some(3_500)), Some(2.5));
        assert_eq!(menu_video_clip_duration(1_000, Some(500)), None);
        let overlay = MenuClipOverlay {
            path: PathBuf::from("/tmp/overlay.mp4"),
            source_start_ms: 2_000,
            loop_duration_ms: 1_500,
            x: 10,
            y: 20,
            width: 320,
            height: 180,
        };
        let filter = menu_overlay_filter(2, &overlay, "30000/1001");
        assert!(filter.contains("trim=duration=1.500"));
        assert!(filter.contains("loop=loop=-1:size=45"));
        assert!(filter.contains("scale=320:180"));
        assert!(filter.contains("[ov1]"));
    }

    #[tokio::test]
    async fn source_fingerprint_hashes_the_whole_file_and_ignores_mtime_only() {
        let root = test_directory("fingerprint");
        let source = root.join("source.bin");
        let mut bytes = vec![1u8; 2 * 1024 * 1024 + 17];
        std::fs::write(&source, &bytes).expect("source");
        let first = source_fingerprint(&source).await.expect("fingerprint");
        assert!(first.starts_with("fp2:"));
        assert_eq!(
            first,
            source_fingerprint(&source)
                .await
                .expect("stable fingerprint")
        );

        // A change in the middle must invalidate the cache even when the file
        // size and mtime are unchanged.
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        std::fs::write(&source, &bytes).expect("changed source");
        let changed = source_fingerprint(&source)
            .await
            .expect("changed fingerprint");
        assert_ne!(first, changed);

        let handle = std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .expect("open source");
        handle
            .set_modified(std::time::SystemTime::now() + Duration::from_secs(2))
            .expect("mtime");
        drop(handle);
        assert_eq!(
            changed,
            source_fingerprint(&source)
                .await
                .expect("mtime-only fingerprint")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cached_artifact_copies_or_links_and_registers_the_blob() {
        let root = test_directory("cache");
        let store = Store::open(root.join("store.db")).expect("store");
        let source = root.join("source.mpg");
        std::fs::write(&source, b"rendered artifact").expect("source");
        let key = ArtifactKey {
            kind: CacheArtifactKind::Episode,
            source_fingerprint: "fp2:test".to_string(),
            inputs_hash: "inputs".to_string(),
            target_profile: "dvd:test:1".to_string(),
            target_bitrate: 4000,
            audio_params_hash: "audio".to_string(),
            encoder_rev: ENCODER_REV,
            toolchain_hash: "ffmpeg-test".to_string(),
        };
        let target = cached_artifact_path(&store, &key, "mpg", &source, 1234)
            .await
            .expect("cached artifact");
        assert_eq!(
            std::fs::read(&target).expect("cached bytes"),
            b"rendered artifact"
        );
        let cached = match store.lookup_artifact(&key) {
            CacheLookup::Hit(artifact) => artifact,
            other => panic!("expected cache hit, got {other:?}"),
        };
        assert_eq!(PathBuf::from(cached.artifact_path), target);
        assert_eq!(cached.duration_ms, 1234);
        let _ = std::fs::remove_dir_all(root);
    }

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
