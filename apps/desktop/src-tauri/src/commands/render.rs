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

const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

static RENDER_TASK: OnceLock<Mutex<Option<tokio::task::AbortHandle>>> = OnceLock::new();
static RENDER_CANCEL_FLAG: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();
static RENDER_CANCELLED: AtomicBool = AtomicBool::new(false);

fn task_slot() -> &'static Mutex<Option<tokio::task::AbortHandle>> {
    RENDER_TASK.get_or_init(|| Mutex::new(None))
}

fn flag_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    RENDER_CANCEL_FLAG.get_or_init(|| Mutex::new(None))
}

/// Cancela la generación activa. Los procesos FFmpeg/herramientas se crean con
/// `kill_on_drop` y la bandera compartida detiene la escritura UDF, por lo que
/// abortar la tarea no deja procesos ni un escritor bloqueado en segundo plano.
#[tauri::command]
#[specta::specta]
pub fn render_cancel() -> Result<(), String> {
    if let Some(flag) = flag_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?
        .as_ref()
    {
        flag.store(true, Ordering::SeqCst);
    }
    let handle = task_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?
        .take();
    match handle {
        Some(handle) => {
            RENDER_CANCELLED.store(true, Ordering::SeqCst);
            handle.abort();
            Ok(())
        }
        None => Err("No hay una generación activa.".to_string()),
    }
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
    if task_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())?
        .is_some()
    {
        return Err("Ya hay una generación en curso.".to_string());
    }
    RENDER_CANCELLED.store(false, Ordering::SeqCst);
    let cancel = Arc::new(AtomicBool::new(false));
    *flag_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())? =
        Some(cancel.clone());
    let started = Instant::now();
    let job_name = project.name.clone();
    let disc_kind = project.disc.capacity.label().to_string();
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
    *task_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la generación.".to_string())? =
        Some(task.abort_handle());

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
        channel
            .send(JobEvent {
                job_id,
                seq,
                at: Utc::now().to_rfc3339(),
                kind,
            })
            .map_err(|error| error.to_string())?;
        seq += 1;
    }

    let _ = task_slot().lock().map(|mut slot| slot.take());
    let _ = flag_slot().lock().map(|mut slot| slot.take());
    let cancelled = RENDER_CANCELLED.swap(false, Ordering::SeqCst);
    let elapsed_seconds = started.elapsed().as_secs_f64();
    match task.await {
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
        Err(error) => {
            if cancelled {
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
            } else {
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
