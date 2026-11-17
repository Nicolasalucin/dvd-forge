use kinodisc_domain::Project;
use kinodisc_plan::BitratePlan;
use std::collections::HashMap;

#[tauri::command(async)]
#[specta::specta]
pub fn plan_calculate(project: Project) -> Result<BitratePlan, String> {
    kinodisc_plan::calculate_bitrate_plan(&project, &HashMap::new())
        .map_err(|error| error.to_string())
}

/// Splits the project into several discs (fill each disc, or one per
/// episode). Each part is rendered like a normal project.
#[tauri::command(async)]
#[specta::specta]
pub fn project_split(
    project: Project,
    strategy: kinodisc_render::split::SplitStrategy,
) -> Result<kinodisc_render::split::SplitResult, String> {
    kinodisc_render::split::split_project(&project, &HashMap::new(), strategy)
}
