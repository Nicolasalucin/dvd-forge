use kinodisc_burn::{
    platform_burner, BurnProgress, BurnReport, BurnRequest, CancellationToken, DeviceId,
    EraseReport, OpticalDevice,
};
use kinodisc_domain::JobId;
use std::sync::{Mutex, OnceLock};
use tauri::ipc::Channel;
use tokio::sync::mpsc;

const PROGRESS_THROTTLE: std::time::Duration = std::time::Duration::from_millis(200);

struct BurnJob {
    id: JobId,
    cancel: CancellationToken,
}

static BURN_JOB: OnceLock<Mutex<Option<BurnJob>>> = OnceLock::new();

fn job_slot() -> &'static Mutex<Option<BurnJob>> {
    BURN_JOB.get_or_init(|| Mutex::new(None))
}

/// Clears the slot only when the burn task that owns it has stopped. If a
/// newer job somehow replaced the slot, the id check leaves it untouched.
struct BurnSlotGuard {
    id: JobId,
}

impl Drop for BurnSlotGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = job_slot().lock() {
            if slot.as_ref().is_some_and(|job| job.id == self.id) {
                *slot = None;
            }
        }
    }
}

/// Cooperative cancellation for the command future. Dropping the Tauri command
/// (webview reload) must not abort the burn task abruptly: it asks the burner
/// to stop and lets it kill its child process cleanly.
struct BurnCancelOnDrop {
    cancel: CancellationToken,
    armed: bool,
}

impl Drop for BurnCancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancel.cancel();
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn burn_cancel() -> Result<(), String> {
    let cancel = job_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())?
        .as_ref()
        .map(|job| job.cancel.clone())
        .ok_or_else(|| "No hay una grabación activa.".to_string())?;
    cancel.cancel();
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn burn_list_devices() -> Result<Vec<OpticalDevice>, String> {
    platform_burner()
        .list_devices()
        .await
        .map_err(|error| error.to_string())
}

/// Erases the rewritable disc in `device` (DVD-RW, DVD+RW, BD-RE) so it can
/// be burned again. It takes the burn slot (no burn or erase can run at the
/// same time) and refuses discs that are not rewritable before touching the
/// drive. An erase cannot be cancelled once started.
#[tauri::command]
#[specta::specta]
pub async fn burn_erase(device: String, full: bool) -> Result<EraseReport, String> {
    let device = DeviceId::new(device);
    let job_id = JobId::new();
    {
        let mut slot = job_slot()
            .lock()
            .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())?;
        if slot.is_some() {
            return Err("Ya hay una grabación en curso.".to_string());
        }
        *slot = Some(BurnJob {
            id: job_id,
            cancel: CancellationToken::new(),
        });
    }
    let _slot_guard = BurnSlotGuard { id: job_id };
    let burner = platform_burner();
    let media = burner
        .probe_media(&device)
        .await
        .map_err(|error| error.to_string())?;
    if !media.rewritable {
        return Err(not_rewritable_message().to_string());
    }
    burner
        .erase(&device, full)
        .await
        .map_err(|error| error.to_string())
}

/// What a user reads when the disc in the drive cannot be erased.
fn not_rewritable_message() -> &'static str {
    "Este disco no es regrabable: solo se pueden borrar DVD-RW, DVD+RW y BD-RE. Un DVD-R o DVD+R grabado no se puede volver a usar."
}

#[tauri::command]
#[specta::specta]
pub async fn burn_start(
    iso_path: String,
    device: String,
    speed: u16,
    verify: bool,
    eject: bool,
    channel: Channel<BurnProgress>,
) -> Result<BurnReport, String> {
    let iso = std::fs::canonicalize(&iso_path)
        .map_err(|error| format!("El ISO no existe o no es accesible: {iso_path}: {error}"))?;
    if !iso.is_file() {
        return Err(format!(
            "El ISO no es un archivo regular: {}",
            iso.display()
        ));
    }
    if !iso
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("iso"))
    {
        return Err("El archivo seleccionado no tiene extensión .iso.".to_string());
    }
    let verification_path = iso.clone();
    let verification =
        tokio::task::spawn_blocking(move || kinodisc_verify::verify_iso(&verification_path, false))
            .await
            .map_err(|error| {
                format!("La verificación estructural terminó inesperadamente: {error}")
            })?
            .map_err(|error| format!("No se pudo verificar la ISO antes de grabar: {error}"))?;
    let failures: Vec<String> = verification
        .burn_blockers()
        .iter()
        .map(|check| format!("{}: {}", check.id, check.message))
        .collect();
    if !failures.is_empty() {
        return Err(format!(
            "La ISO no pasó la verificación estructural: {}",
            failures.join("; ")
        ));
    }
    let job_id = JobId::new();
    let cancel = CancellationToken::new();
    // Check and insert under the same lock so concurrent burn_start calls
    // cannot both start a burn.
    {
        let mut slot = job_slot()
            .lock()
            .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())?;
        if slot.is_some() {
            return Err("Ya hay una grabación en curso.".to_string());
        }
        *slot = Some(BurnJob {
            id: job_id,
            cancel: cancel.clone(),
        });
    }
    let mut command_guard = BurnCancelOnDrop {
        cancel: cancel.clone(),
        armed: true,
    };
    let (sender, mut receiver) = mpsc::channel(64);
    let burner = platform_burner();
    let label = iso
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("KINODISC")
        .to_string();
    let request = BurnRequest {
        iso_path: iso,
        device: DeviceId::new(device),
        speed_x: Some(speed),
        verify,
        eject,
        close_session: true,
        simulation: false,
        label,
    };
    let task_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        let _slot_guard = BurnSlotGuard { id: job_id };
        burner.burn(request, sender, task_cancel).await
    });

    let mut last_sent: Option<std::time::Instant> = None;
    while let Some(progress) = receiver.recv().await {
        // Coalesce per-MiB verify / per-IStream::Read updates like render_start.
        if progress.percent < 100.0
            && last_sent.is_some_and(|sent| sent.elapsed() < PROGRESS_THROTTLE)
        {
            continue;
        }
        last_sent = Some(std::time::Instant::now());
        // Returning early drops command_guard, which requests cooperative
        // cancellation; the worker keeps the slot until it has actually stopped.
        if let Err(error) = channel.send(progress) {
            return Err(error.to_string());
        }
    }

    let result = task.await;
    command_guard.armed = false;
    match result {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(error) => Err(format!("la tarea de grabación falló: {error}")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_data_disc_image_is_not_refused_but_a_broken_one_is() {
        let image = std::env::temp_dir().join(format!("kd-burn-data-{}", std::process::id()));
        let source = image.join("fotos");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("leeme.txt"), b"hola").unwrap();
        let iso = image.join("out/DATOS.iso");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(kinodisc_iso::build_data_iso(&source, &iso, "DATOS", None))
            .unwrap();
        let report = kinodisc_verify::verify_iso(&iso, false).unwrap();
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id == "PROFILE-UNKNOWN"),
            "the verifier still says it is not a video disc"
        );
        assert!(report.burn_blockers().is_empty(), "{:?}", report.checks);
        assert!(report.is_data_image());
        // A file that is not a disc image at all is still refused.
        let junk = image.join("out/basura.iso");
        std::fs::write(&junk, vec![0u8; 64 * 2048]).unwrap();
        let report = kinodisc_verify::verify_iso(&junk, false).unwrap();
        assert!(!report.burn_blockers().is_empty());
        assert!(!report.is_data_image());
        let _ = std::fs::remove_dir_all(&image);
    }
}
