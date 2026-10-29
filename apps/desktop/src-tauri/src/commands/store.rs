use dvdforge_domain::Project;
use dvdforge_store::{RecentProject, RecentProjectInput, Store};

#[tauri::command(async)]
#[specta::specta]
pub fn recent_projects_list(limit: Option<u32>) -> Result<Vec<RecentProject>, String> {
    let store = Store::open_default().map_err(|error| error.to_string())?;
    store
        .recent_projects(limit.unwrap_or(12))
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
#[specta::specta]
pub fn recent_project_forget(file_path: String) -> Result<(), String> {
    let store = Store::open_default().map_err(|error| error.to_string())?;
    store
        .forget_recent(&file_path)
        .map_err(|error| error.to_string())
}

#[tauri::command(async)]
#[specta::specta]
pub fn job_history_list(
    limit: Option<u32>,
) -> Result<Vec<dvdforge_store::JobHistoryEntry>, String> {
    let store = Store::open_default().map_err(|error| error.to_string())?;
    store
        .job_history(limit.unwrap_or(20))
        .map_err(|error| error.to_string())
}

pub fn record_project(path: &str, project: &Project) -> Result<(), String> {
    let store = Store::open_default().map_err(|error| error.to_string())?;
    store
        .record_recent(&RecentProjectInput {
            file_path: path.to_string(),
            name: project.name.clone(),
            disc_kind: project.disc.capacity.label().to_string(),
            thumbnail_path: None,
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}
