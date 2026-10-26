use dvdforge_burn::{
    platform_burner, BurnProgress, BurnReport, BurnRequest, CancellationToken, DeviceId,
    OpticalDevice,
};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::ipc::Channel;
use tokio::sync::mpsc;

static BURN_CANCEL: OnceLock<Mutex<Option<CancellationToken>>> = OnceLock::new();

fn cancel_slot() -> &'static Mutex<Option<CancellationToken>> {
    BURN_CANCEL.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
#[specta::specta]
pub fn burn_cancel() -> Result<(), String> {
    let token = cancel_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())?
        .take();
    match token {
        Some(token) => {
            token.cancel();
            Ok(())
        }
        None => Err("No hay una grabación activa.".to_string()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn burn_list_devices() -> Result<Vec<OpticalDevice>, String> {
    platform_burner()
        .list_devices()
        .await
        .map_err(|error| error.to_string())
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
    if cancel_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())?
        .is_some()
    {
        return Err("Ya hay una grabación en curso.".to_string());
    }
    let cancel = CancellationToken::new();
    *cancel_slot()
        .lock()
        .map_err(|_| "No se pudo acceder al estado de la grabación.".to_string())? =
        Some(cancel.clone());
    let (sender, mut receiver) = mpsc::channel(64);
    let burner = platform_burner();
    let label = PathBuf::from(&iso_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("DVD_FORGE")
        .to_string();
    let request = BurnRequest {
        iso_path: PathBuf::from(iso_path),
        device: DeviceId::new(device),
        speed_x: Some(speed),
        verify,
        eject,
        close_session: true,
        simulation: false,
        label,
    };
    let task = tokio::spawn(async move { burner.burn(request, sender, cancel).await });

    while let Some(progress) = receiver.recv().await {
        channel.send(progress).map_err(|error| error.to_string())?;
    }

    let _ = cancel_slot().lock().map(|mut slot| slot.take());
    match task.await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(error) => Err(format!("la tarea de grabación falló: {error}")),
    }
}
