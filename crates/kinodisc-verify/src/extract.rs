//! Copies the files of a disc image (UDF) into a folder, e.g. to hand a
//! `VIDEO_TS`/`BDMV` folder to a media player or to import an existing disc.
//!
//! The image is untrusted input: every path is checked to stay inside the
//! destination (no `..`, roots, drive prefixes or reserved names), symbolic
//! links and named streams are skipped, and nothing outside `dest` is written.

use libmkiso::{UdfEntryKind, UdfReader};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractReport {
    pub files: usize,
    pub bytes: u64,
    /// Entries ignored because they are links, streams or unsafe names.
    pub skipped: usize,
}

/// Relative, normal-component path for an image entry, or `None` when the
/// name could escape the destination or is not representable on disk.
pub fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let trimmed = name.trim_matches('/');
    if trimmed.is_empty() || trimmed.contains('\\') || trimmed.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in trimmed.split('/') {
        let reserved = {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        };
        if part.is_empty()
            || part == "."
            || part == ".."
            || reserved
            || part.ends_with('.')
            || part.ends_with(' ')
            || part
                .chars()
                .any(|c| c.is_control() || "<>\"|?*".contains(c))
        {
            return None;
        }
        out.push(part);
    }
    out.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(out)
}

/// Extracts every regular file of the UDF image at `iso` below `dest`.
/// Existing files with the same name are replaced; `cancelled` is polled
/// between 4 MiB chunks.
pub fn extract_iso(
    iso: &Path,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ExtractReport, String> {
    let source = libmkiso::FileSource::open(iso)
        .map_err(|error| format!("no se pudo abrir {}: {error}", iso.display()))?;
    let reader =
        UdfReader::open_source(source, libmkiso::UdfLimits::default()).map_err(|error| {
            format!("la imagen no tiene un sistema de archivos UDF válido: {error}")
        })?;
    extract_reader(&reader, dest, cancelled)
}

fn extract_reader(
    reader: &UdfReader<'_>,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ExtractReport, String> {
    std::fs::create_dir_all(dest)
        .map_err(|error| format!("no se pudo crear {}: {error}", dest.display()))?;
    let mut report = ExtractReport::default();
    let mut buffer = vec![0u8; 4 << 20];
    for (index, entry) in reader.entries().iter().enumerate() {
        let wanted = matches!(entry.kind, UdfEntryKind::File | UdfEntryKind::Directory)
            && entry.stream.is_none()
            && entry.link_target.is_none();
        let Some(relative) = wanted.then(|| safe_relative_path(&entry.name)).flatten() else {
            if !entry.name.trim_matches('/').is_empty() {
                report.skipped += 1;
            }
            continue;
        };
        let target = dest.join(&relative);
        if entry.directory {
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("no se pudo crear {}: {error}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("no se pudo crear {}: {error}", parent.display()))?;
        }
        let partial = target.with_extension("kinodisc-part");
        let mut file = std::fs::File::create(&partial)
            .map_err(|error| format!("no se pudo escribir {}: {error}", partial.display()))?;
        let mut offset = 0u64;
        while offset < entry.size {
            if cancelled() {
                drop(file);
                let _ = std::fs::remove_file(&partial);
                return Err("cancelado".to_string());
            }
            let read = reader
                .read_at(index, offset, &mut buffer)
                .map_err(|error| format!("no se pudo leer {}: {error}", entry.name))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|error| format!("no se pudo escribir {}: {error}", partial.display()))?;
            offset += read as u64;
        }
        file.flush().map_err(|error| error.to_string())?;
        drop(file);
        if offset != entry.size {
            let _ = std::fs::remove_file(&partial);
            return Err(format!("{} está truncado en la imagen", entry.name));
        }
        let _ = std::fs::remove_file(&target);
        std::fs::rename(&partial, &target)
            .map_err(|error| format!("no se pudo escribir {}: {error}", target.display()))?;
        report.files += 1;
        report.bytes += entry.size;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_names_are_rejected() {
        for bad in [
            "",
            "..",
            "../x",
            "VIDEO_TS/../../x",
            "C:/x",
            "a\\b",
            "con",
            "COM1.txt",
            "x.",
            "x ",
            "a/./b",
            "a?b",
        ] {
            assert_eq!(safe_relative_path(bad), None, "{bad:?}");
        }
        assert_eq!(
            safe_relative_path("/VIDEO_TS/VTS_01_1.VOB"),
            Some(PathBuf::from("VIDEO_TS").join("VTS_01_1.VOB"))
        );
        assert_eq!(
            safe_relative_path("BDMV/STREAM/00000.m2ts").map(|p| p.components().count()),
            Some(3)
        );
    }

    #[test]
    fn native_dvd_iso_round_trips_to_a_folder() {
        let base = std::env::temp_dir().join(format!("kinodisc-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let tree = base.join("tree");
        std::fs::create_dir_all(tree.join("VIDEO_TS")).unwrap();
        std::fs::create_dir_all(tree.join("AUDIO_TS")).unwrap();
        let vob: Vec<u8> = (0..3 * 2048u32 + 77).map(|i| (i % 251) as u8).collect();
        std::fs::write(tree.join("VIDEO_TS/VIDEO_TS.IFO"), vec![1u8; 2048]).unwrap();
        std::fs::write(tree.join("VIDEO_TS/VTS_01_1.VOB"), &vob).unwrap();
        let iso = base.join("disc.iso");
        kinodisc_iso_writer(&tree, &iso);
        let out = base.join("out");
        let report = extract_iso(&iso, &out, &|| false).unwrap();
        assert_eq!(report.files, 2);
        assert_eq!(
            std::fs::read(out.join("VIDEO_TS/VTS_01_1.VOB")).unwrap(),
            vob
        );
        assert!(out.join("AUDIO_TS").is_dir());
        assert!(extract_iso(&iso, &base.join("cancel"), &|| true).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    fn kinodisc_iso_writer(tree: &Path, iso: &Path) {
        kinodisc_iso::write_dvd_video_iso(tree, iso, "TEST", &|| false).expect("write iso");
    }
}
