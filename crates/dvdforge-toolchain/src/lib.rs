//! Detection and verification of the external tools used by DVD Forge.
//!
//! The detector intentionally treats usage banners as valid evidence for tools
//! such as `dvdauthor`, `spumux`, `growisofs` and `dvd+rw-mediainfo`, which do
//! not implement a conventional `--version` flag. tsMuxeR is also accepted when
//! it returns a non-zero exit code but prints its version banner.

use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ToolId {
    Ffmpeg,
    Ffprobe,
    Dvdauthor,
    Spumux,
    Xorriso,
    Growisofs,
    DvdRwMediaInfo,
    TsMuxer,
}

impl ToolId {
    pub const ALL: [Self; 8] = [
        Self::Ffmpeg,
        Self::Ffprobe,
        Self::Dvdauthor,
        Self::Spumux,
        Self::Xorriso,
        Self::Growisofs,
        Self::DvdRwMediaInfo,
        Self::TsMuxer,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Dvdauthor => "dvdauthor",
            Self::Spumux => "spumux",
            Self::Xorriso => "xorriso",
            Self::Growisofs => "growisofs",
            Self::DvdRwMediaInfo => "dvd+rw-mediainfo",
            Self::TsMuxer => "tsMuxeR",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub id: ToolId,
    pub name: String,
    pub required: bool,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub exit_code: Option<i32>,
    pub detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainReport {
    pub ok: bool,
    pub platform: String,
    pub tools: Vec<ToolStatus>,
    pub missing_required: Vec<ToolId>,
    pub warnings: Vec<String>,
}

impl ToolchainReport {
    pub fn from_statuses(tools: Vec<ToolStatus>) -> Self {
        let missing_required: Vec<ToolId> = tools
            .iter()
            .filter(|tool| tool.required && !tool.found)
            .map(|tool| tool.id)
            .collect();
        let mut warnings = Vec::new();
        for tool in &tools {
            if tool.found && tool.version.is_none() && tool.id != ToolId::DvdRwMediaInfo {
                warnings.push(format!(
                    "{} se encontró en {} pero no se pudo extraer una versión.",
                    tool.name,
                    tool.path.as_deref().unwrap_or("PATH")
                ));
            }
            if let Some(error) = &tool.error {
                warnings.push(format!("{}: {error}", tool.name));
            }
        }

        Self {
            ok: missing_required.is_empty(),
            platform: format!("{}-{}", env::consts::OS, env::consts::ARCH),
            tools,
            missing_required,
            warnings,
        }
    }

    pub fn tool(&self, id: ToolId) -> Option<&ToolStatus> {
        self.tools.iter().find(|tool| tool.id == id)
    }
}

struct ToolSpec {
    id: ToolId,
    name: &'static str,
    binary_names: &'static [&'static str],
    version_args: &'static [&'static str],
    accept_any_exit: bool,
    markers: &'static [&'static str],
}

const SPECS: &[ToolSpec] = &[
    ToolSpec {
        id: ToolId::Ffmpeg,
        name: "FFmpeg",
        binary_names: &["ffmpeg"],
        version_args: &["-version"],
        accept_any_exit: false,
        markers: &["ffmpeg version"],
    },
    ToolSpec {
        id: ToolId::Ffprobe,
        name: "FFprobe",
        binary_names: &["ffprobe"],
        version_args: &["-version"],
        accept_any_exit: false,
        markers: &["ffprobe version"],
    },
    ToolSpec {
        id: ToolId::Dvdauthor,
        name: "DVDAuthor",
        binary_names: &["dvdauthor"],
        version_args: &["--version"],
        accept_any_exit: true,
        markers: &["DVDAuthor", "dvdauthor"],
    },
    ToolSpec {
        id: ToolId::Spumux,
        name: "Spumux",
        binary_names: &["spumux"],
        version_args: &["--version"],
        accept_any_exit: true,
        markers: &["DVDAuthor::spumux", "spumux"],
    },
    ToolSpec {
        id: ToolId::Xorriso,
        name: "xorriso",
        binary_names: &["xorriso"],
        version_args: &["-version"],
        accept_any_exit: false,
        markers: &["xorriso"],
    },
    ToolSpec {
        id: ToolId::Growisofs,
        name: "growisofs",
        binary_names: &["growisofs"],
        version_args: &["-version"],
        accept_any_exit: false,
        markers: &["growisofs"],
    },
    ToolSpec {
        id: ToolId::DvdRwMediaInfo,
        name: "dvd+rw-mediainfo",
        binary_names: &["dvd+rw-mediainfo"],
        version_args: &[],
        accept_any_exit: true,
        markers: &["usage:", "dvd+rw-mediainfo"],
    },
    ToolSpec {
        id: ToolId::TsMuxer,
        name: "tsMuxeR",
        binary_names: &["tsMuxeR", "tsmuxer"],
        version_args: &["--version"],
        accept_any_exit: true,
        markers: &["tsMuxeR version"],
    },
];

