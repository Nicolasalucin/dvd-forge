//! One-click installation of external tools into a per-user folder.
//!
//! Only pinned, versioned archives are used: every download is checked
//! against its published size and SHA-256 before anything is extracted, the
//! archive listing is checked for path traversal, only the named executables
//! are extracted, and no script from the archive is ever run. Downloads use
//! the system `curl` and archives the system `tar` (bsdtar reads ZIP), both
//! shipped with Windows 10+, macOS and Linux distributions.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A pinned archive that provides one or more tools.
#[derive(Debug, Clone, Copy)]
pub struct ToolDownload {
    pub id: &'static str,
    pub version: &'static str,
    /// `std::env::consts::OS` + `-` + `ARCH`.
    pub platform: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    /// Executables extracted from the archive (matched by file name).
    pub files: &'static [&'static str],
    pub license: &'static str,
    pub homepage: &'static str,
    /// Extract every file of the archive (programs plus their DLLs), not
    /// only `files`.
    pub bundle: bool,
}

/// Pinned downloads. Digests are the ones GitHub publishes for the assets.
pub const DOWNLOADS: &[ToolDownload] = &[ToolDownload {
    id: "ffmpeg",
    version: "9.0.2",
    platform: "windows-x86_64",
    url: "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip",
    sha256: "60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba",
    size: 114_768_076,
    files: &["ffmpeg.exe", "ffprobe.exe"],
    license: "GPL-3.0-or-later (FFmpeg essentials build by Gyan Doshi)",
    homepage: "https://www.gyan.dev/ffmpeg/builds/",
    bundle: false,
},
ToolDownload {
    id: "dvdauthor",
    version: "0.7.2",
    platform: "windows-x86_64",
    url: "https://github.com/Nicolasalucin/kinodisc/releases/download/tools-dvdauthor-0.7.2/dvdauthor-0.7.2-windows-x86_64.zip",
    sha256: "26742c9dbb81dc5c0597d1313b31ff946f76cbf11eb3d03e7aac1d7e81639636",
    size: 5_939_822,
    files: &["dvdauthor.exe", "spumux.exe"],
    license: "GPL-2.0-or-later (dvdauthor 0.7.2, built by the Kinodisc CI from the official sources)",
    homepage: "https://github.com/ldo/dvdauthor",
    bundle: true,
}];

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("E_TOOLCHAIN_UNAVAILABLE: no hay descarga automática de {0} para este sistema")]
    Unavailable(String),
    #[error("E_TOOLCHAIN_DOWNLOAD: {0}")]
    Download(String),
    #[error("E_TOOLCHAIN_CHECKSUM: {0}")]
    Checksum(String),
    #[error("E_TOOLCHAIN_ARCHIVE: {0}")]
    Archive(String),
    #[error("E_TOOLCHAIN_IO: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of an installation, also written next to the binaries.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionReport {
    pub tool: String,
    pub version: String,
    pub directory: String,
    pub files: Vec<String>,
    pub sha256: String,
    pub url: String,
    pub license: String,
    /// `false` when a verified copy was already installed (no download).
    pub downloaded: bool,
    /// SHA-256 of every installed file, checked before a tool is launched.
    #[serde(default)]
    pub file_hashes: Vec<FileHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FileHash {
    pub file: String,
    pub sha256: String,
}

/// Pinned downloads for this computer whose programs cannot be found (or
/// were refused by the integrity check, e.g. a pre-1.0 install without
/// per-file hashes) and should be installed before rendering.
pub fn needs_install() -> Vec<&'static ToolDownload> {
    let platform = current_platform();
    DOWNLOADS
        .iter()
        .filter(|download| download.platform == platform)
        .filter(|download| {
            download.files.iter().any(|file| {
                let stem = file.trim_end_matches(".exe");
                crate::find_binary(stem).is_none()
            })
        })
        .collect()
}

