//! "Is there a newer Kinodisc?" One HTTPS GET to the GitHub releases API of
//! the official repository, through the system `curl` (System32 on Windows,
//! like the tool downloads). Nothing about the user or the computer is sent
//! (generic User-Agent, no parameters) and nothing is downloaded or
//! installed: the app only shows a link to the release page.

use serde::{Deserialize, Serialize};

const LATEST_URL: &str = "https://api.github.com/repos/Nicolasalucin/kinodisc/releases/latest";
/// Only links into the official release pages are ever shown.
pub const RELEASES_PREFIX: &str = "https://github.com/Nicolasalucin/kinodisc/releases/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The newer release, `None` when `current` is up to date.
pub fn check(current: &str) -> Result<Option<UpdateInfo>, String> {
    let output = crate::background_command(crate::provision::system_tool("curl"))
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--max-time",
            "10",
            "--max-filesize",
            "1000000",
            "--user-agent",
            "Kinodisc-update-check",
            "--header",
            "Accept: application/vnd.github+json",
            LATEST_URL,
        ])
        .output()
        .map_err(|error| format!("no se pudo ejecutar curl: {error}"))?;
    if !output.status.success() {
        return Err(
            "no se pudo consultar las versiones (sin conexión o límite de GitHub)".to_string(),
        );
    }
    newer_release(&output.stdout, current)
}

/// Parses the API answer and compares versions (pure, tested).
pub fn newer_release(json: &[u8], current: &str) -> Result<Option<UpdateInfo>, String> {
    let release: Release = serde_json::from_slice(json).map_err(|error| error.to_string())?;
    if release.draft || release.prerelease || !release.html_url.starts_with(RELEASES_PREFIX) {
        return Ok(None);
    }
    let (Some(latest), Some(installed)) =
        (parse_version(&release.tag_name), parse_version(current))
    else {
        return Ok(None);
    };
    Ok((latest > installed).then(|| UpdateInfo {
        version: release.tag_name.trim_start_matches('v').to_string(),
        url: release.html_url,
    }))
}

/// `v1.2.3` / `1.2.3` → (1, 2, 3); pre-release tags (`-beta`) are ignored.
fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    let value = value.trim().trim_start_matches('v');
    if value.contains('-') {
        return None;
    }
    let mut parts = value.split('.').map(|part| part.parse::<u32>().ok());
    Some((
        parts.next()??,
        parts.next().unwrap_or(Some(0))?,
        parts.next().unwrap_or(Some(0))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, url: &str) -> Vec<u8> {
        format!(r#"{{"tag_name":"{tag}","html_url":"{url}","draft":false,"prerelease":false}}"#)
            .into_bytes()
    }

    #[test]
    fn newer_versions_are_reported() {
        let json = release(
            "v1.1.0",
            "https://github.com/Nicolasalucin/kinodisc/releases/tag/v1.1.0",
        );
        let found = newer_release(&json, "1.0.1")
            .expect("parse")
            .expect("newer");
        assert_eq!(found.version, "1.1.0");
        assert!(newer_release(&json, "1.1.0").expect("parse").is_none());
        assert!(newer_release(&json, "1.2.0").expect("parse").is_none());
        // 1.10 is newer than 1.9 (numeric, not text, comparison).
        let ten = release(
            "v1.10.0",
            "https://github.com/Nicolasalucin/kinodisc/releases/tag/v1.10.0",
        );
        assert!(newer_release(&ten, "1.9.3").expect("parse").is_some());
    }

    #[test]
    fn foreign_links_and_prereleases_are_ignored() {
        let foreign = release("v9.0.0", "https://example.com/kinodisc");
        assert!(newer_release(&foreign, "1.0.0").expect("parse").is_none());
        let beta = release(
            "v2.0.0-beta",
            "https://github.com/Nicolasalucin/kinodisc/releases/tag/v2.0.0-beta",
        );
        assert!(newer_release(&beta, "1.0.0").expect("parse").is_none());
    }
}
