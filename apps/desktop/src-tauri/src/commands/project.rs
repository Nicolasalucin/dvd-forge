use crate::context;
use chrono::Utc;
use dvdforge_domain::{
    AspectRatio, AssetId, AssetSource, AudioDefaults, AuthoringMode, AvchdResolution,
    AvchdVideoConfig, BudgetPolicy, BurnPreferences, CanvasSpec, DiscCapacity, DiscConfig,
    DvdVideoConfig, FrameRate, MediaAsset, MediaKind, MediaProbe, Menu, MenuBackground, MenuId,
    MenuPlacement, MenuRole, MenuTiming, PlayAllConfig, Project, ProjectId, SubpictureSpec,
    TvStandard, VideoConfig,
};

/// Creates a blank DVD-Video project shell. Media and episodes are added by the
/// editor once the user imports files.
/// Permite abrir un proyecto al arrancar para integración/automatización de UI.
/// Se activa con la variable de entorno `DVD_FORGE_PROJECT`.
#[tauri::command]
#[specta::specta]
pub fn startup_project_path() -> Option<String> {
    std::env::var("DVD_FORGE_PROJECT")
        .ok()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .filter(|path| std::path::Path::new(path).is_file())
}

#[tauri::command]
#[specta::specta]
pub fn project_new(
    name: String,
    standard: TvStandard,
    aspect: AspectRatio,
    mode: AuthoringMode,
    avchd_resolution: Option<AvchdResolution>,
    avchd_frame_rate: Option<FrameRate>,
) -> Result<Project, String> {
    context::set_current_directory();
    let now = Utc::now().to_rfc3339();
    let project_id = ProjectId::new();
    let volume_label = sanitize_volume_label(&name);

    let (disc, assets, menus, root_menu, budget) = match mode {
        AuthoringMode::DvdVideo => {
            let menu_id = MenuId::new();
            let background_id = AssetId::new();
            let canvas = CanvasSpec::new(standard, aspect);
            let background = MediaAsset {
                id: background_id,
                display_name: "background.png".to_string(),
                source: AssetSource::Project,
                relative_path: Some("assets/background.png".to_string()),
                sha256: "0".repeat(64),
                bytes: 0,
                kind: MediaKind::Image,
                probe: MediaProbe {
                    format_name: "png".to_string(),
                    duration_ms: None,
                    size_bytes: None,
                    bitrate_bps: None,
                    video: None,
                    audio: Vec::new(),
                    subtitle_streams: 0,
                    subtitle_tracks: Vec::new(),
                    chapters: Vec::new(),
                },
                proxy: None,
                imported_at: now.clone(),
            };
            let menu = Menu {
                id: menu_id,
                name: "Menú principal".to_string(),
                role: MenuRole::Root,
                placement: MenuPlacement::Vmgm,
                canvas,
                background: MenuBackground::Static {
                    asset: background_id,
                },
                scale_mode: dvdforge_domain::MenuScaleMode::Cover,
                dim_percent: 0,
                blur_px: 0,
                audio: None,
                motion_config: None,
                items: Vec::new(),
                buttons: Vec::new(),
                subpicture: SubpictureSpec::default(),
                timing: MenuTiming::default(),
            };
            (
                DiscConfig {
                    mode: AuthoringMode::DvdVideo,
                    standard,
                    aspect,
                    capacity: DiscCapacity::Dvd5,
                    volume_label: volume_label.clone(),
                    region_mask: 0,
                    layer_break_mode: Default::default(),
                    video: VideoConfig::Dvd(DvdVideoConfig::default()),
                    audio: AudioDefaults::default(),
                },
                vec![background],
                vec![menu],
                menu_id,
                BudgetPolicy::default(),
            )
        }
        AuthoringMode::Avchd => {
            let resolution = avchd_resolution.unwrap_or(AvchdResolution::R720p);
            let frame_rate = avchd_frame_rate.unwrap_or_else(|| default_avchd_fps(resolution));
            let config = AvchdVideoConfig {
                resolution,
                frame_rate,
                ..AvchdVideoConfig::default()
            };
            let errors: Vec<String> = config
                .validate()
                .into_iter()
                .filter(dvdforge_domain::Diagnostic::is_error)
                .map(|diagnostic| diagnostic.to_string())
                .collect();
            if !errors.is_empty() {
                return Err(errors.join("; "));
            }
            let effective_standard = avchd_standard(resolution, frame_rate);
            // AVCHD se entrega en 16:9 incluso en SD; el SAR se ajusta en el encoder.
            let effective_aspect = AspectRatio::SixteenNine;
            let menu_id = MenuId::new();
            let background_id = AssetId::new();
            let background = MediaAsset {
                id: background_id,
                display_name: "background.png".to_string(),
                source: AssetSource::Project,
                relative_path: Some("assets/background.png".to_string()),
                sha256: "0".repeat(64),
                bytes: 0,
                kind: MediaKind::Image,
                probe: MediaProbe {
                    format_name: "png".to_string(),
                    duration_ms: None,
                    size_bytes: None,
                    bitrate_bps: None,
                    video: None,
                    audio: Vec::new(),
                    subtitle_streams: 0,
                    subtitle_tracks: Vec::new(),
                    chapters: Vec::new(),
                },
                proxy: None,
                imported_at: now.clone(),
            };
            let menu = Menu {
                id: menu_id,
                name: "Menú principal".to_string(),
                role: MenuRole::Root,
                placement: MenuPlacement::Vmgm,
                canvas: CanvasSpec::new(effective_standard, effective_aspect),
                background: MenuBackground::Static {
                    asset: background_id,
                },
                scale_mode: dvdforge_domain::MenuScaleMode::Cover,
                dim_percent: 0,
                blur_px: 0,
                audio: None,
                motion_config: None,
                items: Vec::new(),
                buttons: Vec::new(),
                subpicture: SubpictureSpec::default(),
                timing: MenuTiming::default(),
            };
            (
                DiscConfig {
                    mode: AuthoringMode::Avchd,
                    standard: effective_standard,
                    aspect: effective_aspect,
                    capacity: DiscCapacity::Dvd5,
                    volume_label,
                    region_mask: 0,
                    layer_break_mode: Default::default(),
                    video: VideoConfig::Avchd(config),
                    audio: AudioDefaults::default(),
                },
                vec![background],
                vec![menu],
                menu_id,
                BudgetPolicy {
                    header_margin_ppm: 30_000,
                    mux_overhead_ppm: 50_000,
                    min_video_kbps: 1_000,
                    max_video_kbps: 24_000,
                    ..BudgetPolicy::default()
                },
            )
        }
    };

    Ok(Project {
        schema_version: 1,
        id: project_id,
        name,
        created_at: now.clone(),
        modified_at: now,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        disc,
        assets,
        first_play: None,
        menus,
        root_menu,
        episodes: Vec::new(),
        play_all: PlayAllConfig::default(),
        budget,
        burn: BurnPreferences::default(),
    })
}

