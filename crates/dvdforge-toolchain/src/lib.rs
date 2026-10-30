//! Detection and verification of the external tools used by DVD Forge.
//!
//! The detector intentionally treats usage banners as valid evidence for tools
//! such as `dvdauthor`, `spumux`, `growisofs` and `dvd+rw-mediainfo`, which do
//! not implement a conventional `--version` flag. tsMuxeR is also accepted when
//! it returns a non-zero exit code but prints its version banner.

use serde::{Deserialize, Serialize};
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Protocols allowed for local media inputs passed to FFmpeg/FFprobe. Keeping
/// the list explicit blocks `http`, `concat`, `subfile` and similar protocols
/// that a crafted playlist or IPC path could otherwise trigger.
pub const MEDIA_PROTOCOL_WHITELIST: &str = "file,pipe";

/// `CREATE_NO_WINDOW`: the release desktop build uses the Windows GUI
/// subsystem, so a console child spawned without it opens its own console
/// window (and closing that window kills the child).
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `std::process::Command` that never opens a console window on Windows.
/// Wrap it with `tokio::process::Command::from` for async spawns.
pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum HardwareEncoder {
    NvencH264,
    QsvH264,
    VaapiH264,
    AmfH264,
    VideoToolboxH264,
}

impl HardwareEncoder {
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::NvencH264 => "h264_nvenc",
            Self::QsvH264 => "h264_qsv",
            Self::VaapiH264 => "h264_vaapi",
            Self::AmfH264 => "h264_amf",
            Self::VideoToolboxH264 => "h264_videotoolbox",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::NvencH264 => "NVIDIA NVENC",
            Self::QsvH264 => "Intel Quick Sync",
            Self::VaapiH264 => "VA-API",
            Self::AmfH264 => "AMD AMF",
            Self::VideoToolboxH264 => "Apple VideoToolbox",
        }
    }

    pub fn supported_on(self, platform: &str) -> bool {
        match self {
            Self::NvencH264 => matches!(platform, "linux" | "windows"),
            Self::QsvH264 => matches!(platform, "linux" | "windows"),
            Self::VaapiH264 => platform == "linux",
            Self::AmfH264 => platform == "windows",
            Self::VideoToolboxH264 => platform == "macos",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EncoderCapabilities {
    pub h264: Vec<HardwareEncoder>,
    pub hevc: Vec<HardwareEncoder>,
    pub probed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EncoderPreference {
    #[default]
    Auto,
    CpuOnly,
    Prefer(HardwareEncoder),
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainReport {
    pub ok: bool,
    pub platform: String,
    pub tools: Vec<ToolStatus>,
    pub missing_required: Vec<ToolId>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub hardware_encoders: EncoderCapabilities,
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
            hardware_encoders: EncoderCapabilities::default(),
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

/// Environment variable used to override the path of a tool, e.g. `ffmpeg`
/// maps to `DVD_FORGE_FFMPEG` and `dvd+rw-mediainfo` to
/// `DVD_FORGE_DVD_RW_MEDIAINFO`.
pub fn binary_env_var(name: &str) -> String {
    let suffix: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("DVD_FORGE_{suffix}")
}

/// Resolves a tool binary for runtime use.
///
/// The lookup order is: explicit `DVD_FORGE_<NAME>` override, packaged
/// `resources/toolchains/<platform>` directories, system prefixes and finally
/// `PATH`. When nothing is found the bare name is returned so the operating
/// system can still resolve it at spawn time.
pub fn resolve_binary(name: &str) -> PathBuf {
    if let Some(value) = env::var_os(binary_env_var(name)) {
        let candidate = PathBuf::from(value);
        if is_executable(&candidate) {
            return candidate;
        }
    }
    let platform = env::consts::OS;
    if let Some(found) = find_binary_in_dirs(&[name], platform, &default_search_directories()) {
        return found;
    }
    PathBuf::from(name)
}

/// Runs a command with a hard deadline. The child is killed and reaped when
/// the timeout elapses so a hung tool cannot block the caller forever.
pub fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("stdout no redirigido"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| std::io::Error::other("stderr no redirigido"))?;
    // Drain both pipes concurrently with the child. Windows anonymous pipes
    // hold only 4 KiB; polling try_wait without reading deadlocks the child as
    // soon as a version banner or diagnostic exceeds that buffer.
    let stdout_reader = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer)?;
        Ok(buffer)
    });
    let stderr_reader = std::thread::spawn(move || -> std::io::Result<Vec<u8>> {
        let mut buffer = Vec::new();
        stderr.read_to_end(&mut buffer)?;
        Ok(buffer)
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            // Don't join the readers: a grandchild that inherited the pipes
            // (wrapper script, `sh -c`) keeps them open after the kill and
            // would hold us past the deadline. The threads exit on EOF.
            drop(stdout_reader);
            drop(stderr_reader);
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("el comando superó el límite de {timeout:?}"),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| std::io::Error::other("lector de stdout abortado"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| std::io::Error::other("lector de stderr abortado"))??;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

/// Detects every tool in `ToolId::ALL`, filtered by platform.
pub fn detect_toolchain() -> ToolchainReport {
    let platform = env::consts::OS;
    let directories = default_search_directories();
    // Each probe spawns a process (5 s timeout); run them in parallel so the
    // doctor costs max(probe) instead of sum(probes). Order is preserved.
    let statuses: Vec<ToolStatus> = std::thread::scope(|scope| {
        let probes: Vec<_> = SPECS
            .iter()
            .filter(|spec| tool_supported_on(spec.id, platform))
            .map(|spec| {
                let directories = &directories;
                scope.spawn(move || detect_tool(spec, platform, directories))
            })
            .collect();
        probes
            .into_iter()
            .map(|probe| {
                probe
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    let mut report = ToolchainReport::from_statuses(statuses);
    if let Some(ffmpeg) = report
        .tool(ToolId::Ffmpeg)
        .and_then(|tool| tool.path.as_deref())
    {
        report.hardware_encoders = detect_hardware_encoders(Path::new(ffmpeg));
    }
    report
}

/// Probes the FFmpeg hardware encoders available on this machine.
///
/// Presence in `ffmpeg -encoders` is not enough: the driver may be missing or
/// the GPU may be busy. Each encoder is confirmed with a one-frame lavfi smoke
/// test before it is reported as available. Results are cached per process.
pub fn detect_hardware_encoders(ffmpeg: &Path) -> EncoderCapabilities {
    static CACHE: std::sync::OnceLock<EncoderCapabilities> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| probe_hardware_encoders(ffmpeg))
        .clone()
}

fn probe_hardware_encoders(ffmpeg: &Path) -> EncoderCapabilities {
    let platform = env::consts::OS;
    let mut command = background_command(ffmpeg);
    command.args(["-hide_banner", "-encoders"]);
    let list = output_with_timeout(&mut command, Duration::from_secs(10))
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let mut h264 = Vec::new();
    for encoder in [
        HardwareEncoder::NvencH264,
        HardwareEncoder::QsvH264,
        HardwareEncoder::VaapiH264,
        HardwareEncoder::AmfH264,
        HardwareEncoder::VideoToolboxH264,
    ] {
        if !encoder.supported_on(platform)
            || !list.contains(encoder.ffmpeg_name())
            || !hardware_smoke_test(ffmpeg, encoder)
        {
            continue;
        }
        h264.push(encoder);
    }
    EncoderCapabilities {
        h264,
        hevc: Vec::new(),
        probed_at: Some(format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .unwrap_or(0)
        )),
    }
}

fn hardware_smoke_test(ffmpeg: &Path, encoder: HardwareEncoder) -> bool {
    let mut command = background_command(ffmpeg);
    command.args(["-hide_banner", "-loglevel", "error"]);
    if encoder == HardwareEncoder::VaapiH264 {
        command.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    command.args([
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x256:r=24",
        "-frames:v",
        "1",
    ]);
    if encoder == HardwareEncoder::VaapiH264 {
        command.args(["-vf", "format=nv12,hwupload"]);
    }
    command.args(["-c:v", encoder.ffmpeg_name(), "-f", "null"]);
    command.arg(if cfg!(windows) { "NUL" } else { "/dev/null" });
    output_with_timeout(&mut command, Duration::from_secs(5))
        .is_ok_and(|output| output.status.success())
}

pub fn resolve_hardware_encoder(
    preference: EncoderPreference,
    capabilities: &EncoderCapabilities,
) -> Option<HardwareEncoder> {
    match preference {
        EncoderPreference::CpuOnly => None,
        EncoderPreference::Auto => capabilities.h264.first().copied(),
        EncoderPreference::Prefer(encoder) => {
            capabilities.h264.contains(&encoder).then_some(encoder)
        }
    }
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

    let output = output_with_timeout(
        background_command(&path).args(spec.version_args),
        Duration::from_secs(5),
    );
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

    #[test]
    fn hardware_preference_never_selects_a_gpu_in_cpu_only_mode() {
        let capabilities = EncoderCapabilities {
            h264: vec![HardwareEncoder::NvencH264],
            hevc: Vec::new(),
            probed_at: None,
        };
        assert_eq!(
            resolve_hardware_encoder(EncoderPreference::CpuOnly, &capabilities),
            None
        );
        assert_eq!(
            resolve_hardware_encoder(EncoderPreference::Auto, &capabilities),
            Some(HardwareEncoder::NvencH264)
        );
        assert!(HardwareEncoder::VaapiH264.supported_on("linux"));
        assert!(!HardwareEncoder::VaapiH264.supported_on("windows"));
        assert!(HardwareEncoder::VideoToolboxH264.supported_on("macos"));
    }

    #[test]
    fn binary_override_env_var_normalizes_names() {
        assert_eq!(binary_env_var("ffmpeg"), "DVD_FORGE_FFMPEG");
        assert_eq!(
            binary_env_var("dvd+rw-mediainfo"),
            "DVD_FORGE_DVD_RW_MEDIAINFO"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_binary_honors_the_explicit_override() {
        let directory = temp_dir("override");
        let binary = directory.join("custom-tool");
        std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").expect("write");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let key = binary_env_var("dvdforge-custom-tool");
        env::set_var(&key, &binary);
        let resolved = resolve_binary("dvdforge-custom-tool");
        env::remove_var(&key);
        let _ = std::fs::remove_dir_all(&directory);
        assert_eq!(resolved, binary);
    }

    #[test]
    fn resolve_binary_falls_back_to_the_bare_name() {
        assert_eq!(
            resolve_binary("dvdforge-tool-that-does-not-exist"),
            PathBuf::from("dvdforge-tool-that-does-not-exist")
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_with_timeout_drains_large_stdout_and_stderr() {
        let output = output_with_timeout(
            Command::new("sh").args(["-c", "seq 1 200000; seq 1 200000 >&2"]),
            Duration::from_secs(30),
        )
        .expect("large output");
        assert!(output.status.success());
        assert!(output.stdout.len() > 4 * 1024);
        assert!(output.stderr.len() > 4 * 1024);
        assert!(output.stdout.starts_with(b"1\n"));
        assert!(output.stderr.starts_with(b"1\n"));
    }

    #[cfg(unix)]
    #[test]
    fn output_with_timeout_kills_hung_commands() {
        let started = std::time::Instant::now();
        let error = output_with_timeout(
            Command::new("sh").args(["-c", "sleep 5"]),
            Duration::from_millis(100),
        )
        .expect_err("timeout");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
