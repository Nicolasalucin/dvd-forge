#![cfg_attr(mobile, tauri::mobile_entry_point)]

pub mod commands;
pub mod context;

use dvdforge_burn::{BurnProgress, BurnReport, OpticalDevice};
use dvdforge_domain::{DiscConfig, Episode, MediaProbe, Menu, NavigationGraph, Project};
use dvdforge_jobs::{JobEvent, JobEventKind};
use dvdforge_plan::BitratePlan;
use dvdforge_render::RenderOutcome;
use specta_typescript::Typescript;
use tauri_specta::{collect_commands, Builder};

use commands::{
    burn_cancel, burn_list_devices, burn_start, job_history_list, media_extract_frame,
    media_playback_support, media_prepare_preview, media_preview_url, media_probe_file,
    menu_navigation, menu_save_background, menu_save_labels, open_folder, plan_calculate,
    project_new, project_open, project_save, read_image_data_url, recent_project_forget,
    recent_projects_list, render_cancel, render_start, startup_project_path, tools_doctor,
    PlaybackSupport,
};

pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/api/bindings.ts");

pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            project_new,
            project_save,
            project_open,
            startup_project_path,
            media_probe_file,
            media_extract_frame,
            media_prepare_preview,
            media_playback_support,
            media_preview_url,
            read_image_data_url,
            menu_navigation,
            plan_calculate,
            burn_list_devices,
            render_start,
            render_cancel,
            burn_start,
            burn_cancel,
            open_folder,
            menu_save_background,
            menu_save_labels,
            recent_projects_list,
            recent_project_forget,
            job_history_list,
            tools_doctor
        ])
        .typ::<Project>()
        .typ::<DiscConfig>()
        .typ::<Menu>()
        .typ::<NavigationGraph>()
        .typ::<Episode>()
        .typ::<MediaProbe>()
        .typ::<PlaybackSupport>()
        .typ::<BitratePlan>()
        .typ::<OpticalDevice>()
        .typ::<JobEventKind>()
        .typ::<JobEvent>()
        .typ::<RenderOutcome>()
        .typ::<BurnProgress>()
        .typ::<BurnReport>()
        .typ::<dvdforge_store::RecentProject>()
        .typ::<dvdforge_store::JobHistoryEntry>()
        .typ::<dvdforge_toolchain::ToolchainReport>()
        .dangerously_cast_bigints_to_number()
}

pub fn export_bindings() -> Result<(), String> {
    let path = std::path::Path::new(BINDINGS_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let builder = specta_builder();
    builder
        .export(Typescript::default(), path)
        .map_err(|error| error.to_string())
}

pub fn run() {
    let builder = specta_builder();
    #[cfg(debug_assertions)]
    if let Err(error) = builder.export(Typescript::default(), BINDINGS_PATH) {
        eprintln!("No se pudieron exportar los bindings TypeScript: {error}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al ejecutar DVD Forge");
}
