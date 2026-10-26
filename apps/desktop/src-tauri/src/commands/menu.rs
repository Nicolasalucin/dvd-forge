use base64::Engine;
use dvdforge_domain::{ButtonId, Menu, NavigationGraph};
use std::path::PathBuf;

/// Directorio temporal donde el editor deja la composición rasterizada de un
/// menú (fondo + textos + miniaturas) para que el render la use.
pub fn composited_background_dir(project_id: &str) -> PathBuf {
    std::env::temp_dir()
        .join("dvdforge-ui-layers")
        .join(sanitize_component(project_id))
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

#[tauri::command]
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
#[tauri::command]
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
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("PNG base64 inválido: {error}"))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("La composición del menú no es un PNG válido.".to_string());
    }
    let directory = composited_background_dir(project_id);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(file_name);
    std::fs::write(&path, bytes).map_err(|error| error.to_string())?;
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
}
