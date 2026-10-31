//! Read-only structural verifier shared by the CLI and the desktop app.
//!
//! The verifier deliberately parses the image itself instead of trusting the
//! writer that produced it. It validates the UDF envelope, then dispatches to
//! independent BDMV or DVD-Video checks.

pub mod bdmv;
pub mod dvd;
pub mod menu;
pub mod report;
pub mod stream;
pub mod udf;

use dvdforge_domain::Severity;
use libmkiso::{ReadAt, UdfReader};
use report::{CheckList, CheckStatus, VerifyLocation, VerifyProfile, VerifyReport};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

pub const CRATE_NAME: &str = "dvdforge-verify";
pub const SECTOR_BYTES: u64 = 2_048;

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("el ISO está vacío: {0}")]
    EmptyIso(String),
    #[error("no se pudo abrir el UDF: {0}")]
    Udf(String),
}

struct FileSource {
    file: Mutex<File>,
    len: u64,
}

impl FileSource {
    fn new(file: File, len: u64) -> Self {
        Self {
            file: Mutex::new(file),
            len,
        }
    }
}

impl ReadAt for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("verifier file lock poisoned"))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read(buffer)
    }
}

/// Verifies one ISO image synchronously. Callers that run inside an async
/// runtime should wrap this in `spawn_blocking`.
pub fn verify_iso(path: &Path, deep: bool) -> Result<VerifyReport, VerifyError> {
    let started = Instant::now();
    let mut raw = File::open(path)?;
    let size_bytes = raw.metadata()?.len();
    if size_bytes == 0 {
        return Err(VerifyError::EmptyIso(path.display().to_string()));
    }

    let mut checks = CheckList::new();
    udf::check_raw(&mut raw, size_bytes, &mut checks);
    if deep {
        udf::check_digest(path, &mut checks)?;
    } else {
        checks.skip(
            "UDF-DIGEST",
            "Usa --deep para calcular el SHA-256 completo.",
        );
    }

    raw.seek(SeekFrom::Start(0))?;
    let reader = match UdfReader::open_source(
        FileSource::new(raw, size_bytes),
        libmkiso::UdfLimits::default(),
    ) {
        Ok(reader) => {
            checks.pass(
                "UDF-ROOT",
                "El lector UDF indexó el árbol de directorios y el file set.",
            );
            checks.pass(
                "UDF-TAG266",
                "El lector validó los descriptores extendidos (Tag 266/EFE) de la imagen.",
            );
            checks.pass(
                "UDF-MIRROR",
                "La partición de metadata y su mirror se abrieron correctamente.",
            );
            udf::check_entries(&reader, size_bytes, &mut checks);
            Some(reader)
        }
        Err(error) => {
            checks.fail("UDF-ROOT", format!("No se pudo abrir el UDF: {error}"));
            checks.fail(
                "UDF-TAG266",
                "No se pudieron validar los descriptores extendidos.",
            );
            None
        }
    };

    let profile = if let Some(reader) = &reader {
        detect_profile(reader)
    } else {
        VerifyProfile::Auto
    };
    match (profile, reader.as_ref()) {
        (VerifyProfile::Bdmv, Some(reader)) => bdmv::verify(reader, &mut checks),
        (VerifyProfile::DvdVideo, Some(reader)) => dvd::verify(reader, size_bytes, &mut checks),
        (VerifyProfile::Auto, _) => checks.fail(
            "PROFILE-UNKNOWN",
            "La imagen no contiene VIDEO_TS/VIDEO_TS.IFO ni BDMV/index.bdmv.",
        ),
        (_, None) => {}
    }

    let checks = checks.into_vec();
    Ok(VerifyReport::from_checks(
        path.display().to_string(),
        size_bytes,
        profile,
        checks,
        started.elapsed().as_millis() as u64,
    ))
}

pub(crate) fn detect_profile(reader: &UdfReader<'_>) -> VerifyProfile {
    if has(reader, "BDMV/index.bdmv") {
        VerifyProfile::Bdmv
    } else if has(reader, "VIDEO_TS/VIDEO_TS.IFO") {
        VerifyProfile::DvdVideo
    } else {
        VerifyProfile::Auto
    }
}

pub(crate) fn has(reader: &UdfReader<'_>, path: &str) -> bool {
    reader
        .entries()
        .iter()
        .any(|entry| normalize(&entry.name) == path)
}

