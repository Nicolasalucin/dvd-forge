use crate::context;
use chrono::Utc;
use kinodisc_domain::{
    AspectRatio, AssetId, AssetSource, AudioDefaults, AuthoringMode, AvchdResolution,
    AvchdVideoConfig, BudgetPolicy, BurnPreferences, CanvasSpec, DiscCapacity, DiscConfig,
    DvdVideoConfig, FrameRate, MediaAsset, MediaKind, MediaProbe, Menu, MenuBackground, MenuId,
    MenuPlacement, MenuRole, MenuTiming, PlayAllConfig, Project, ProjectId, SubpictureSpec,
    TvStandard, VideoConfig,
};

/// Creates a blank DVD-Video project shell. Media and episodes are added by the
/// editor once the user imports files.
/// Permite abrir un proyecto al arrancar para integración/automatización de UI.
/// Se activa con la variable de entorno `KINODISC_PROJECT`.
#[tauri::command]
#[specta::specta]
pub fn startup_project_path() -> Option<String> {
    kinodisc_toolchain::legacy::env_var("KINODISC_PROJECT")
        .ok_or(())
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
                scale_mode: kinodisc_domain::MenuScaleMode::Cover,
                dim_percent: 0,
                blur_px: 0,
                audio: None,
                motion_config: None,
                items: Vec::new(),
                buttons: Vec::new(),
                subpicture: SubpictureSpec::default(),
                timing: MenuTiming::default(),
                template: None,
                transition: None,
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
                    player_preset: Default::default(),
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
            let mut config = AvchdVideoConfig {
                resolution,
                frame_rate,
                ..AvchdVideoConfig::default()
            };
            config.level = config.effective_level();
            let errors: Vec<String> = config
                .validate()
                .into_iter()
                .filter(kinodisc_domain::Diagnostic::is_error)
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
                canvas: match resolution.dimensions() {
                    (width @ (1920 | 1280), height) => {
                        CanvasSpec::hd(width as u16, height as u16, effective_standard)
                    }
                    _ => CanvasSpec::new(effective_standard, effective_aspect),
                },
                background: MenuBackground::Static {
                    asset: background_id,
                },
                scale_mode: kinodisc_domain::MenuScaleMode::Cover,
                dim_percent: 0,
                blur_px: 0,
                audio: None,
                motion_config: None,
                items: Vec::new(),
                buttons: Vec::new(),
                subpicture: SubpictureSpec::default(),
                timing: MenuTiming::default(),
                template: None,
                transition: None,
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
                    player_preset: Default::default(),
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
    kinodisc_project::ensure_valid(&project)
        .map_err(|error| format!("No se puede guardar un proyecto inválido: {error}"))?;
    kinodisc_project::save_project(&project, &path).map_err(|error| error.to_string())?;
    context::set_from_file(std::path::Path::new(&path));
    if let Err(error) = super::store::record_project(&path, &project) {
        eprintln!("No se pudo actualizar la lista de proyectos recientes: {error}");
    }
    Ok(())
}

/// Packs the project and every file it uses into one `.kinodisc-bundle`.
/// Relative paths are resolved against the folder of the current project.
#[tauri::command(async)]
#[specta::specta]
pub fn bundle_export(
    project: Project,
    path: String,
) -> Result<kinodisc_project::bundle::BundleReport, String> {
    let project_dir = context::project_dir()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "No se pudo determinar la carpeta del proyecto.".to_string())?;
    kinodisc_project::bundle::export_bundle(&project, &project_dir, std::path::Path::new(&path))
        .map_err(|error| error.to_string())
}

/// Restores a bundle into `destination` (empty or new folder), verifying
/// every file, and opens the restored project.
#[tauri::command(async)]
#[specta::specta]
pub fn bundle_import(bundle: String, destination: String) -> Result<String, String> {
    let imported = kinodisc_project::bundle::import_bundle(
        std::path::Path::new(&bundle),
        std::path::Path::new(&destination),
    )
    .map_err(|error| error.to_string())?;
    Ok(imported.project_path)
}

