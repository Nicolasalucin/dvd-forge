//! Self-contained project bundles (`.kinodisc-bundle`).
//!
//! A bundle is a deterministic tar stream compressed with zstd:
//!
//! ```text
//! manifest.json          format, versions, every file with size + SHA-256
//! project.json           the project with every path rewritten to `assets/…`
//! assets/<n>-<name>      media, external audio, subtitles and font files
//! ```
//!
//! Entries are sorted, timestamps/owners are zeroed and paths are relative,
//! so the same project always produces the same bundle. Import verifies
//! every file's size and SHA-256 while streaming it to disk, rejects path
//! traversal and leaves nothing behind when a check fails. All I/O is
//! streamed (no multi-gigabyte file is read into memory).

use crate::{project_to_json, CURRENT_SCHEMA_VERSION};
use kinodisc_domain::{AssetSource, Diagnostic, Project};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

pub const BUNDLE_FORMAT: &str = "kinodisc-bundle";
pub const BUNDLE_FORMAT_VERSION: u32 = 1;
pub const BUNDLE_EXTENSION: &str = "kinodisc";
/// Bundles written by DVD Forge ≤ 0.9 (same format, older names).
pub const LEGACY_BUNDLE_FORMAT: &str = "dvdforge-bundle";
pub const LEGACY_BUNDLE_EXTENSION: &str = "dvdforge-bundle";
const MANIFEST_NAME: &str = "manifest.json";
const PROJECT_NAME: &str = "project.json";
const IO_BUFFER: usize = 1024 * 1024;
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("E_BUNDLE_IO: {0}")]
    Io(#[from] io::Error),
    #[error("E_BUNDLE_CORRUPT: {0}")]
    Corrupt(String),
    #[error("E_BUNDLE_SCHEMA: {0}")]
    Schema(String),
    #[error("E_BUNDLE_DESTINATION: {0}")]
    Destination(String),
}

impl BundleError {
    /// Stable diagnostic code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "E_BUNDLE_IO",
            Self::Corrupt(_) => "E_BUNDLE_CORRUPT",
            Self::Schema(_) => "E_BUNDLE_SCHEMA",
            Self::Destination(_) => "E_BUNDLE_DESTINATION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BundleFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    pub format: String,
    pub format_version: u32,
    pub schema_version: u32,
    pub created_by: String,
    pub project_name: String,
    pub files: Vec<BundleFile>,
}

