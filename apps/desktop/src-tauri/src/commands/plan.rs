use dvdforge_domain::Project;
use dvdforge_plan::BitratePlan;
use std::collections::HashMap;

#[tauri::command]
#[specta::specta]
pub fn plan_calculate(project: Project) -> Result<BitratePlan, String> {
    dvdforge_plan::calculate_bitrate_plan(&project, &HashMap::new())
        .map_err(|error| error.to_string())
}
