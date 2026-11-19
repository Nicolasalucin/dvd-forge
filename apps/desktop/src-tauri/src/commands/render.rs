use chrono::Utc;
use kinodisc_domain::{JobId, Project};
use kinodisc_jobs::{JobEvent, JobEventKind};
use kinodisc_render::{render_project_with_options, RenderError, RenderOptions, RenderOutcome};
use kinodisc_store::{NewJobHistory, Store};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

struct RenderJob {
    id: JobId,
    cancel: Arc<AtomicBool>,
    abort: Option<AbortHandle>,
}

static RENDER_JOB: OnceLock<Mutex<Option<RenderJob>>> = OnceLock::new();

fn job_slot() -> &'static Mutex<Option<RenderJob>> {
    RENDER_JOB.get_or_init(|| Mutex::new(None))
}

/// Clears the shared render slot and aborts the worker when the command future
/// is dropped (webview reload, channel error, panic...). The guard is disarmed
/// only after the worker has finished, so no orphaned FFmpeg/UDF task survives.
struct RenderGuard {
    id: JobId,
    cancel: Arc<AtomicBool>,
    abort: Option<AbortHandle>,
    armed: bool,
}

impl RenderGuard {
    fn new(id: JobId, cancel: Arc<AtomicBool>) -> Self {
        Self {
            id,
            cancel,
            abort: None,
            armed: true,
        }
    }

    fn set_abort(&mut self, abort: AbortHandle) {
        self.abort = Some(abort);
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RenderGuard {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.store(true, Ordering::SeqCst);
            if let Some(abort) = &self.abort {
                abort.abort();
            }
        }
        if let Ok(mut slot) = job_slot().lock() {
            if slot.as_ref().is_some_and(|job| job.id == self.id) {
                *slot = None;
            }
        }
    }
}

/// Cancela la generación activa. Los procesos FFmpeg/herramientas se crean con
/// `kill_on_drop` y la bandera compartida detiene la escritura UDF, por lo que
/// abortar la tarea no deja procesos ni un escritor bloqueado en segundo plano.
#[tauri::command]
#[specta::specta]
pub fn render_cancel() -> Result<(), String> {
    let abort = {
        let mut slot = job_slot()
            .lock()
            .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?;
        let job = slot
            .as_mut()
            .ok_or_else(|| "No hay una generación activa.".to_string())?;
        job.cancel.store(true, Ordering::SeqCst);
        job.abort.clone()
    };
    if let Some(abort) = abort {
        abort.abort();
    }
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderExport {
    pub folder: String,
    pub files: u32,
    pub bytes: f64,
}

/// Copies a finished disc image into `<output_dir>/<folder_name>` as a
/// VIDEO_TS/BDMV folder (byte-identical to the disc) and, when
/// `remove_iso` is set, deletes the image afterwards.
#[tauri::command]
#[specta::specta]
pub async fn render_export_folder(
    iso_path: String,
    output_dir: String,
    folder_name: String,
    remove_iso: bool,
) -> Result<FolderExport, String> {
    let iso = PathBuf::from(&iso_path);
    if !iso.is_file() || !iso_path.to_ascii_lowercase().ends_with(".iso") {
        return Err("No se encontró la imagen ISO generada.".to_string());
    }
    let name = kinodisc_verify::extract::safe_relative_path(folder_name.trim())
        .filter(|path| path.components().count() == 1)
        .unwrap_or_else(|| PathBuf::from("DISC"));
    let destination = PathBuf::from(&output_dir).join(name);
    let target = destination.clone();
    let report = tokio::task::spawn_blocking(move || {
        kinodisc_verify::extract::extract_iso(&iso, &target, &|| false)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| format!("No se pudo crear la carpeta del disco: {error}"))?;
    if remove_iso {
        let _ = std::fs::remove_file(&iso_path);
    }
    Ok(FolderExport {
        folder: destination.display().to_string(),
        files: report.files as u32,
        bytes: report.bytes as f64,
    })
}

/// Work files of one render (encoded episodes, menus, authoring XML, the
/// disc tree) live in `<output>/.kinodisc-work/<job>`, on the same drive as
/// the result, and are removed when the render ends. Only the finished image
/// is left in the folder the user chose.
const WORK_DIR: &str = ".kinodisc-work";

/// Work folders older than this are leftovers of a crash or a killed app.
const STALE_WORK: Duration = Duration::from_secs(6 * 3600);

fn remove_work_dir(output: &Path, work: &Path) {
    let _ = std::fs::remove_dir_all(work);
    // Only removed when empty (another job may still use it).
    let _ = std::fs::remove_dir(output.join(WORK_DIR));
}

fn remove_stale_work(output: &Path) {
    let Ok(entries) = std::fs::read_dir(output.join(WORK_DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_WORK);
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.parse::<JobId>().is_ok());
        if stale && ours && entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The finished image's file name: one component ending in `.iso`.
fn final_iso_name(name: Option<&str>) -> PathBuf {
    name.and_then(|name| kinodisc_verify::extract::safe_relative_path(name.trim()))
        .filter(|path| path.components().count() == 1)
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("iso"))
        })
        .unwrap_or_else(|| PathBuf::from("disc.iso"))
}

