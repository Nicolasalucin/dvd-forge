use dvdforge_verify::report::VerifyReport;

/// Verifies a generated ISO before it is presented to the user or burned.
#[tauri::command]
#[specta::specta]
pub async fn verify_iso(path: String, deep: bool) -> Result<VerifyReport, String> {
    let iso = std::path::PathBuf::from(&path);
    if !iso.is_file() {
        return Err(format!("El ISO no existe o no es accesible: {path}"));
    }
    tokio::task::spawn_blocking(move || dvdforge_verify::verify_iso(&iso, deep))
        .await
        .map_err(|error| format!("La verificación terminó inesperadamente: {error}"))?
        .map_err(|error| error.to_string())
}