pub(crate) fn read_file(
    reader: &UdfReader<'_>,
    path: &str,
    maximum: u64,
) -> Result<Vec<u8>, String> {
    let index = reader
        .entries()
        .iter()
        .position(|entry| normalize(&entry.name) == path)
        .ok_or_else(|| format!("no existe {path}"))?;
    reader
        .read_entry(index, maximum)
        .map_err(|error| format!("no se pudo leer {path}: {error}"))
}

pub(crate) fn read_prefix(
    reader: &UdfReader<'_>,
    path: &str,
    length: usize,
) -> Result<Vec<u8>, String> {
    let index = reader
        .entries()
        .iter()
        .position(|entry| normalize(&entry.name) == path)
        .ok_or_else(|| format!("no existe {path}"))?;
    let mut output = vec![0u8; length];
    let read = reader
        .read_at(index, 0, &mut output)
        .map_err(|error| format!("no se pudo leer {path}: {error}"))?;
    output.truncate(read);
    Ok(output)
}

pub(crate) fn paths_with_prefix(reader: &UdfReader<'_>, prefix: &str) -> Vec<String> {
    reader
        .entries()
        .iter()
        .map(|entry| normalize(&entry.name))
        .filter(|name| name.starts_with(prefix))
        .collect()
}

pub(crate) fn normalize(value: &str) -> String {
    value
        .trim_start_matches('/')
        .trim_end_matches('/')
        .replace('\\', "/")
}

pub(crate) fn location(file: &str, offset: Option<u64>, sector: Option<u64>) -> VerifyLocation {
    VerifyLocation {
        file: file.to_string(),
        offset,
        sector,
    }
}

pub(crate) fn fail_with(
    checks: &mut CheckList,
    id: &str,
    message: String,
    file: &str,
    offset: Option<u64>,
) {
    checks.push(
        id,
        Severity::Error,
        CheckStatus::Fail,
        message,
        Some(location(file, offset, None)),
    );
}

pub(crate) fn warn_with(
    checks: &mut CheckList,
    id: &str,
    message: String,
    file: &str,
    offset: Option<u64>,
) {
    checks.push(
        id,
        Severity::Warning,
        CheckStatus::Warn,
        message,
        Some(location(file, offset, None)),
    );
}

pub(crate) fn be16(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)
        .map(|slice| u16::from_be_bytes([slice[0], slice[1]]))
}