/// Linux-only helpers are not required on macOS/Windows because those systems
/// use `drutil`/`hdiutil` and IMAPI2 respectively.
pub fn tool_supported_on(id: ToolId, platform: &str) -> bool {
    match id {
        ToolId::Growisofs | ToolId::DvdRwMediaInfo => platform == "linux",
        _ => matches!(platform, "linux" | "macos" | "windows"),
    }
}

/// Adds the platform executable suffix and keeps the bare name as a fallback.
/// Keeping both makes the resolver testable on Linux while preferring `.exe` on
/// Windows.
pub fn platform_binary_candidates(name: &str, platform: &str) -> Vec<String> {
    if platform == "windows" {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    }
}

/// Returns the ordered search path for a platform. `resource_root` points to
/// the Tauri `resources/` directory; PATH entries are appended at the end.
pub fn search_directories_for_platform(
    platform: &str,
    exe_dir: Option<&Path>,
    cwd: &Path,
    resource_root: Option<&Path>,
    path_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let mut push = |path: PathBuf| {
        if !directories
            .iter()
            .any(|existing: &PathBuf| existing == &path)
        {
            directories.push(path);
        }
    };

    if let Some(root) = resource_root {
        push(root.join("toolchains").join(platform));
    }
    if let Some(exe_dir) = exe_dir {
        push(exe_dir.join("resources").join("toolchains").join(platform));
        if platform == "macos" {
            push(
                exe_dir
                    .join("..")
                    .join("Resources")
                    .join("toolchains")
                    .join(platform),
            );
        }
    }
    push(cwd.join("resources").join("toolchains").join(platform));
    match platform {
        "macos" => {
            push(PathBuf::from("/opt/homebrew/bin"));
            push(PathBuf::from("/usr/local/bin"));
        }
        "linux" => push(PathBuf::from("/usr/local/bin")),
        _ => {}
    }
    for directory in path_dirs {
        push(directory.clone());
    }
    directories
}

/// Runtime search path used by `detect_toolchain`.
pub fn default_search_directories() -> Vec<PathBuf> {
    let platform = env::consts::OS;
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let resource_root = env::var_os("DVD_FORGE_RESOURCE_DIR").map(PathBuf::from);
    let path_dirs = env::var_os("PATH")
        .map(|value| env::split_paths(&value).collect::<Vec<PathBuf>>())
        .unwrap_or_default();
    search_directories_for_platform(
        platform,
        exe_dir.as_deref(),
        &cwd,
        resource_root.as_deref(),
        &path_dirs,
    )
}