/// Moves the image out of the work folder, replacing an earlier image with
/// the same name (producing the project again).
fn publish_iso(work_iso: &Path, output: &Path, name: &Path) -> std::io::Result<PathBuf> {
    let target = output.join(name);
    if target.is_file() {
        std::fs::remove_file(&target)?;
    }
    std::fs::rename(work_iso, &target)?;
    Ok(target)
}

#[tauri::command]
#[specta::specta]
pub async fn render_start(
    project: Project,
    output_dir: String,
    hardware_encoding: kinodisc_toolchain::EncoderPreference,
    iso_name: Option<String>,
    channel: Channel<JobEvent>,
) -> Result<RenderOutcome, String> {
    let job_id = JobId::new();
    let project_dir = crate::context::project_dir()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "No se pudo determinar el directorio del proyecto.".to_string())?;
    let output_root = PathBuf::from(&output_dir);
    let iso_name = final_iso_name(iso_name.as_deref());
    remove_stale_work(&output_root);
    let work = output_root.join(WORK_DIR).join(job_id.to_string());
    let output = work.clone();
    let composited_backgrounds =
        crate::commands::menu::composited_background_dir(&project.id.to_string());
    let cancel = Arc::new(AtomicBool::new(false));
    // Check and insert under the same lock: two simultaneous render_start
    // calls can no longer both pass the is_some() check.
    {
        let mut slot = job_slot()
            .lock()
            .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?;
        if slot.is_some() {
            return Err("Ya hay una generación en curso.".to_string());
        }
        *slot = Some(RenderJob {
            id: job_id,
            cancel: Arc::clone(&cancel),
            abort: None,
        });
    }
    let mut guard = RenderGuard::new(job_id, Arc::clone(&cancel));
    let started = Instant::now();
    let job_name = project.name.clone();
    let disc_kind = project.disc.capacity.label().to_string();
    let cancel_flag = Arc::clone(&cancel);
    let (sender, mut receiver) = mpsc::channel(256);
    let task = tokio::spawn(async move {
        render_project_with_options(
            project,
            project_dir,
            output,
            job_id,
            RenderOptions {
                composited_backgrounds: Some(composited_backgrounds),
                cancel: Some(cancel),
                hardware_encoding,
                ..RenderOptions::default()
            },
            sender,
        )
        .await
    });
    let abort = task.abort_handle();
    guard.set_abort(abort.clone());
    {
        let mut slot = job_slot()
            .lock()
            .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?;
        if let Some(job) = slot.as_mut().filter(|job| job.id == job_id) {
            job.abort = Some(abort);
        }
    }

    let mut seq = 0u64;
    let mut last_progress = Instant::now() - PROGRESS_THROTTLE;
    while let Some(kind) = receiver.recv().await {
        let mut emit = true;
        if let JobEventKind::Progress { permille, .. } = &kind {
            if *permille < 1000 && last_progress.elapsed() < PROGRESS_THROTTLE {
                emit = false;
            } else {
                last_progress = Instant::now();
            }
        }
        if !emit {
            continue;
        }
        // Do not use `?` here: returning early must still run the Drop guard so
        // the worker is aborted and the shared slot is released.
        if let Err(error) = channel.send(JobEvent {
            job_id,
            seq,
            at: Utc::now().to_rfc3339(),
            kind,
        }) {
            return Err(error.to_string());
        }
        seq += 1;
    }

    let result = task.await;
    guard.disarm();
    drop(guard);
    let elapsed_seconds = started.elapsed().as_secs_f64();
    // Finished or not, the work folder goes; a finished image moves out first.
    let result = match result {
        Ok(Ok(mut outcome)) => {
            match publish_iso(Path::new(&outcome.iso_path), &output_root, &iso_name) {
                Ok(path) => {
                    outcome.iso_path = path.display().to_string();
                    Ok(Ok(outcome))
                }
                Err(error) => Ok(Err(RenderError::Io(error))),
            }
        }
        other => other,
    };
    remove_work_dir(&output_root, &work);
    match result {
        Ok(Ok(outcome)) => {
            record_job(
                job_id,
                &job_name,
                &disc_kind,
                Some(outcome.iso_path.clone()),
                "completed",
                "ISO generado",
                elapsed_seconds,
            );
            Ok(outcome)
        }
        Ok(Err(RenderError::Cancelled)) => {
            let message = "Generación cancelada por el usuario.".to_string();
            record_job(
                job_id,
                &job_name,
                &disc_kind,
                None,
                "cancelled",
                &message,
                elapsed_seconds,
            );
            let _ = channel.send(JobEvent {
                job_id,
                seq,
                at: Utc::now().to_rfc3339(),
                kind: JobEventKind::Cancelled { stage: None },
            });
            Err(message)
        }
        Ok(Err(error)) => {
            record_job(
                job_id,
                &job_name,
                &disc_kind,
                None,
                "failed",
                &error.to_string(),
                elapsed_seconds,
            );
            let _ = channel.send(JobEvent {
                job_id,
                seq,
                at: Utc::now().to_rfc3339(),
                kind: JobEventKind::Failed {
                    code: "E_RENDER".to_string(),
                    message: error.to_string(),
                    stage: None,
                    log_path: None,
                },
            });
            Err(error.to_string())
        }
        Err(error) if error.is_cancelled() || cancel_flag.load(Ordering::SeqCst) => {
            let message = "Generación cancelada por el usuario.".to_string();
            record_job(
                job_id,
                &job_name,
                &disc_kind,
                None,
                "cancelled",
                &message,
                elapsed_seconds,
            );
            let _ = channel.send(JobEvent {
                job_id,
                seq,
                at: Utc::now().to_rfc3339(),
                kind: JobEventKind::Cancelled { stage: None },
            });
            Err(message)
        }
        Err(error) => {
            let message = format!("la tarea de generación terminó inesperadamente: {error}");
            record_job(
                job_id,
                &job_name,
                &disc_kind,
                None,
                "failed",
                &message,
                elapsed_seconds,
            );
            let _ = channel.send(JobEvent {
                job_id,
                seq,
                at: Utc::now().to_rfc3339(),
                kind: JobEventKind::Failed {
                    code: "E_RENDER_JOIN".to_string(),
                    message: message.clone(),
                    stage: None,
                    log_path: None,
                },
            });
            Err(message)
        }
    }
}