pub(crate) fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .map(|slice| u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

pub(crate) fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    stream::hash_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libmkiso::{AllocationMode, UdfImage, UdfOptions, UdfPartition, UdfRevision};

    fn put16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
    }

    fn put32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn vts_ifo() -> Vec<u8> {
        let mut ifo = vec![0u8; 5 * 2048];
        ifo[..12].copy_from_slice(b"DVDVIDEO-VTS");
        put32(&mut ifo, 0xC4, 0);
        put32(&mut ifo, 0xCC, 1);
        let pgci = 2048;
        put16(&mut ifo, pgci, 1);
        put32(&mut ifo, pgci + 4, 0x30);
        put32(&mut ifo, pgci + 8 + 4, 0x10);
        let pgc = pgci + 0x10;
        ifo[pgc + 3] = 1;
        put16(&mut ifo, pgc + 0xE8, 0x20);
        let cell = pgc + 0x20;
        put32(&mut ifo, cell + 8, 100);
        put32(&mut ifo, cell + 0x14, 200);
        ifo
    }

    fn dvd_image(path: &Path, mismatch_bup: bool) {
        let mut image = UdfImage::new();
        let mut vmg = vec![0u8; 2 * 2048];
        vmg[..12].copy_from_slice(b"DVDVIDEO-VMG");
        vmg[0x23] = 0;
        let vts = vts_ifo();
        image
            .add_bytes("VIDEO_TS/VIDEO_TS.IFO", vmg.clone())
            .unwrap();
        image
            .add_bytes(
                "VIDEO_TS/VIDEO_TS.BUP",
                if mismatch_bup {
                    let mut bad = vmg;
                    bad[0x23] = 0x3f;
                    bad
                } else {
                    vmg
                },
            )
            .unwrap();
        image
            .add_bytes("VIDEO_TS/VTS_01_0.IFO", vts.clone())
            .unwrap();
        image.add_bytes("VIDEO_TS/VTS_01_0.BUP", vts).unwrap();
        let mut vob = vec![0u8; 100 * 2048];
        vob[..4].copy_from_slice(&[0x00, 0x00, 0x01, 0xBD]);
        vob[4..6].copy_from_slice(&0x0010u16.to_be_bytes());
        vob[6] = 0x80;
        vob[7] = 0;
        vob[8] = 0x20;
        vob[9..11].copy_from_slice(&0x0008u16.to_be_bytes());
        vob[11..13].copy_from_slice(&0x0004u16.to_be_bytes());
        image.add_bytes("VIDEO_TS/VTS_01_0.VOB", vob).unwrap();
        let options = UdfOptions {
            revision: UdfRevision::V102,
            partition: UdfPartition::Physical,
            allocation: AllocationMode::Short,
            ..UdfOptions::default()
        };
        image.write(path, &options).unwrap();
    }

    #[test]
    fn verifies_a_minimal_dvd_udf_tree_and_detects_bup_mismatch() {
        let root = std::env::temp_dir().join(format!("dvdforge-verify-dvd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let good = root.join("good.iso");
        dvd_image(&good, false);
        let report = verify_iso(&good, false).expect("verify good");
        assert_eq!(report.profile, VerifyProfile::DvdVideo);
        assert_eq!(report.verdict, report::VerifyVerdict::Pass);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "DVD-IFO-BUP" && check.status == report::CheckStatus::Pass));
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "DVD-SPU-SEQ" && check.status == report::CheckStatus::Pass));

        let bad = root.join("bad.iso");
        dvd_image(&bad, true);
        let report = verify_iso(&bad, false).expect("verify bad");
        assert_eq!(report.verdict, report::VerifyVerdict::Fail);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "DVD-IFO-BUP" && check.status == report::CheckStatus::Fail));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn verifies_a_minimal_bdmv_udf_tree() {
        let root =
            std::env::temp_dir().join(format!("dvdforge-verify-bdmv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let iso = root.join("bdmv.iso");
        let mut index = vec![0u8; 120];
        index[..8].copy_from_slice(b"INDX0200");
        put32(&mut index, 8, 40);
        put32(&mut index, 40, 40);
        let mut mobj = vec![0u8; 50];
        mobj[..8].copy_from_slice(b"MOBJ0200");
        put32(&mut mobj, 40, 6);
        put16(&mut mobj, 48, 0);
        let mut mpls = vec![0u8; 100];
        mpls[..8].copy_from_slice(b"MPLS0200");
        put32(&mut mpls, 8, 40);
        put32(&mut mpls, 40, 40);
        put16(&mut mpls, 46, 1);
        put16(&mut mpls, 50, 30);
        mpls[52..57].copy_from_slice(b"00000");
        put32(&mut mpls, 67, 90_000);
        let mut clpi = vec![0u8; 64];
        clpi[..8].copy_from_slice(b"HDMV0200");
        let mut m2ts = vec![0u8; 192 * 2];
        for packet in m2ts.as_chunks_mut::<192>().0 {
            packet[4] = 0x47;
        }
        let mut image = UdfImage::new();
        image.add_bytes("BDMV/index.bdmv", index).unwrap();
        image.add_bytes("BDMV/MovieObject.bdmv", mobj).unwrap();
        image.add_bytes("BDMV/PLAYLIST/00000.mpls", mpls).unwrap();
        image.add_bytes("BDMV/CLIPINF/00000.clpi", clpi).unwrap();
        image.add_bytes("BDMV/STREAM/00000.m2ts", m2ts).unwrap();
        let options = UdfOptions {
            revision: UdfRevision::V250,
            partition: UdfPartition::Metadata { mirror: true },
            allocation: AllocationMode::Long,
            ..UdfOptions::default()
        };
        image.write(&iso, &options).unwrap();
        let report = verify_iso(&iso, false).expect("verify bdmv");
        assert_eq!(report.profile, VerifyProfile::Bdmv);
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "BDMV-INDEX" && check.status == report::CheckStatus::Pass));
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "BDMV-MOBJ" && check.status == report::CheckStatus::Pass));
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "BDMV-MPLS" && check.status == report::CheckStatus::Pass));
        assert!(report
            .checks
            .iter()
            .any(|check| check.id == "BDMV-M2TS" && check.status == report::CheckStatus::Pass));
        let _ = std::fs::remove_dir_all(root);
    }
}
