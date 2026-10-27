use base64::Engine;
use dvdforge_domain::{ButtonId, Menu, NavigationGraph};
use std::io::Write;
use std::path::PathBuf;

/// Maximum accepted base64 payload for a rasterized menu composition. 64 MB is
/// far above a 4K PNG but keeps a hostile IPC call from exhausting memory.
const MAX_COMPOSITION_BASE64: usize = 64 * 1024 * 1024;

/// Directorio de caché privada donde el editor deja la composición rasterizada
/// de un menú (fondo + textos + miniaturas) para que el render la use.
pub fn composited_background_dir(project_id: &str) -> PathBuf {
    crate::commands::media::cache_dir("menu-compositions").join(sanitize_component(project_id))
}

/// Recalcula la matriz de navegación real del menú en Rust para que el
/// simulador del editor no duplique la heurística de dvdauthor.
#[tauri::command]
#[specta::specta]
pub fn menu_navigation(menu: Menu, focus: Option<String>) -> NavigationGraph {
    let focus = focus
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| value.trim().parse::<ButtonId>().ok())
        .or_else(|| menu.buttons.first().map(|button| button.id));
    menu.navigation(focus)
}

#[tauri::command(async)]
#[specta::specta]
pub fn menu_save_background(
    project_id: String,
    menu_id: String,
    data_url: String,
) -> Result<String, String> {
    save_composition_png(
        &project_id,
        &format!("{}.png", sanitize_component(&menu_id)),
        &data_url,
    )
}

/// Guarda la capa transparente con textos, sombras e iconos que se superpone
/// a los vídeos de los botones en un menú animado.
#[tauri::command(async)]
#[specta::specta]
pub fn menu_save_labels(
    project_id: String,
    menu_id: String,
    data_url: String,
) -> Result<String, String> {
    save_composition_png(
        &project_id,
        &format!("labels-{}.png", sanitize_component(&menu_id)),
        &data_url,
    )
}

fn save_composition_png(
    project_id: &str,
    file_name: &str,
    data_url: &str,
) -> Result<String, String> {
    let (_, encoded) = data_url
        .split_once(',')
        .ok_or_else(|| "La imagen no es una data URL válida.".to_string())?;
    if encoded.len() > MAX_COMPOSITION_BASE64 {
        return Err("La composición del menú supera el límite de 64 MB.".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("PNG base64 inválido: {error}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("La composición del menú no es un PNG válido.".to_string());
    }
    let directory = composited_background_dir(project_id);
    crate::commands::media::create_private_dir(&directory)
        .map_err(|error| format!("No se pudo crear la caché de composiciones: {error}"))?;
    let path = directory.join(file_name);
    // Remove regular files and symlinks without following them, then create the
    // final file exclusively so a link cannot redirect the write.
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if metadata.is_dir() {
            return Err("La ruta de la composición apunta a una carpeta.".to_string());
        }
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("No se pudo crear la composición del menú: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("No se pudo escribir la composición del menú: {error}"))?;
    Ok(path.display().to_string())
}

fn sanitize_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_navigation_tolerates_empty_ids_from_ipc() {
        let raw = include_str!("../../../../../fixtures/projects/minimal.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
        value["menus"][0]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][0]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][1]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][1]["navOverride"] = serde_json::json!({
            "up": "",
            "down": null,
            "left": "",
            "right": ""
        });
        let menu_value = value["menus"][0].clone();
        let menu: Menu = serde_json::from_value(menu_value).expect("menu with empty IDs");
        let graph = menu_navigation(menu, None);
        assert!(!graph.nodes.is_empty());
    }

    #[test]
    fn composition_cache_rejects_invalid_data_urls_and_non_png_payloads() {
        let error = save_composition_png("project", "menu.png", "not-a-data-url")
            .expect_err("invalid data URL");
        assert!(error.contains("data URL"));

        let error =
            save_composition_png("project", "menu.png", "data:image/png;base64,bm90LWEtcG5n")
                .expect_err("non-PNG payload");
        assert!(error.contains("PNG"));
    }
}
