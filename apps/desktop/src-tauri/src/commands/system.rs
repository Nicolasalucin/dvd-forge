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

/// Tools with an automatic download that must be (re)installed: missing,
/// or installed by a pre-1.0 build without per-file hashes.
#[tauri::command(async)]
#[specta::specta]
pub fn toolchain_needs_install() -> Vec<String> {
    kinodisc_toolchain::provision::needs_install()
        .into_iter()
        .map(|download| download.id.to_string())
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
    // Folders only: never hand a file or URL to the system opener.
    if target.exists() && !target.is_dir() {
        return Err("Solo se pueden abrir carpetas.".to_string());
    }
    if !target.exists() {
        return Err(format!(
            "La carpeta todavía no existe: {}",
            target.display()
        ));
    }
    let mut child = std::process::Command::new(system_opener())
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

/// Shuts the computer down ("Shut down when done" after a long production;
/// the app shows a 60 s countdown with Cancel before calling this). Fixed
/// arguments and absolute system paths only: a planted `shutdown` next to
/// the app or on PATH is never run.
#[tauri::command(async)]
#[specta::specta]
pub fn system_shutdown() -> Result<(), String> {
    let (program, args): (std::path::PathBuf, &[&str]) = shutdown_command()?;
    if !program.is_file() {
        return Err(format!("No se encontró {}.", program.display()));
    }
    let status = kinodisc_toolchain::background_command(&program)
        .args(args)
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("El sistema rechazó el apagado (puede requerir permisos).".to_string())
    }
}

fn shutdown_command() -> Result<(std::path::PathBuf, &'static [&'static str]), String> {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").ok_or("SystemRoot no está definido.")?;
        let program = std::path::PathBuf::from(root)
            .join("System32")
            .join("shutdown.exe");
        Ok((program, &["/s", "/t", "0"]))
    }
    #[cfg(target_os = "macos")]
    {
        Ok((
            std::path::PathBuf::from("/usr/bin/osascript"),
            &["-e", "tell application \"System Events\" to shut down"],
        ))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // systemd hosts let the logged-in user power off through polkit.
        for program in ["/usr/bin/systemctl", "/bin/systemctl"] {
            if std::path::Path::new(program).is_file() {
                return Ok((std::path::PathBuf::from(program), &["poweroff"]));
            }
        }
        Err("Este sistema no permite apagar desde la aplicación.".to_string())
    }
}

/// The OS "open with the default app" launcher, by absolute path where the
/// system has a fixed one (a planted `explorer.exe` is never picked up).
fn system_opener() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let explorer = std::path::PathBuf::from(root).join("explorer.exe");
            if explorer.is_file() {
                return explorer;
            }
        }
        std::path::PathBuf::from("explorer")
    }
    #[cfg(target_os = "macos")]
    {
        std::path::PathBuf::from("/usr/bin/open")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::path::PathBuf::from("xdg-open")
    }
}

/// Newer Kinodisc release, if any (one HTTPS request to GitHub, no data
/// sent; the app calls it at most once a day and it can be turned off).
#[tauri::command(async)]
#[specta::specta]
pub fn update_check() -> Result<Option<kinodisc_toolchain::update::UpdateInfo>, String> {
    kinodisc_toolchain::update::check(env!("CARGO_PKG_VERSION"))
}

/// Opens an official release page in the browser. Any other URL is refused,
/// so the update notice can never become a way to open arbitrary links.
#[tauri::command(async)]
#[specta::specta]
pub fn open_release_page(url: String) -> Result<(), String> {
    let safe = url.starts_with(kinodisc_toolchain::update::RELEASES_PREFIX)
        && url.len() < 200
        && url
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "/:._-".contains(ch));
    if !safe {
        return Err("Solo se pueden abrir las páginas de versiones de Kinodisc.".to_string());
    }
    let mut child = std::process::Command::new(system_opener())
        .arg(&url)
        .spawn()
        .map_err(|error| error.to_string())?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