/// True when every file recorded in `report` is still byte-identical.
fn files_unchanged(directory: &Path, report: &ProvisionReport) -> bool {
    !report.file_hashes.is_empty()
        && report.file_hashes.iter().all(|entry| {
            sha256_file(&directory.join(&entry.file))
                .map(|(_, hash)| hash == entry.sha256)
                .unwrap_or(false)
        })
}

/// Uncached core of [`is_trusted_binary`]: `name` in `directory` is trusted
/// when a provenance record lists it with unchanged hashes and no
/// unrecorded shared library sits in the folder.
fn folder_trusts(directory: &Path, name: &str) -> bool {
    let reports: Vec<ProvisionReport> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(".provenance.json")
        })
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice::<ProvisionReport>(&bytes).ok())
        .collect();
    let recorded = |file: &str| {
        reports.iter().any(|report| {
            report
                .file_hashes
                .iter()
                .any(|entry| entry.file.eq_ignore_ascii_case(file))
        })
    };
    // Windows loads DLLs from the program's folder first: an unrecorded DLL
    // next to our tools (DLL planting) makes the whole folder untrusted.
    let planted_library = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .any(|file| {
            let lower = file.to_ascii_lowercase();
            (lower.ends_with(".dll") || lower.ends_with(".so") || lower.ends_with(".dylib"))
                && !recorded(&file)
        });
    !planted_library
        && reports.iter().any(|report| {
            report
                .file_hashes
                .iter()
                .any(|entry| entry.file.eq_ignore_ascii_case(name))
                && files_unchanged(directory, report)
        })
}

/// Whether `binary` may be launched. Files outside the per-user toolchain
/// folder are not ours to vouch for (PATH, portable folder) and pass. Files
/// inside it must be listed in a provenance record with an unchanged
/// SHA-256 — a tampered or unknown file there is refused, so malware cannot
/// drop a fake ffmpeg.exe or DLL into the folder Kinodisc trusts. Results
/// are cached per process (hashing ~100 MB once is fine, every launch is not).
pub fn is_trusted_binary(binary: &Path) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static CACHE: Mutex<Option<HashMap<PathBuf, bool>>> = Mutex::new(None);
    let Some(directory) = user_toolchain_dir() else {
        return true;
    };
    let Ok(canonical_dir) = fs::canonicalize(&directory) else {
        return true;
    };
    let Ok(canonical) = fs::canonicalize(binary) else {
        return true;
    };
    if canonical.parent() != Some(canonical_dir.as_path()) {
        return true;
    }
    if let Ok(cache) = CACHE.lock() {
        if let Some(verdict) = cache.as_ref().and_then(|map| map.get(&canonical)) {
            return *verdict;
        }
    }
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let verdict = folder_trusts(&canonical_dir, &name);
    if let Ok(mut cache) = CACHE.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(canonical, verdict);
    }
    verdict
}

pub fn current_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Pinned download for `tool` on this computer, if any.
pub fn download_for(tool: &str) -> Option<&'static ToolDownload> {
    let platform = current_platform();
    DOWNLOADS
        .iter()
        .find(|download| download.platform == platform && download.files_for(tool))
}

impl ToolDownload {
    fn files_for(&self, tool: &str) -> bool {
        self.id == tool
            || self
                .files
                .iter()
                .any(|file| file.trim_end_matches(".exe") == tool)
    }
}

/// Command a person can run to install `tool` with the system package
/// manager when there is no pinned download.
pub fn install_hint(tool: &str) -> Option<String> {
    let package = match tool {
        "ffmpeg" | "ffprobe" => "ffmpeg",
        "dvdauthor" | "spumux" => "dvdauthor",
        "xorriso" => "xorriso",
        "growisofs" | "dvd+rw-mediainfo" => "dvd+rw-tools",
        _ => return None,
    };
    match std::env::consts::OS {
        "windows" if package == "ffmpeg" => Some("winget install Gyan.FFmpeg".to_string()),
        "windows" if package == "dvdauthor" => {
            Some("kinodisc-cli toolchain install dvdauthor".to_string())
        }
        "windows" => None,
        "macos" => Some(format!("brew install {package}")),
        "freebsd" => Some(format!("pkg install {package}")),
        _ => Some(format!(
            "sudo apt install {package}   ·   sudo dnf install {package}   ·   sudo pacman -S {package}"
        )),
    }
}

