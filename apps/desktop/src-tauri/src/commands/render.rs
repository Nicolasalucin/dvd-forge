use chrono::Utc;
use dvdforge_domain::{JobId, Project};
use dvdforge_jobs::{JobEvent, JobEventKind};
use dvdforge_render::{render_project_with_options, RenderError, RenderOptions, RenderOutcome};
use dvdforge_store::{NewJobHistory, Store};
use std::path::PathBuf;
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
    abort: Option<AbortHandle>,
    armed: bool,
}

impl RenderGuard {
    fn new(id: JobId) -> Self {
        Self {
            id,
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

#[tauri::command]
#[specta::specta]
pub async fn render_start(
    project: Project,
    output_dir: String,
    channel: Channel<JobEvent>,
) -> Result<RenderOutcome, String> {
    let job_id = JobId::new();
    let project_dir = crate::context::project_dir()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "No se pudo determinar el directorio del proyecto.".to_string())?;
    let output = PathBuf::from(&output_dir);
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
    let mut guard = RenderGuard::new(job_id);
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
