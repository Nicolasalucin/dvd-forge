use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static PROJECT_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<PathBuf>> {
    PROJECT_DIR.get_or_init(|| Mutex::new(None))
}

pub fn set_from_file(path: &Path) {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    if let Ok(mut current) = slot().lock() {
        *current = directory;
    }
}

pub fn set_current_directory() {
    if let Ok(mut current) = slot().lock() {
        *current = std::env::current_dir().ok();
    }
}

pub fn project_dir() -> Option<PathBuf> {
    slot().lock().ok().and_then(|current| current.clone())
}
