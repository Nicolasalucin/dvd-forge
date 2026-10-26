use base64::Engine;
use dvdforge_desktop::commands::{
    burn_list_devices, menu_save_background, plan_calculate, project_new, project_open,
    project_save, read_image_data_url,
};
use dvdforge_domain::{AspectRatio, AuthoringMode, AvchdResolution, FrameRate, TvStandard};
use dvdforge_project::load_project;
use std::path::PathBuf;
use std::sync::Mutex;

/// Los comandos de proyecto guardan el directorio activo en un contexto
/// global; estos tests lo serializan para no interferir entre sí.
static CONTEXT_LOCK: Mutex<()> = Mutex::new(());

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/projects/minimal.json")
}

#[test]
fn project_new_creates_a_blank_dvd_shell() {
    let _context = CONTEXT_LOCK.lock().expect("context lock");
    let project = project_new(
        "Test DVD".into(),
        TvStandard::Pal,
        AspectRatio::SixteenNine,
        AuthoringMode::DvdVideo,
        None,
        None,
    )
    .expect("project_new");
    assert_eq!(project.name, "Test DVD");
    assert_eq!(project.disc.standard, TvStandard::Pal);
    assert_eq!(project.disc.aspect, AspectRatio::SixteenNine);
    assert_eq!(project.menus.len(), 1);
    assert_eq!(project.menus[0].canvas.width, 720);
    assert_eq!(project.menus[0].canvas.height, 576);
}

#[test]
fn project_save_and_open_round_trip() {
    let _context = CONTEXT_LOCK.lock().expect("context lock");
    let project = load_project(fixture_path()).expect("fixture");
    let path = std::env::temp_dir().join(format!(
        "dvdforge-desktop-roundtrip-{}.json",
        std::process::id()
    ));
    project_save(path.display().to_string(), project.clone()).expect("save");
    let loaded = project_open(path.display().to_string()).expect("open");
    assert_eq!(loaded.id, project.id);
    assert_eq!(loaded.menus.len(), project.menus.len());
    let _ = std::fs::remove_file(path);
}

#[test]
fn plan_calculate_uses_the_fixture_metadata() {
    let project = load_project(fixture_path()).expect("fixture");
    let plan = plan_calculate(project).expect("plan");
    assert!(plan.feasible);
    assert!(plan.fill_ppm > 980_000);
    assert!(plan.video_kbps > 4_000);
}

#[test]
#[ignore = "requiere una unidad óptica conectada en /dev/sr*"]
fn burn_list_devices_sees_the_physical_drive() {
    let devices = tauri::async_runtime::block_on(burn_list_devices())
        .expect("burn_list_devices debe responder");
    assert!(
        devices.iter().any(|device| {
            device.vendor.contains("HL-DT-ST")
                || device.model.contains("HL-DT-ST")
                || device.model.contains("DVDRAM")
        }),
        "se esperaba la unidad HL-DT-ST DVDRAM, encontradas: {devices:?}"
    );
}

#[tokio::test]
#[ignore = "render completo de ~1 minuto; requiere ffmpeg/dvdauthor/xorriso"]
async fn render_start_forwards_job_events_and_produces_iso() {
    use dvdforge_desktop::commands::{project_open, render_start};
    use dvdforge_jobs::{JobEvent, JobEventKind};
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use tauri::ipc::{Channel, InvokeResponseBody};

    let path = fixture_path();
    let project = project_open(path.display().to_string()).expect("fixture");
    let output =
        std::env::temp_dir().join(format!("dvdforge-command-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output);

    let events = Arc::new(Mutex::new(Vec::<JobEvent>::new()));
    let captured = Arc::clone(&events);
    let channel = Channel::<JobEvent>::new(move |body| {
        if let InvokeResponseBody::Json(json) = body {
            if let Ok(event) = serde_json::from_str::<JobEvent>(&json) {
                captured.lock().expect("events lock").push(event);
            }
        }
        Ok(())
    });

    let outcome = render_start(project, output.display().to_string(), channel)
        .await
        .expect("render_start");
    assert!(Path::new(&outcome.iso_path).is_file());
    assert!(outcome.iso_path.ends_with("disc.iso"));

    let events = events.lock().expect("events lock");
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, JobEventKind::StageStarted { .. })),
        "faltan eventos de etapa"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, JobEventKind::Progress { .. })),
        "faltan eventos de progreso"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, JobEventKind::Completed { .. })),
        "falta el evento de finalización"
    );
    assert!(
        events.iter().all(|event| event.job_id == outcome.job_id),
        "los eventos deben compartir el JobId devuelto"
    );

    let _ = std::fs::remove_dir_all(output);
}

#[test]
fn menu_save_background_writes_a_png_for_the_render() {
    const ONE_PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGPgFlIAAABpAD4z6GE4AAAAAElFTkSuQmCC";
    let project_id = format!("test-project-{}", std::process::id());
    let data_url = format!("data:image/png;base64,{ONE_PIXEL_PNG}");
    let path = menu_save_background(project_id.clone(), "menu-test".into(), data_url)
        .expect("menu_save_background");
    let bytes = std::fs::read(&path).expect("png escrito");
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir()
            .join("dvdforge-ui-layers")
            .join(project_id),
    );
}

#[test]
fn read_image_data_url_resolves_project_relative_assets() {
    const ONE_PIXEL_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGPgFlIAAABpAD4z6GE4AAAAAElFTkSuQmCC";
    let fixture = fixture_path();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let project_dir = std::env::temp_dir().join(format!(
        "dvdforge-read-image-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir_all(project_dir.join("assets")).expect("assets dir");
    std::fs::copy(&fixture, project_dir.join("minimal.json")).expect("fixture copy");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(ONE_PIXEL_PNG)
        .expect("png");
    std::fs::write(project_dir.join("assets/background.png"), bytes).expect("background.png");

    let _context = CONTEXT_LOCK.lock().expect("context lock");
    let project_path = project_dir.join("minimal.json");
    let _ = project_open(project_path.display().to_string()).expect("fixture");
    let url = read_image_data_url("assets/background.png".into()).expect("data url");
    assert!(url.starts_with("data:image/png;base64,"));
    let _ = std::fs::remove_dir_all(&project_dir);
}

#[test]
fn project_new_creates_an_avchd_shell_with_a_menu() {
    let _context = CONTEXT_LOCK.lock().expect("context lock");
    let project = project_new(
        "Test AVCHD".into(),
        TvStandard::Ntsc,
        AspectRatio::SixteenNine,
        AuthoringMode::Avchd,
        Some(AvchdResolution::R720p),
        Some(FrameRate::FPS_23_976),
    )
    .expect("project_new avchd");
    assert_eq!(project.disc.mode, AuthoringMode::Avchd);
    assert_eq!(project.menus.len(), 1);
    assert_eq!(project.assets.len(), 1);
    assert_eq!(project.root_menu, project.menus[0].id);
    assert!(matches!(
        project.disc.video,
        dvdforge_domain::VideoConfig::Avchd(_)
    ));
}
