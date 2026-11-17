use kinodisc_domain::Project;
use kinodisc_plan::BitratePlan;
use std::collections::HashMap;

#[tauri::command(async)]
#[specta::specta]
pub fn plan_calculate(project: Project) -> Result<BitratePlan, String> {
    kinodisc_plan::calculate_bitrate_plan(&project, &HashMap::new())
        .map_err(|error| error.to_string())
}