// Sync commands run on the main (UI) thread; saving now fsyncs and also
// writes SQLite, so run it on the async runtime instead.
#[tauri::command(async)]
#[specta::specta]
pub fn project_save(path: String, project: Project) -> Result<(), String> {
    dvdforge_project::ensure_valid(&project)
        .map_err(|error| format!("No se puede guardar un proyecto inválido: {error}"))?;
    dvdforge_project::save_project(&project, &path).map_err(|error| error.to_string())?;
    context::set_from_file(std::path::Path::new(&path));
    if let Err(error) = super::store::record_project(&path, &project) {
        eprintln!("No se pudo actualizar la lista de proyectos recientes: {error}");
    }
    Ok(())
}

#[tauri::command(async)]
#[specta::specta]
pub fn project_open(path: String) -> Result<Project, String> {
    let project = dvdforge_project::load_and_validate(&path).map_err(|error| error.to_string())?;
    context::set_from_file(std::path::Path::new(&path));
    if let Err(error) = super::store::record_project(&path, &project) {
        eprintln!("No se pudo actualizar la lista de proyectos recientes: {error}");
    }
    Ok(project)
}

fn default_avchd_fps(resolution: AvchdResolution) -> FrameRate {
    match resolution {
        AvchdResolution::R1080p | AvchdResolution::R720p => FrameRate::FPS_23_976,
        AvchdResolution::R1080i | AvchdResolution::Sd480i => FrameRate::FPS_29_97,
        AvchdResolution::Sd576i => FrameRate::PAL,
    }
}

fn avchd_standard(resolution: AvchdResolution, frame_rate: FrameRate) -> TvStandard {
    match resolution {
        AvchdResolution::Sd480i => TvStandard::Ntsc,
        AvchdResolution::Sd576i => TvStandard::Pal,
        _ if frame_rate == FrameRate::PAL || frame_rate == FrameRate::FPS_50 => TvStandard::Pal,
        _ => TvStandard::Ntsc,
    }
}

fn sanitize_volume_label(name: &str) -> String {
    let mut label: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    label = label.trim_matches('_').to_string();
    if label.is_empty() {
        label = "DVD_FORGE".to_string();
    }
    label.truncate(32);
    label
}