/// Finds the first executable candidate in the already ordered directories.
pub fn find_binary_in_dirs(
    binary_names: &[&str],
    platform: &str,
    directories: &[PathBuf],
) -> Option<PathBuf> {
    for directory in directories {
        for name in binary_names {
            for file_name in platform_binary_candidates(name, platform) {
                let candidate = directory.join(file_name);
                if is_executable(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Detects every tool in `ToolId::ALL`, filtered by platform.
pub fn detect_toolchain() -> ToolchainReport {
    let platform = env::consts::OS;
    let directories = default_search_directories();
    let statuses = SPECS
        .iter()
        .filter(|spec| tool_supported_on(spec.id, platform))
        .map(|spec| detect_tool(spec, platform, &directories))
        .collect();
    ToolchainReport::from_statuses(statuses)
}

fn detect_tool(spec: &ToolSpec, platform: &str, directories: &[PathBuf]) -> ToolStatus {
    let Some(path) = find_binary_in_dirs(spec.binary_names, platform, directories) else {
        return ToolStatus {
            id: spec.id,
            name: spec.name.to_string(),
            required: true,
            found: false,
            path: None,
            version: None,
            exit_code: None,
            detail: None,
            error: Some(
                "no se encontró en recursos empaquetados, rutas del sistema ni PATH".to_string(),
            ),
        };
    };

    let output = Command::new(&path).args(spec.version_args).output();
    match output {
        Ok(output) => {
            let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
            if !output.stderr.is_empty() {
                if !combined.is_empty() && !combined.ends_with('\n') {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            let detail = first_nonempty_line(&combined).map(|line| truncate(line, 240));
            let version = extract_version(spec.id, &combined);
            let marker_found = spec.markers.iter().any(|marker| {
                combined
                    .to_ascii_lowercase()
                    .contains(&marker.to_ascii_lowercase())
            });
            let exit_code = output.status.code();
            let accepted =
                accepted_status(output.status.success(), spec.accept_any_exit, marker_found);

            ToolStatus {
                id: spec.id,
                name: spec.name.to_string(),
                required: true,
                found: accepted,
                path: Some(path.display().to_string()),
                version,
                exit_code,
                detail,
                error: if accepted {
                    None
                } else {
                    Some(format!(
                        "el comando terminó con código {:?} y no mostró un banner reconocible",
                        exit_code
                    ))
                },
            }
        }
        Err(error) => ToolStatus {
            id: spec.id,
            name: spec.name.to_string(),
            required: true,
            found: false,
            path: Some(path.display().to_string()),
            version: None,
            exit_code: None,
            detail: None,
            error: Some(format!("no se pudo ejecutar: {error}")),
        },
    }
}

fn accepted_status(exit_success: bool, accept_any_exit: bool, marker_found: bool) -> bool {
    exit_success || accept_any_exit || marker_found
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        let mut truncated: String = value.chars().take(max).collect();
        truncated.push('…');
        truncated
    }
}

fn extract_version(id: ToolId, output: &str) -> Option<String> {
    match id {
        ToolId::Ffmpeg => extract_after(output, "ffmpeg version "),
        ToolId::Ffprobe => extract_after(output, "ffprobe version "),
        ToolId::Dvdauthor | ToolId::Spumux => extract_after(output, "version "),
        ToolId::Xorriso => extract_second_token(output, "xorriso ")
            .or_else(|| extract_after(output, "xorriso version ")),
        ToolId::Growisofs => extract_after(output, "version "),
        ToolId::DvdRwMediaInfo => None,
        ToolId::TsMuxer => extract_after(output, "tsMuxeR version "),
    }
    .map(|version| {
        version
            .trim_matches(|c: char| c.is_whitespace() || c == ',' || c == '.')
            .to_string()
    })
    .filter(|version| !version.is_empty())
}

fn extract_after(output: &str, marker: &str) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    let marker_lower = marker.to_ascii_lowercase();
    let index = lower.find(&marker_lower)?;
    let rest = &output[index + marker.len()..];
    rest.split_whitespace().next().map(str::to_string)
}

fn extract_second_token(output: &str, prefix: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.trim_start().starts_with(prefix))
        .and_then(|line| line.split_whitespace().nth(1))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = env::temp_dir().join(format!(
            "dvdforge-toolchain-{label}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn make_executable(path: &Path) {
        std::fs::write(path, "stub").expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("permissions");
        }
    }

    #[test]
    fn platform_binary_names_prefer_exe_on_windows() {
        assert_eq!(
            platform_binary_candidates("ffmpeg", "windows"),
            vec!["ffmpeg.exe".to_string(), "ffmpeg".to_string()]
        );
        assert_eq!(
            platform_binary_candidates("ffmpeg", "linux"),
            vec!["ffmpeg".to_string()]
        );
        assert_eq!(
            platform_binary_candidates("tsMuxeR", "macos"),
            vec!["tsMuxeR".to_string()]
        );
    }

    #[test]
    fn search_directories_put_resources_before_path() {
        let cwd = Path::new("/work/app");
        let exe = Path::new("/work/app/bin");
        let resources = Path::new("/work/app/resources");
        let path_dirs = vec![PathBuf::from("/custom/bin")];
        let dirs =
            search_directories_for_platform("windows", Some(exe), cwd, Some(resources), &path_dirs);
        assert_eq!(dirs[0], resources.join("toolchains/windows"));
        assert!(dirs.contains(&exe.join("resources/toolchains/windows")));
        assert!(dirs.contains(&cwd.join("resources/toolchains/windows")));
        assert_eq!(dirs.last(), Some(&PathBuf::from("/custom/bin")));
    }

    #[test]
    fn macos_and_linux_include_system_prefixes() {
        let cwd = Path::new("/work");
        let mac = search_directories_for_platform("macos", None, cwd, None, &[]);
        assert!(mac.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert!(mac.contains(&PathBuf::from("/usr/local/bin")));
        let linux = search_directories_for_platform("linux", None, cwd, None, &[]);
        assert!(linux.contains(&PathBuf::from("/usr/local/bin")));
    }

    #[test]
    fn find_binary_prefers_windows_exe() {
        let dir = temp_dir("exe-priority");
        make_executable(&dir.join("ffmpeg.exe"));
        make_executable(&dir.join("ffmpeg"));
        let found = find_binary_in_dirs(&["ffmpeg"], "windows", std::slice::from_ref(&dir))
            .expect("windows binary");
        assert!(found.ends_with("ffmpeg.exe"));
        let found = find_binary_in_dirs(&["ffmpeg"], "linux", std::slice::from_ref(&dir))
            .expect("linux binary");
        assert!(found.ends_with("ffmpeg"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_binary_uses_resource_directory() {
        let dir = temp_dir("resource");
        let resources = dir.join("resources");
        let windows = resources.join("toolchains/windows");
        std::fs::create_dir_all(&windows).expect("windows resource dir");
        make_executable(&windows.join("xorriso.exe"));
        let dirs = search_directories_for_platform("windows", None, &dir, Some(&resources), &[]);
        let found = find_binary_in_dirs(&["xorriso"], "windows", &dirs).expect("resource binary");
        assert_eq!(found, windows.join("xorriso.exe"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn linux_helpers_are_not_required_on_other_platforms() {
        assert!(tool_supported_on(ToolId::Growisofs, "linux"));
        assert!(tool_supported_on(ToolId::DvdRwMediaInfo, "linux"));
        assert!(!tool_supported_on(ToolId::Growisofs, "macos"));
        assert!(!tool_supported_on(ToolId::DvdRwMediaInfo, "windows"));
        assert!(tool_supported_on(ToolId::TsMuxer, "windows"));
        assert!(tool_supported_on(ToolId::Ffmpeg, "macos"));
    }

    #[test]
    fn extracts_ffmpeg_version() {
        let output = "ffmpeg version n9.0.2 Copyright (c) 2000-2026 the FFmpeg developers\n";
        assert_eq!(
            extract_version(ToolId::Ffmpeg, output).as_deref(),
            Some("n9.0.2")
        );
    }

    #[test]
    fn extracts_dvdauthor_version_from_usage_banner() {
        let output = "DVDAuthor::dvdauthor, version 0.7.2.\nBuild options: ...\n";
        assert_eq!(
            extract_version(ToolId::Dvdauthor, output).as_deref(),
            Some("0.7.2")
        );
    }

    #[test]
    fn extracts_xorriso_version() {
        let output = "xorriso 1.5.8.pl02 : RockRidge filesystem manipulator, libburnia project.\n";
        assert_eq!(
            extract_version(ToolId::Xorriso, output).as_deref(),
            Some("1.5.8.pl02")
        );
    }

    #[test]
    fn extracts_tsmuxer_banner() {
        let output = "tsMuxeR version 2.7.0. github.com/justdan96/tsMuxer\n";
        assert_eq!(
            extract_version(ToolId::TsMuxer, output).as_deref(),
            Some("2.7.0")
        );
    }

    #[test]
    fn accepts_nonzero_exit_when_banner_is_present() {
        assert!(accepted_status(false, true, true));
        assert!(accepted_status(false, false, true));
        assert!(!accepted_status(false, false, false));
    }

    #[test]
    fn report_fails_when_required_tool_is_missing() {
        let report = ToolchainReport::from_statuses(vec![ToolStatus {
            id: ToolId::Ffmpeg,
            name: "FFmpeg".into(),
            required: true,
            found: false,
            path: None,
            version: None,
            exit_code: None,
            detail: None,
            error: Some("missing".into()),
        }]);
        assert!(!report.ok);
        assert_eq!(report.missing_required, vec![ToolId::Ffmpeg]);
    }
}