/// Per-user folder searched before PATH (`<data>/org.kinodisc.app/toolchain`).
pub fn user_toolchain_dir() -> Option<PathBuf> {
    crate::legacy::app_data_dir().map(|dir| dir.join("toolchain"))
}

fn sha256_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut reader = BufReader::with_capacity(1024 * 1024, fs::File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hex::encode(hasher.finalize())))
}

/// Absolute path of an OS-provided helper (curl/tar ship in System32 since
/// Windows 10 1803), so a planted `tar.exe` next to the app or on a
/// user-controlled PATH entry cannot be picked up. Other systems use PATH.
fn system_tool(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let candidate = PathBuf::from(root)
                .join("System32")
                .join(format!("{name}.exe"));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(name)
}

/// Archive entries must be relative and must not climb out of the target.
pub fn archive_entry_is_safe(entry: &str) -> bool {
    let normalized = entry.replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && !normalized.contains(':')
        && normalized.split('/').all(|part| part != "..")
}

fn provenance_path(directory: &Path, download: &ToolDownload) -> PathBuf {
    directory.join(format!("{}.provenance.json", download.id))
}

fn already_installed(directory: &Path, download: &ToolDownload) -> Option<ProvisionReport> {
    let report: ProvisionReport =
        serde_json::from_slice(&fs::read(provenance_path(directory, download)).ok()?).ok()?;
    // A pre-1.0 record has no per-file hashes: reinstall so every file is
    // covered by a verified hash.
    let complete = report.sha256 == download.sha256
        && download
            .files
            .iter()
            .all(|file| directory.join(file).is_file())
        && files_unchanged(directory, &report);
    complete.then_some(ProvisionReport {
        downloaded: false,
        ..report
    })
}

fn run(command: &mut Command, what: &str) -> Result<std::process::Output, ProvisionError> {
    let output = crate::background_command(command.get_program())
        .args(command.get_args())
        .output()
        .map_err(|error| {
            ProvisionError::Download(format!("no se pudo ejecutar {what}: {error}"))
        })?;
    Ok(output)
}

/// Downloads, verifies and installs `download`. `progress(done, total)` is
/// called while the archive downloads.
pub fn provision(
    download: &ToolDownload,
    mut progress: impl FnMut(u64, u64),
) -> Result<ProvisionReport, ProvisionError> {
    let directory = user_toolchain_dir()
        .ok_or_else(|| ProvisionError::Download("no se encontró la carpeta de usuario".into()))?;
    fs::create_dir_all(&directory)?;
    // Only the user may write where Kinodisc keeps the programs it runs.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    if let Some(report) = already_installed(&directory, download) {
        return Ok(report);
    }
    let staging = directory.join(format!(".staging-{}", download.id));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let result = install(download, &directory, &staging, &mut progress);
    let _ = fs::remove_dir_all(&staging);
    result
}