#[tauri::command(async)]
#[specta::specta]
pub fn project_open(path: String) -> Result<Project, String> {
    if !std::path::Path::new(&path).is_file() {
        return Err("E_PROJECT_NOT_FOUND: No se encuentra el archivo del proyecto; puede que se haya movido, borrado o que esté en un disco desconectado.".to_string());
    }
    let project = kinodisc_project::load_and_validate(&path).map_err(|error| error.to_string())?;
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

/// Disc volume label from the project name: ASCII capitals, digits and "_"
/// (the ISO 9660/UDF safe set). Accented Latin letters keep their base
/// letter ("Serie Ñandú" → "SERIE_NANDU"; it used to be "SERIE__AND"), and
/// any other run of characters becomes one "_".
fn sanitize_volume_label(name: &str) -> String {
    let mut label = String::new();
    for character in name.chars() {
        let plain = ascii_letter(character);
        if plain.is_empty() {
            if !label.ends_with('_') {
                label.push('_');
            }
        } else {
            label.push_str(&plain.to_ascii_uppercase());
        }
    }
    let mut label = label.trim_matches('_').to_string();
    if label.is_empty() {
        label = "KINODISC".to_string();
    }
    label.truncate(32);
    label.trim_end_matches('_').to_string()
}

/// The ASCII spelling of a letter or digit (accents dropped), or "" for
/// anything else.
fn ascii_letter(character: char) -> String {
    if character.is_ascii_alphanumeric() {
        return character.to_string();
    }
    let plain = match character.to_lowercase().next().unwrap_or(character) {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'ā' | 'ą' => "a",
        'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'í' | 'ì' | 'î' | 'ï' | 'ī' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' | 'ō' | 'ő' => "o",
        'ú' | 'ù' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
        'ý' | 'ÿ' => "y",
        'ñ' | 'ń' | 'ň' => "n",
        'ç' | 'ć' | 'č' => "c",
        'ś' | 'š' | 'ş' => "s",
        'ź' | 'ż' | 'ž' => "z",
        'ł' => "l",
        'ř' => "r",
        'ď' => "d",
        'ť' => "t",
        'ğ' => "g",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        _ => "",
    };
    plain.to_string()
}

/// Crash-recovery copy of an unsaved project (never the user's own file).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutosaveEntry {
    pub project_id: String,
    pub name: String,
    /// The project's file when it had been saved before, if any.
    pub path: Option<String>,
    pub saved_at: String,
    pub project_json: String,
}

const AUTOSAVE_LIMIT: usize = 32 * 1024 * 1024;

fn autosave_dir() -> Result<std::path::PathBuf, String> {
    kinodisc_toolchain::legacy::app_data_dir()
        .map(|dir| dir.join("autosave"))
        .ok_or_else(|| "No se encontró la carpeta de datos del usuario.".to_string())
}

/// Only canonical UUIDs name autosave files (no paths, no traversal).
fn autosave_file(project_id: &str) -> Result<std::path::PathBuf, String> {
    let valid = project_id.len() == 36
        && project_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-');
    if !valid {
        return Err("Identificador de proyecto no válido.".to_string());
    }
    Ok(autosave_dir()?.join(format!("{}.json", project_id.to_ascii_lowercase())))
}

#[tauri::command(async)]
#[specta::specta]
pub fn autosave_write(entry: AutosaveEntry) -> Result<(), String> {
    if entry.project_json.len() > AUTOSAVE_LIMIT {
        return Err("El proyecto es demasiado grande para el autoguardado.".to_string());
    }
    // Must be a project, not arbitrary data.
    serde_json::from_str::<Project>(&entry.project_json)
        .map_err(|error| format!("Proyecto no válido: {error}"))?;
    let target = autosave_file(&entry.project_id)?;
    std::fs::create_dir_all(target.parent().unwrap_or(std::path::Path::new(".")))
        .map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&entry).map_err(|error| error.to_string())?;
    let partial = target.with_extension("part");
    std::fs::write(&partial, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, &target).map_err(|error| error.to_string())
}

#[tauri::command(async)]
#[specta::specta]
pub fn autosave_list() -> Vec<AutosaveEntry> {
    let Ok(dir) = autosave_dir() else {
        return Vec::new();
    };
    let mut entries: Vec<AutosaveEntry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|item| item.path().extension().is_some_and(|ext| ext == "json"))
        .filter(|item| {
            item.metadata()
                .is_ok_and(|meta| meta.len() as usize <= AUTOSAVE_LIMIT + 4096)
        })
        .filter_map(|item| std::fs::read(item.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<AutosaveEntry>(&bytes).ok())
        .collect();
    entries.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    entries
}

#[tauri::command(async)]
#[specta::specta]
pub fn autosave_discard(project_id: String) -> Result<(), String> {
    let target = autosave_file(&project_id)?;
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod autosave_tests {
    #[test]
    fn autosave_names_must_be_uuids() {
        assert!(super::autosave_file("../../evil").is_err());
        assert!(super::autosave_file("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/").is_err());
        assert!(super::autosave_file("C:/x").is_err());
        let ok = super::autosave_file("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE").unwrap();
        assert!(ok.ends_with("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.json"));
    }
}

#[cfg(test)]
mod volume_label_tests {
    use super::sanitize_volume_label;

    #[test]
    fn volume_labels_keep_base_letters() {
        assert_eq!(sanitize_volume_label("Serie Ñandú"), "SERIE_NANDU");
        assert_eq!(
            sanitize_volume_label("Cumpleaños de Sofía 🎂"),
            "CUMPLEANOS_DE_SOFIA"
        );
        assert_eq!(sanitize_volume_label("Straße 2026"), "STRASSE_2026");
        assert_eq!(sanitize_volume_label("運動会 2026"), "2026");
        assert_eq!(sanitize_volume_label("🎂"), "KINODISC");
        assert!(sanitize_volume_label(&"Ä".repeat(40)).len() <= 32);
    }
}
