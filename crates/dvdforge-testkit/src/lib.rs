//! Test fixtures and golden-file helpers.

use std::path::{Path, PathBuf};

pub const CRATE_NAME: &str = "dvdforge-testkit";

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn fixture_path(relative: impl AsRef<Path>) -> PathBuf {
    workspace_root().join("fixtures").join(relative)
}

pub fn project_fixture(name: &str) -> PathBuf {
    fixture_path(Path::new("projects").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_contains_cargo_manifest() {
        assert!(workspace_root().join("Cargo.toml").is_file());
    }
}