fn install(
    download: &ToolDownload,
    directory: &Path,
    staging: &Path,
    progress: &mut impl FnMut(u64, u64),
) -> Result<ProvisionReport, ProvisionError> {
    let archive = staging.join("download.zip");
    // Download (curl follows redirects and fails on HTTP errors).
    let mut child = crate::background_command(system_tool("curl"))
        .args([
            "--location",
            "--fail",
            "--silent",
            "--show-error",
            "--output",
        ])
        .arg(&archive)
        .arg(download.url)
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| ProvisionError::Download(format!("no se pudo ejecutar curl: {error}")))?;
    loop {
        let done = fs::metadata(&archive).map(|meta| meta.len()).unwrap_or(0);
        progress(done.min(download.size), download.size);
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                return Err(ProvisionError::Download(format!(
                    "la descarga falló ({}); comprueba la conexión a Internet",
                    stderr.trim()
                )));
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    progress(download.size, download.size);

    // Integrity before anything is extracted.
    let (size, sha256) = sha256_file(&archive)?;
    if size != download.size || sha256 != download.sha256 {
        return Err(ProvisionError::Checksum(format!(
            "el archivo descargado no coincide con la versión publicada (tamaño {size}, SHA-256 {sha256})"
        )));
    }

    // List entries, refuse traversal, find the wanted executables.
    let listing = run(
        Command::new(system_tool("tar")).arg("-tf").arg(&archive),
        "tar",
    )?;
    if !listing.status.success() {
        return Err(ProvisionError::Archive(
            "no se pudo leer el contenido del archivo".into(),
        ));
    }
    let entries: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    if let Some(bad) = entries.iter().find(|entry| !archive_entry_is_safe(entry)) {
        return Err(ProvisionError::Archive(format!("ruta no permitida: {bad}")));
    }
    let leaf = |entry: &str| {
        entry
            .replace('\\', "/")
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    // Every named executable must be present.
    let mut wanted = Vec::new();
    for file in download.files {
        let entry = entries
            .iter()
            .find(|entry| leaf(entry) == *file)
            .ok_or_else(|| ProvisionError::Archive(format!("falta {file} en el archivo")))?;
        wanted.push(entry.clone());
    }
    // Bundles also need their libraries (DLLs, licence) next to the programs.
    if download.bundle {
        for entry in &entries {
            let name = leaf(entry);
            if !name.is_empty() && !entry.ends_with('/') && !wanted.contains(entry) {
                wanted.push(entry.clone());
            }
        }
    }
    let extract_dir = staging.join("x");
    fs::create_dir_all(&extract_dir)?;
    let extracted = run(
        Command::new(system_tool("tar"))
            .arg("-xf")
            .arg(&archive)
            .arg("-C")
            .arg(&extract_dir)
            .args(&wanted),
        "tar",
    )?;
    if !extracted.status.success() {
        return Err(ProvisionError::Archive(format!(
            "no se pudo extraer: {}",
            String::from_utf8_lossy(&extracted.stderr).trim()
        )));
    }
    let mut installed = Vec::new();
    let mut file_hashes = Vec::new();
    for entry in &wanted {
        let source = extract_dir.join(entry);
        let name = leaf(entry);
        let target = directory.join(&name);
        let _ = fs::remove_file(&target);
        fs::rename(&source, &target).or_else(|_| fs::copy(&source, &target).map(|_| ()))?;
        let (_, sha256) = sha256_file(&target)?;
        file_hashes.push(FileHash { file: name, sha256 });
        installed.push(target.display().to_string());
    }
    let report = ProvisionReport {
        tool: download.id.to_string(),
        version: download.version.to_string(),
        directory: directory.display().to_string(),
        files: installed,
        sha256: download.sha256.to_string(),
        url: download.url.to_string(),
        license: download.license.to_string(),
        downloaded: true,
        file_hashes,
    };
    if let Ok(json) = serde_json::to_vec_pretty(&report) {
        fs::write(provenance_path(directory, download), json)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_entries_are_rejected() {
        assert!(archive_entry_is_safe(
            "ffmpeg-9.0.2-essentials_build/bin/ffmpeg.exe"
        ));
        assert!(!archive_entry_is_safe("../evil.exe"));
        assert!(!archive_entry_is_safe("a/../../evil.exe"));
        assert!(!archive_entry_is_safe("/etc/passwd"));
        assert!(!archive_entry_is_safe("C:/Windows/evil.exe"));
        assert!(!archive_entry_is_safe(""));
    }

    #[test]
    fn pinned_downloads_are_well_formed() {
        for download in DOWNLOADS {
            assert!(download.url.starts_with("https://"));
            assert_eq!(download.sha256.len(), 64);
            assert!(download.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(download.size > 0);
            assert!(!download.files.is_empty());
            assert!(download.files_for(download.id));
        }
    }

    #[test]
    fn ffprobe_is_provided_by_the_ffmpeg_archive() {
        let ffmpeg = DOWNLOADS
            .iter()
            .find(|download| download.id == "ffmpeg")
            .unwrap();
        assert!(ffmpeg.files_for("ffprobe"));
        assert!(!ffmpeg.files_for("dvdauthor"));
    }

    #[test]
    fn install_hints_cover_common_tools() {
        assert!(install_hint("ffmpeg").is_some());
        assert!(install_hint("unknown-tool").is_none());
    }
    #[test]
    fn checksum_mismatch_fails_before_extracting() {
        let root = std::env::temp_dir().join(format!("kinodisc-provision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let payload = root.join("payload.zip");
        fs::write(&payload, b"not the published archive").unwrap();
        let url = format!(
            "file:///{}",
            payload.display().to_string().replace('\\', "/")
        );
        let download = ToolDownload {
            id: "ffmpeg",
            version: "test",
            platform: "test",
            url: Box::leak(url.into_boxed_str()),
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            size: 25,
            files: &["ffmpeg.exe"],
            license: "test",
            homepage: "test",
            bundle: false,
        };
        let directory = root.join("toolchain");
        let staging = directory.join(".staging-ffmpeg");
        fs::create_dir_all(&staging).unwrap();
        let error = install(&download, &directory, &staging, &mut |_, _| {}).unwrap_err();
        assert!(matches!(error, ProvisionError::Checksum(_)), "{error}");
        assert!(!directory.join("ffmpeg.exe").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tampered_or_unknown_files_in_the_toolchain_are_refused() {
        let base = std::env::temp_dir().join(format!("kinodisc-trust-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let previous = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", &base);
        let directory = user_toolchain_dir().unwrap();
        fs::create_dir_all(&directory).unwrap();
        let tool = directory.join("ffmpeg.exe");
        fs::write(&tool, b"genuine").unwrap();
        let (_, sha256) = sha256_file(&tool).unwrap();
        let report = ProvisionReport {
            tool: "ffmpeg".into(),
            version: "1".into(),
            directory: directory.display().to_string(),
            files: vec![tool.display().to_string()],
            sha256: "x".into(),
            url: "x".into(),
            license: "x".into(),
            downloaded: true,
            file_hashes: vec![FileHash {
                file: "ffmpeg.exe".into(),
                sha256,
            }],
        };
        fs::write(
            directory.join("ffmpeg.provenance.json"),
            serde_json::to_vec(&report).unwrap(),
        )
        .unwrap();
        assert!(is_trusted_binary(&tool));
        // A file dropped into the folder without a record is refused.
        let planted = directory.join("dvdauthor.exe");
        fs::write(&planted, b"evil").unwrap();
        assert!(!is_trusted_binary(&planted));
        // An unrecorded DLL next to the tools poisons the folder.
        fs::remove_file(&planted).unwrap();
        assert!(folder_trusts(&directory, "ffmpeg.exe"));
        fs::write(directory.join("version.dll"), b"evil").unwrap();
        assert!(!folder_trusts(&directory, "ffmpeg.exe"));
        fs::remove_file(directory.join("version.dll")).unwrap();
        // Files elsewhere are not judged here.
        assert!(is_trusted_binary(&base.join("elsewhere.exe")));
        // Tampering is detected by the hash check itself.
        fs::write(&tool, b"tampered").unwrap();
        assert!(!files_unchanged(&directory, &report));
        match previous {
            Some(value) => std::env::set_var("XDG_DATA_HOME", value),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        let _ = fs::remove_dir_all(&base);
    }
}
