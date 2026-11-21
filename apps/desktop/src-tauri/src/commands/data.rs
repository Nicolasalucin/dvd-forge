//! Data discs: a plain image of any folder (photos, documents, backups),
//! burned later like any other ISO. Nothing in the folder is run or changed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Entries counted before giving up (a whole drive picked by mistake).
const MAX_ENTRIES: u32 = 500_000;

static CANCEL: OnceLock<Mutex<Option<Arc<AtomicBool>>>> = OnceLock::new();

fn cancel_slot() -> &'static Mutex<Option<Arc<AtomicBool>>> {
    CANCEL.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderSize {
    pub files: u32,
    pub bytes: f64,
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DataIsoOutcome {
    pub iso_path: String,
    pub bytes: f64,
}

/// Files and bytes under `folder` (symbolic links are not followed), to say
/// which disc it fits before the image is made.
fn measure(folder: &Path) -> Result<FolderSize, String> {
    let mut files = 0u32;
    let mut bytes = 0u64;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory)
            .map_err(|error| format!("No se pudo leer {}: {error}", directory.display()))?;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else if let Ok(metadata) = entry.metadata() {
                files += 1;
                bytes = bytes.saturating_add(metadata.len());
            }
            if files > MAX_ENTRIES {
                return Err("La carpeta tiene demasiados archivos para un disco.".to_string());
            }
        }
    }
    Ok(FolderSize {
        files,
        bytes: bytes as f64,
    })
}

#[tauri::command(async)]
#[specta::specta]
pub fn data_folder_size(folder: String) -> Result<FolderSize, String> {
    let folder = PathBuf::from(folder);
    if !folder.is_dir() {
        return Err("Elige una carpeta.".to_string());
    }
    measure(&folder)
}

/// The output folder already has an image with this label.
const EXISTS_MESSAGE: &str =
    "Ya existe una imagen con esa etiqueta en la carpeta de salida. Cambia la etiqueta o elige otra carpeta.";

/// What the user reads when the data image could not be made. `cancelled`
/// is the state of the cancel flag when the build ended.
fn data_iso_error(error: &kinodisc_iso::IsoError, cancelled: bool) -> String {
    if cancelled {
        return "Disco de datos cancelado.".to_string();
    }
    match error {
        kinodisc_iso::IsoError::OutputInsideSource => {
            "Elige otra carpeta de salida: la imagen no puede guardarse dentro de la carpeta que se graba."
                .to_string()
        }
        other => format!("No se pudo crear el disco de datos: {other}"),
    }
}

/// Writes `<output_dir>/<LABEL>.iso` with the contents of `folder`. The label
/// is made disc-safe (ASCII capitals, digits and `_`).
#[tauri::command]
#[specta::specta]
pub async fn data_iso_create(
    folder: String,
    output_dir: String,
    label: String,
) -> Result<DataIsoOutcome, String> {
    let source = PathBuf::from(&folder);
    if !source.is_dir() {
        return Err("Elige la carpeta que quieres grabar.".to_string());
    }
    let output = PathBuf::from(&output_dir);
    if !output.is_absolute() {
        return Err("Elige la carpeta donde guardar la imagen.".to_string());
    }
    let fallback = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("DATOS");
    let label = super::project::sanitize_volume_label(if label.trim().is_empty() {
        fallback
    } else {
        &label
    });
    let iso_path = output.join(format!("{label}.iso"));
    // Never replace a file that is already there (QA 1.2.1: an existing
    // "<LABEL>.iso" was overwritten without a word).
    if iso_path.exists() {
        return Err(EXISTS_MESSAGE.to_string());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = cancel_slot()
            .lock()
            .map_err(|_| "No se pudo iniciar el disco de datos.".to_string())?;
        if slot.is_some() {
            return Err("Ya se está creando un disco de datos.".to_string());
        }
        *slot = Some(Arc::clone(&cancel));
    }
    let result =
        kinodisc_iso::build_data_iso(&source, &iso_path, &label, Some(Arc::clone(&cancel))).await;
    if let Ok(mut slot) = cancel_slot().lock() {
        *slot = None;
    }
    let report = result.map_err(|error| data_iso_error(&error, cancel.load(Ordering::SeqCst)))?;
    Ok(DataIsoOutcome {
        iso_path: report.iso_path.display().to_string(),
        bytes: report.size_bytes as f64,
    })
}

#[tauri::command]
#[specta::specta]
pub fn data_iso_cancel() -> Result<(), String> {
    if let Ok(slot) = cancel_slot().lock() {
        if let Some(flag) = slot.as_ref() {
            flag.store(true, Ordering::SeqCst);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_size_counts_files_and_skips_nothing_real() {
        let root = std::env::temp_dir().join(format!("kd-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("uno.txt"), b"12345").unwrap();
        std::fs::write(root.join("a/b/dos.bin"), vec![0u8; 100]).unwrap();
        let size = measure(&root).unwrap();
        assert_eq!((size.files, size.bytes as u64), (2, 105));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn data_iso_errors_read_as_whole_sentences() {
        use kinodisc_iso::IsoError;
        assert_eq!(
            data_iso_error(&IsoError::OutputInsideSource, false),
            "Elige otra carpeta de salida: la imagen no puede guardarse dentro de la carpeta que se graba."
        );
        // A cancelled run says so, whatever the builder reported.
        for error in [
            IsoError::BdmvAuthoring("creación del disco de datos cancelada".to_string()),
            IsoError::OutputInsideSource,
        ] {
            assert_eq!(data_iso_error(&error, true), "Disco de datos cancelado.");
        }
        assert_eq!(
            data_iso_error(&IsoError::EmptyIso, false),
            "No se pudo crear el disco de datos: el ISO está vacío"
        );
    }
}
