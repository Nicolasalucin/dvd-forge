//! Compatibility with installations made before the rename to Kinodisc
//! (DVD Forge ≤ 0.9).
//!
//! - Environment variables: `KINODISC_X` wins; `DVDFORGE_X` and `DVD_FORGE_X`
//!   are still honoured.
//! - Per-user data folder: `org.dvdforge.desktop` (recent projects, job
//!   history, render cache, downloaded tools) is moved once to
//!   `org.kinodisc.app`, and `dvdforge.db` becomes `kinodisc.db`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const APP_DIR: &str = "org.kinodisc.app";
pub const LEGACY_APP_DIR: &str = "org.dvdforge.desktop";

/// Reads `KINODISC_<suffix>`, falling back to the pre-1.0 names.
/// `name` must start with `KINODISC_`.
pub fn env_var_os(name: &str) -> Option<OsString> {
    if let Some(value) = std::env::var_os(name) {
        return Some(value);
    }
    let suffix = name.strip_prefix("KINODISC_")?;
    std::env::var_os(format!("DVDFORGE_{suffix}"))
        .or_else(|| std::env::var_os(format!("DVD_FORGE_{suffix}")))
}

/// String form of [`env_var_os`].
pub fn env_var(name: &str) -> Option<String> {
    env_var_os(name).and_then(|value| value.into_string().ok())
}

/// Folder that holds the per-user application folders (`<base>/<APP_DIR>`).
pub fn data_base() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    #[cfg(windows)]
    if let Some(value) = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(value));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".local").join("share"))
}

/// `<base>/org.kinodisc.app`, after migrating a pre-1.0 folder once.
pub fn app_data_dir() -> Option<PathBuf> {
    let base = data_base()?;
    migrate_in(&base);
    Some(base.join(APP_DIR))
}

/// Moves `<base>/org.dvdforge.desktop` to `<base>/org.kinodisc.app` when the
/// new folder does not exist yet. Same parent folder, so it is a cheap rename
/// even with a multi-GB render cache. Failures leave the old folder in place
/// (the app then starts fresh), never half-moved.
pub fn migrate_in(base: &Path) -> bool {
    static DONE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
    let Ok(mut done) = DONE.lock() else {
        return false;
    };
    if done.iter().any(|path| path == base) {
        return false;
    }
    done.push(base.to_path_buf());
    let old = base.join(LEGACY_APP_DIR);
    let new = base.join(APP_DIR);
    if !old.is_dir() || new.exists() {
        return false;
    }
    if std::fs::rename(&old, &new).is_err() {
        return false;
    }
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let legacy = new.join(format!("dvdforge.db{suffix}"));
        let current = new.join(format!("kinodisc.db{suffix}"));
        if legacy.is_file() && !current.exists() {
            let _ = std::fs::rename(legacy, current);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_folder_and_database_are_migrated_once() {
        let base = std::env::temp_dir().join(format!("kinodisc-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join(LEGACY_APP_DIR);
        std::fs::create_dir_all(old.join("toolchain")).unwrap();
        std::fs::write(old.join("dvdforge.db"), b"db").unwrap();
        std::fs::write(old.join("toolchain/ffmpeg.exe"), b"x").unwrap();
        assert!(migrate_in(&base));
        let new = base.join(APP_DIR);
        assert_eq!(std::fs::read(new.join("kinodisc.db")).unwrap(), b"db");
        assert!(new.join("toolchain/ffmpeg.exe").is_file());
        assert!(!old.exists());
        // Second call is a no-op.
        assert!(!migrate_in(&base));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn existing_new_folder_is_never_overwritten() {
        let base = std::env::temp_dir().join(format!("kinodisc-legacy2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(LEGACY_APP_DIR)).unwrap();
        std::fs::create_dir_all(base.join(APP_DIR)).unwrap();
        assert!(!migrate_in(&base));
        assert!(base.join(LEGACY_APP_DIR).is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }
}
