#![cfg_attr(mobile, tauri::mobile_entry_point)]

pub mod commands;
pub mod context;

use kinodisc_burn::{BurnProgress, BurnReport, OpticalDevice};
use kinodisc_domain::{DiscConfig, Episode, MediaProbe, Menu, NavigationGraph, Project};
use kinodisc_jobs::{JobEvent, JobEventKind};
use kinodisc_plan::BitratePlan;
use kinodisc_render::RenderOutcome;
use specta_typescript::Typescript;
use tauri_specta::{collect_commands, Builder};

use commands::{
    autosave_discard, autosave_list, autosave_write, bundle_export, bundle_import, burn_cancel,
    burn_list_devices, burn_start, disc_import, disc_inspect, image_save_png, job_history_list,
    join_create, media_detect_crop, media_detect_scenes, media_extract_frame, media_filter_preview,
    media_missing, media_playback_support, media_prepare_preview, media_preview_url,
    media_probe_file, media_sidecar_subtitles, menu_apply_chapter_menus, menu_apply_template,
    menu_navigation, menu_save_background, menu_save_labels, menu_templates, open_folder,
    open_release_page, plan_calculate, project_new, project_open, project_save, project_split,
    read_image_data_url, recent_project_forget, recent_projects_list, render_cancel,
    render_export_folder, render_quality_sample, render_start, slideshow_create,
    startup_project_path, system_fonts, system_shutdown, toolchain_install,
    toolchain_install_options, toolchain_needs_install, tools_doctor, update_check, verify_iso,
    PlaybackSupport,
};

pub const BINDINGS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/api/bindings.ts");

pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            project_new,
            project_save,
            autosave_write,
            autosave_list,
            autosave_discard,
            project_open,
            bundle_export,
            bundle_import,
            startup_project_path,
            media_probe_file,
            media_missing,
            media_detect_scenes,
            media_detect_crop,
            media_filter_preview,
            media_extract_frame,
            slideshow_create,
            join_create,
            render_quality_sample,
            disc_inspect,
            disc_import,
            media_prepare_preview,
            media_playback_support,
            media_preview_url,
            media_sidecar_subtitles,
            read_image_data_url,
            menu_navigation,
            menu_templates,
            menu_apply_chapter_menus,
            menu_apply_template,
            plan_calculate,
            project_split,
            burn_list_devices,
            render_start,
            render_cancel,
            render_export_folder,
            burn_start,
            burn_cancel,
            open_folder,
            system_shutdown,
            update_check,
            open_release_page,
            menu_save_background,
            menu_save_labels,
            image_save_png,
            recent_projects_list,
            recent_project_forget,
            job_history_list,
            tools_doctor,
            system_fonts,
            toolchain_install_options,
            toolchain_needs_install,
            toolchain_install,
            verify_iso
        ])
        .typ::<Project>()
        .typ::<DiscConfig>()
        .typ::<Menu>()
        .typ::<NavigationGraph>()
        .typ::<kinodisc_menu::templates::MenuTemplateInfo>()
        .typ::<kinodisc_menu::templates::TemplateApplyResult>()
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
        .typ::<kinodisc_store::RecentProject>()
        .typ::<kinodisc_store::JobHistoryEntry>()
        .typ::<kinodisc_toolchain::ToolchainReport>()
        .typ::<kinodisc_verify::report::VerifyReport>()
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
            use tauri::Manager;
            // KINODISC_CACHE_DIR keeps test runs (QA walkthroughs) out of the
            // user's own cache folder; everyone else uses the app cache dir.
            if let Some(cache) = kinodisc_toolchain::legacy::env_var_os("KINODISC_CACHE_DIR")
                .filter(|value| !value.is_empty())
            {
                commands::media::set_cache_root(std::path::PathBuf::from(cache));
            } else if let Ok(cache) = app.path().app_cache_dir() {
                commands::media::set_cache_root(cache);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error al ejecutar Kinodisc");
}
