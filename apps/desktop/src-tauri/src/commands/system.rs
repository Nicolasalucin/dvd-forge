#[tauri::command(async)]
#[specta::specta]
pub fn tools_doctor() -> kinodisc_toolchain::ToolchainReport {
    kinodisc_toolchain::detect_toolchain()
}

/// How a missing tool can be installed on this computer.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolInstallOption {
    pub tool: String,
    /// A pinned, checksum-verified download is available (one click).
    pub automatic: bool,
    pub version: Option<String>,
    pub size_bytes: Option<f64>,
    pub license: Option<String>,
    /// Package-manager command when there is no automatic download.
    pub hint: Option<String>,
}

#[tauri::command(async)]
#[specta::specta]
pub fn toolchain_install_options(tools: Vec<String>) -> Vec<ToolInstallOption> {
    use kinodisc_toolchain::provision::{download_for, install_hint};
    tools
        .into_iter()
        .map(|tool| {
            let download = download_for(&tool);
            ToolInstallOption {
                automatic: download.is_some(),
                version: download.map(|download| download.version.to_string()),
                size_bytes: download.map(|download| download.size as f64),
                license: download.map(|download| download.license.to_string()),
                hint: install_hint(&tool),
                tool,
            }
        })
        .collect()
}

/// Downloads, verifies (size + SHA-256) and installs a tool into the
/// per-user toolchain folder, which is searched before PATH.
#[tauri::command(async)]
#[specta::specta]
pub fn toolchain_install(
    tool: String,
) -> Result<kinodisc_toolchain::provision::ProvisionReport, String> {
    let download = kinodisc_toolchain::provision::download_for(&tool).ok_or_else(|| {
        format!("E_TOOLCHAIN_UNAVAILABLE: no hay descarga automática de {tool} para este sistema")
    })?;
    kinodisc_toolchain::provision::provision(download, |_, _| {}).map_err(|error| error.to_string())
}

/// Font families installed on this computer, sorted and de-duplicated, so
/// menu buttons can use any local font (the label raster runs in the webview
/// and therefore renders every installed family).
#[tauri::command(async)]
#[specta::specta]
pub fn system_fonts() -> Vec<String> {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = fontdb::Database::new();
            db.load_system_fonts();
            let mut families: Vec<String> = db
                .faces()
                .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
                .filter(|name| !name.trim().is_empty() && !name.starts_with('.'))
                .collect();
            families.sort_by_key(|name| name.to_lowercase());
            families.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            families
        })
        .clone()
}

#[tauri::command(async)]
#[specta::specta]
pub fn open_folder(path: String) -> Result<(), String> {
    let requested = std::path::PathBuf::from(&path);
    let target = resolve_folder_target(&requested);
    if !target.exists() {
        return Err(format!(
            "La carpeta todavía no existe: {}",
            target.display()
        ));
    }
    let command = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(command)
        .arg(&target)
        .spawn()
        .map_err(|error| error.to_string())?;
    // A dropped std `Child` is never reaped: each click left a zombie
    // xdg-open/open process. The launcher exits almost immediately.
    std::thread::Builder::new()
        .name("kinodisc-open-folder".to_string())
        .spawn(move || {
            let _ = child.wait();
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn resolve_folder_target(requested: &std::path::Path) -> std::path::PathBuf {
    if requested.is_file() {
        requested
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(std::path::Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| requested.to_path_buf())
    } else {
        requested.to_path_buf()
    }
}