fn record_job(
    job_id: JobId,
    project_name: &str,
    disc_kind: &str,
    iso_path: Option<String>,
    status: &str,
    summary: &str,
    elapsed_seconds: f64,
) {
    let iso_size_bytes = iso_path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let Ok(store) = Store::open_default() else {
        return;
    };
    let _ = store.record_job(&NewJobHistory {
        id: job_id.to_string(),
        project_name: project_name.to_string(),
        disc_kind: disc_kind.to_string(),
        iso_path,
        iso_size_bytes,
        duration_seconds: elapsed_seconds,
        status: status.to_string(),
        log_summary: summary.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_names_are_single_iso_file_names() {
        assert_eq!(
            final_iso_name(Some("Boda de Ana.iso")),
            PathBuf::from("Boda de Ana.iso")
        );
        assert_eq!(
            final_iso_name(Some("運動会 2026.iso")),
            PathBuf::from("運動会 2026.iso")
        );
        for bad in ["../x.iso", "a/b.iso", "x.exe", "", r"C:\x.iso"] {
            assert_eq!(
                final_iso_name(Some(bad)),
                PathBuf::from("disc.iso"),
                "{bad}"
            );
        }
        assert_eq!(final_iso_name(None), PathBuf::from("disc.iso"));
    }

    #[test]
    fn only_the_image_is_left_in_the_output_folder() {
        let output = std::env::temp_dir().join(format!("kd-publish-{}", JobId::new()));
        let job = JobId::new();
        let work = output.join(WORK_DIR).join(job.to_string());
        std::fs::create_dir_all(work.join("episodes")).unwrap();
        std::fs::write(work.join("episodes").join("e1.mpg"), b"x").unwrap();
        std::fs::write(work.join("disc.iso"), b"iso").unwrap();
        std::fs::write(output.join("Mi DVD.iso"), b"old").unwrap();
        let published =
            publish_iso(&work.join("disc.iso"), &output, Path::new("Mi DVD.iso")).unwrap();
        remove_work_dir(&output, &work);
        assert_eq!(std::fs::read(&published).unwrap(), b"iso");
        let left: Vec<_> = std::fs::read_dir(&output)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("Mi DVD.iso")]);
        std::fs::remove_dir_all(&output).unwrap();
    }

    #[test]
    fn render_start_payload_tolerates_empty_ids() {
        let raw = include_str!("../../../../../fixtures/projects/minimal.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).expect("fixture json");
        value["id"] = serde_json::json!("");
        value["rootMenu"] = serde_json::json!("");
        value["assets"][0]["id"] = serde_json::json!("");
        value["episodes"][0]["id"] = serde_json::json!("");
        value["episodes"][0]["video"] = serde_json::json!("");
        value["menus"][0]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][0]["id"] = serde_json::json!("");
        value["menus"][0]["buttons"][1]["navOverride"] =
            serde_json::json!({"up": "", "down": null, "left": "", "right": ""});
        let project: Project =
            serde_json::from_value(value).expect("render_start project with empty IDs");
        assert!(!project.id.as_uuid().is_nil());
        assert!(!project.menus[0].buttons[1].nav_override.up.is_some());
    }
}