impl BundleManifest {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BundleReport {
    pub path: String,
    pub manifest: BundleManifest,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BundleImport {
    pub project_path: String,
    pub manifest: BundleManifest,
}

/// One file referenced by the project, with where it lives today.
struct Reference {
    source: PathBuf,
    bundle_path: String,
}

fn resolve(project_dir: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        project_dir.join(candidate)
    }
}

/// File name safe on every OS; keeps the extension.
fn sanitized_name(path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// Assigns bundle paths to referenced files, de-duplicating by source path.
#[derive(Default)]
struct Collector {
    by_source: HashMap<PathBuf, String>,
    references: Vec<Reference>,
    warnings: Vec<Diagnostic>,
}

impl Collector {
    /// Returns the bundle path for `path`, or `None` when the file cannot be
    /// embedded (a warning is recorded and the original path is kept).
    fn add(&mut self, project_dir: &Path, path: &str, what: &str) -> Option<String> {
        let source = resolve(project_dir, path);
        if !source.is_file() {
            self.warnings.push(Diagnostic::warning(
                "W_BUNDLE_EXTERNAL_ASSET",
                format!(
                    "No se pudo incluir {what} «{}»: el archivo no existe; el paquete conserva la ruta original.",
                    source.display()
                ),
            ));
            return None;
        }
        let key = fs::canonicalize(&source).unwrap_or_else(|_| source.clone());
        if let Some(existing) = self.by_source.get(&key) {
            return Some(existing.clone());
        }
        let bundle_path = format!(
            "assets/{:03}-{}",
            self.references.len() + 1,
            sanitized_name(&source)
        );
        self.by_source.insert(key, bundle_path.clone());
        self.references.push(Reference {
            source,
            bundle_path: bundle_path.clone(),
        });
        Some(bundle_path)
    }
}

/// Clones `project` with every file path pointing into the bundle.
fn portable_project(project: &Project, project_dir: &Path) -> (Project, Collector) {
    let mut project = project.clone();
    let mut collector = Collector::default();
    for asset in &mut project.assets {
        let current = match &asset.source {
            AssetSource::Project => asset.relative_path.clone(),
            AssetSource::External { absolute_path } => Some(absolute_path.clone()),
        };
        // Generated preview proxies are rebuilt on demand.
        asset.proxy = None;
        let Some(current) = current else { continue };
        if asset.is_placeholder() {
            continue;
        }
        if let Some(bundle_path) = collector.add(project_dir, &current, "el medio") {
            asset.source = AssetSource::Project;
            asset.relative_path = Some(bundle_path);
        }
    }
    for episode in &mut project.episodes {
        episode.thumbnail_path = None;
        for audio in &mut episode.audio {
            if let Some(path) = audio.source_path.clone() {
                if let Some(bundle_path) = collector.add(project_dir, &path, "el audio externo") {
                    audio.source_path = Some(bundle_path);
                }
            }
        }
        for track in &mut episode.subtitles {
            if let Some(path) = track.source_path.clone() {
                if let Some(bundle_path) = collector.add(project_dir, &path, "el subtítulo") {
                    track.source_path = Some(bundle_path);
                }
            }
            if let Some(path) = track.style.font_path.clone() {
                if let Some(bundle_path) = collector.add(project_dir, &path, "la fuente") {
                    track.style.font_path = Some(bundle_path);
                }
            }
        }
    }
    (project, collector)
}

fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut reader = BufReader::with_capacity(IO_BUFFER, File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; IO_BUFFER];
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

fn header(size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    // `append_data` fills the path (with GNU long-name support) and checksum.
    header
}

/// Writes `project` and every file it references into a bundle at `output`.
///
/// `project_dir` is the folder the project's relative paths are based on.
pub fn export_bundle(
    project: &Project,
    project_dir: &Path,
    output: &Path,
) -> Result<BundleReport, BundleError> {
    let (portable, collector) = portable_project(project, project_dir);
    let Collector {
        mut references,
        warnings,
        ..
    } = collector;
    references.sort_by(|a, b| a.bundle_path.cmp(&b.bundle_path));

    let project_json = project_to_json(&portable)
        .map_err(|error| BundleError::Corrupt(error.to_string()))?
        .into_bytes();
    let mut files = Vec::with_capacity(references.len() + 1);
    files.push(BundleFile {
        path: PROJECT_NAME.to_string(),
        bytes: project_json.len() as u64,
        sha256: hex::encode(Sha256::digest(&project_json)),
    });
    for reference in &references {
        let (bytes, sha256) = hash_file(&reference.source)?;
        files.push(BundleFile {
            path: reference.bundle_path.clone(),
            bytes,
            sha256,
        });
    }
    let manifest = BundleManifest {
        format: BUNDLE_FORMAT.to_string(),
        format_version: BUNDLE_FORMAT_VERSION,
        schema_version: portable.schema_version,
        created_by: format!("Kinodisc {}", env!("CARGO_PKG_VERSION")),
        project_name: portable.name.clone(),
        files,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| BundleError::Corrupt(error.to_string()))?;

    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let temporary = output.with_extension("partial");
    let result = (|| -> Result<(), BundleError> {
        let file = BufWriter::with_capacity(IO_BUFFER, File::create(&temporary)?);
        let encoder = zstd::stream::Encoder::new(file, ZSTD_LEVEL)?;
        let mut archive = tar::Builder::new(encoder);
        archive.mode(tar::HeaderMode::Deterministic);
        let mut head = header(manifest_json.len() as u64);
        archive.append_data(&mut head, MANIFEST_NAME, manifest_json.as_slice())?;
        let mut head = header(project_json.len() as u64);
        archive.append_data(&mut head, PROJECT_NAME, project_json.as_slice())?;
        for (reference, entry) in references.iter().zip(manifest.files.iter().skip(1)) {
            let reader = BufReader::with_capacity(IO_BUFFER, File::open(&reference.source)?);
            let mut head = header(entry.bytes);
            // A file that changes size while bundling would corrupt the tar
            // stream; `take` + the size check below fail closed instead.
            let mut limited = reader.take(entry.bytes);
            archive.append_data(&mut head, &entry.path, &mut limited)?;
            if limited.limit() != 0 {
                return Err(BundleError::Corrupt(format!(
                    "«{}» cambió de tamaño mientras se empaquetaba",
                    reference.source.display()
                )));
            }
        }
        let encoder = archive.into_inner()?;
        let mut file = encoder.finish()?;
        file.flush()?;
        file.into_inner()
            .map_err(|error| BundleError::Io(error.into_error()))?
            .sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    fs::rename(&temporary, output)?;
    Ok(BundleReport {
        path: output.display().to_string(),
        manifest,
        warnings,
    })
}

fn safe_relative(path: &Path) -> Result<PathBuf, BundleError> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            _ => {
                return Err(BundleError::Corrupt(format!(
                    "ruta no permitida en el paquete: {}",
                    path.display()
                )))
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(BundleError::Corrupt("entrada sin nombre".to_string()));
    }
    Ok(clean)
}

fn open_archive(bundle: &Path) -> Result<tar::Archive<impl Read>, BundleError> {
    let file = BufReader::with_capacity(IO_BUFFER, File::open(bundle)?);
    let decoder = zstd::stream::Decoder::with_buffer(file)
        .map_err(|error| BundleError::Corrupt(format!("no es un paquete zstd: {error}")))?;
    Ok(tar::Archive::new(decoder))
}

fn parse_manifest(bytes: &[u8]) -> Result<BundleManifest, BundleError> {
    let manifest: BundleManifest = serde_json::from_slice(bytes)
        .map_err(|error| BundleError::Corrupt(format!("manifest.json inválido: {error}")))?;
    if manifest.format != BUNDLE_FORMAT && manifest.format != LEGACY_BUNDLE_FORMAT {
        return Err(BundleError::Corrupt(format!(
            "formato «{}» desconocido",
            manifest.format
        )));
    }
    if manifest.format_version > BUNDLE_FORMAT_VERSION {
        return Err(BundleError::Schema(format!(
            "el paquete usa el formato {} y esta versión de Kinodisc lee hasta el {}; actualiza Kinodisc",
            manifest.format_version, BUNDLE_FORMAT_VERSION
        )));
    }
    if manifest.schema_version != CURRENT_SCHEMA_VERSION {
        return Err(BundleError::Schema(format!(
            "el proyecto usa el esquema {} y se esperaba {}",
            manifest.schema_version, CURRENT_SCHEMA_VERSION
        )));
    }
    Ok(manifest)
}

/// Reads only the manifest (first entry) of a bundle.
pub fn read_bundle_manifest(bundle: &Path) -> Result<BundleManifest, BundleError> {
    let mut archive = open_archive(bundle)?;
    let mut entries = archive.entries()?;
    let mut first = entries
        .next()
        .ok_or_else(|| BundleError::Corrupt("el paquete está vacío".to_string()))??;
    if first.path()?.as_ref() != Path::new(MANIFEST_NAME) {
        return Err(BundleError::Corrupt(
            "la primera entrada no es manifest.json".to_string(),
        ));
    }
    let mut bytes = Vec::new();
    (&mut first)
        .take(16 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    parse_manifest(&bytes)
}

/// Writes a reader to `target` while hashing; returns (bytes, sha256).
fn write_hashed(reader: &mut impl Read, target: &Path) -> io::Result<(u64, String)> {
    let mut writer = BufWriter::with_capacity(IO_BUFFER, File::create(target)?);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; IO_BUFFER];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        writer.write_all(&buffer[..read])?;
        total += read as u64;
    }
    writer.flush()?;
    Ok((total, hex::encode(hasher.finalize())))
}

/// Extracts `bundle` into `destination` (which must not exist or be empty)
/// and returns the path of the restored project.
pub fn import_bundle(bundle: &Path, destination: &Path) -> Result<BundleImport, BundleError> {
    if destination.exists() {
        let occupied = fs::read_dir(destination)?.next().is_some();
        if occupied {
            return Err(BundleError::Destination(format!(
                "la carpeta «{}» no está vacía",
                destination.display()
            )));
        }
    }
    let created = !destination.exists();
    fs::create_dir_all(destination)?;
    let result = extract(bundle, destination);
    if result.is_err() {
        // Leave nothing half-imported behind.
        if created {
            let _ = fs::remove_dir_all(destination);
        } else if let Ok(entries) = fs::read_dir(destination) {
            for entry in entries.flatten() {
                let _ = fs::remove_dir_all(entry.path()).or_else(|_| fs::remove_file(entry.path()));
            }
        }
    }
    result
}

fn extract(bundle: &Path, destination: &Path) -> Result<BundleImport, BundleError> {
    let mut archive = open_archive(bundle)?;
    let mut manifest: Option<BundleManifest> = None;
    let mut expected: BTreeMap<String, BundleFile> = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let raw_path = entry.path()?.into_owned();
        let relative = safe_relative(&raw_path)?;
        let name = relative.to_string_lossy().replace('\\', "/");
        if manifest.is_none() {
            if name != MANIFEST_NAME {
                return Err(BundleError::Corrupt(
                    "la primera entrada no es manifest.json".to_string(),
                ));
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(16 * 1024 * 1024)
                .read_to_end(&mut bytes)?;
            let parsed = parse_manifest(&bytes)?;
            for file in &parsed.files {
                safe_relative(Path::new(&file.path))?;
                expected.insert(file.path.clone(), file.clone());
            }
            manifest = Some(parsed);
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(BundleError::Corrupt(format!(
                "entrada no permitida (no es un archivo): {name}"
            )));
        }
        let Some(file) = expected.remove(&name) else {
            return Err(BundleError::Corrupt(format!(
                "«{name}» no figura en el manifiesto"
            )));
        };
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let (bytes, sha256) = write_hashed(&mut entry, &target)?;
        if bytes != file.bytes || sha256 != file.sha256 {
            return Err(BundleError::Corrupt(format!(
                "«{name}» está dañado (tamaño o SHA-256 distinto)"
            )));
        }
    }
    let manifest =
        manifest.ok_or_else(|| BundleError::Corrupt("el paquete está vacío".to_string()))?;
    if let Some(missing) = expected.keys().next() {
        return Err(BundleError::Corrupt(format!(
            "falta «{missing}» en el paquete"
        )));
    }
    let project_path = destination.join(PROJECT_NAME);
    // The restored project must load; schema mismatches surface here too.
    crate::load_project(&project_path).map_err(|error| BundleError::Schema(error.to_string()))?;
    Ok(BundleImport {
        project_path: project_path.display().to_string(),
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "kinodisc-bundle-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    /// Minimal fixture with a project-relative video, an external subtitle
    /// and one asset whose file is missing.
    fn fixture(dir: &Path) -> Project {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects");
        let mut project = crate::load_project(root.join("minimal.json")).expect("minimal fixture");
        fs::create_dir_all(dir.join("media")).unwrap();
        fs::write(dir.join("media/clip one.mp4"), vec![7u8; 300_000]).unwrap();
        let external = dir.join("outside.srt");
        fs::write(&external, "1\n00:00:00,000 --> 00:00:01,000\nHola\n").unwrap();
        for (index, asset) in project.assets.iter_mut().enumerate() {
            asset.source = AssetSource::Project;
            asset.relative_path = Some(if index == 0 {
                "media/clip one.mp4".to_string()
            } else {
                format!("media/missing-{index}.png")
            });
        }
        if let Some(episode) = project.episodes.first_mut() {
            if let Some(track) = episode.subtitles.first_mut() {
                track.source_path = Some(external.display().to_string());
            }
        }
        project
    }

    #[test]
    fn export_import_round_trip_restores_files_and_rewrites_paths() {
        let source = temp("src");
        let project = fixture(&source);
        let bundle = source.join("out/project.kinodisc-bundle");
        let report = export_bundle(&project, &source, &bundle).expect("export");
        assert!(bundle.is_file());
        assert!(report
            .warnings
            .iter()
            .all(|warning| warning.code == "W_BUNDLE_EXTERNAL_ASSET"));
        assert_eq!(report.manifest.files[0].path, "project.json");

        // Deterministic: a second export is byte-identical.
        let again = source.join("out/again.kinodisc-bundle");
        export_bundle(&project, &source, &again).expect("export again");
        assert_eq!(fs::read(&bundle).unwrap(), fs::read(&again).unwrap());

        assert_eq!(read_bundle_manifest(&bundle).unwrap(), report.manifest);

        let target = temp("dst");
        let imported = import_bundle(&bundle, &target.join("restored")).expect("import");
        let restored = crate::load_project(&imported.project_path).expect("restored project");
        let video = restored.assets[0].relative_path.clone().unwrap();
        assert!(video.starts_with("assets/"));
        assert!(!video.contains(' '));
        let restored_dir = target.join("restored");
        assert_eq!(
            fs::read(restored_dir.join(&video)).unwrap(),
            vec![7u8; 300_000]
        );
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn tampered_bundle_is_rejected_and_leaves_nothing_behind() {
        let source = temp("tamper-src");
        let project = fixture(&source);
        let bundle = source.join("p.kinodisc-bundle");
        export_bundle(&project, &source, &bundle).expect("export");
        // Re-pack the archive with one payload byte changed.
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        let mut archive = open_archive(&bundle).unwrap();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let name = entry.path().unwrap().to_string_lossy().into_owned();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            entries.push((name, bytes));
        }
        let last = entries.last_mut().unwrap();
        last.1[0] ^= 0xFF;
        let tampered = source.join("t.kinodisc-bundle");
        {
            let encoder = zstd::stream::Encoder::new(File::create(&tampered).unwrap(), 1).unwrap();
            let mut builder = tar::Builder::new(encoder);
            for (name, bytes) in &entries {
                let mut head = header(bytes.len() as u64);
                builder
                    .append_data(&mut head, name, bytes.as_slice())
                    .unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }
        let target = temp("tamper-dst").join("restored");
        let error = import_bundle(&tampered, &target).unwrap_err();
        assert_eq!(error.code(), "E_BUNDLE_CORRUPT");
        assert!(!target.exists());
        let _ = fs::remove_dir_all(&source);
    }

    #[test]
    fn traversal_and_non_empty_destination_are_refused() {
        assert!(safe_relative(Path::new("../evil")).is_err());
        assert!(safe_relative(Path::new("/abs/evil")).is_err());
        assert_eq!(
            safe_relative(Path::new("assets/./a.mp4")).unwrap(),
            PathBuf::from("assets/a.mp4")
        );
        let occupied = temp("occupied");
        fs::write(occupied.join("keep.txt"), "x").unwrap();
        let error = import_bundle(&occupied.join("missing.bundle"), &occupied).unwrap_err();
        assert_eq!(error.code(), "E_BUNDLE_DESTINATION");
        assert!(occupied.join("keep.txt").is_file());
        let _ = fs::remove_dir_all(&occupied);
    }
    #[test]
    fn corrupted_bundles_fail_cleanly() {
        let source = temp("fuzz-src");
        let project = fixture(&source);
        let bundle = source.join("p.kinodisc-bundle");
        export_bundle(&project, &source, &bundle).expect("export");
        let original = fs::read(&bundle).unwrap();
        let mut state: u64 = 0x5EED;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let case = source.join("case.kinodisc-bundle");
        for seed in 0..120 {
            let mut bytes = original.clone();
            if seed % 4 == 0 {
                let cut = (next() as usize) % bytes.len();
                bytes.truncate(cut);
            } else {
                for _ in 0..1 + next() % 6 {
                    let at = (next() as usize) % bytes.len();
                    bytes[at] = next() as u8;
                }
            }
            fs::write(&case, &bytes).unwrap();
            let target = source.join(format!("restored-{seed}"));
            let outcome = std::panic::catch_unwind(|| import_bundle(&case, &target));
            let result = outcome.unwrap_or_else(|_| panic!("import panicked (case {seed})"));
            if result.is_err() {
                assert!(!target.exists(), "case {seed} left files behind");
            }
        }
        let _ = fs::remove_dir_all(&source);
    }
}
