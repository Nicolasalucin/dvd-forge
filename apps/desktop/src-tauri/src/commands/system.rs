#[tauri::command(async)]
#[specta::specta]
pub fn tools_doctor() -> dvdforge_toolchain::ToolchainReport {
    dvdforge_toolchain::detect_toolchain()
}

#[tauri::command(async)]
#[specta::specta]
pub fn open_folder(path: String) -> Result<(), String> {
    let requested = std::path::PathBuf::from(&path);
    let target = if requested.is_file() {
        requested
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or(requested)
    } else {
        requested
    };
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
    std::process::Command::new(command)
        .arg(&target)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
