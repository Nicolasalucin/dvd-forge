//! ISO/UDF image creation and DVD-Video validation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;
use tokio::process::Command;

pub const CRATE_NAME: &str = "dvdforge-iso";
pub const SECTOR_BYTES: u64 = 2_048;
pub const DVD_UDF_REVISION: &str = "1.02";
pub const AVCHD_UDF_REVISION: &str = "2.50";
pub const VIDEO_TS_IFO: &str = "VIDEO_TS/VIDEO_TS.IFO";
pub const VTS_01_IFO: &str = "VIDEO_TS/VTS_01_0.IFO";

#[derive(Debug, Error)]
pub enum IsoError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("la etiqueta de volumen no es válida: {0}")]
    InvalidLabel(String),
    #[error("no existe {0}")]
    MissingSource(String),
    #[error("xorriso falló con código {code}: {stderr}")]
    XorrisoFailed { code: i32, stderr: String },
    #[error("el ISO no contiene {0}")]
    MissingIsoEntry(String),
    #[error("el ISO está vacío")]
    EmptyIso,
    #[error("no se pudo autorar el UDF del BDMV: {0}")]
    BdmvAuthoring(String),
}

#[derive(Debug, Clone)]
pub struct IsoReport {
    pub iso_path: PathBuf,
    pub size_bytes: u64,
    pub has_video_ts_ifo: bool,
    pub has_vts_01_ifo: bool,
    pub listing: String,
}

#[derive(Debug, Clone)]
pub struct Xorriso {
    pub binary: PathBuf,
    /// CachyOS ships xorriso without mkisofs' `-dvd-video` option. In that
    /// case the wrapper falls back to genisoimage/mkisofs for the image.
    pub fallback_binary: PathBuf,
}

impl Default for Xorriso {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("xorriso"),
            fallback_binary: PathBuf::from("genisoimage"),
        }
    }
}

impl Xorriso {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            fallback_binary: PathBuf::from("genisoimage"),
        }
    }

    /// Builds a DVD-Video ISO with the approved xorriso profile.
    pub async fn build_dvd_iso(
        &self,
        source_dir: &Path,
        iso_path: &Path,
        label: &str,
    ) -> Result<IsoReport, IsoError> {
        if label.trim().is_empty() || label.len() > 32 || !label.is_ascii() {
            return Err(IsoError::InvalidLabel(label.to_string()));
        }
        let video_ts = source_dir.join("VIDEO_TS");
        if !video_ts.is_dir() {
            return Err(IsoError::MissingSource(video_ts.display().to_string()));
        }
        if let Some(parent) = iso_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if iso_path.exists() {
            std::fs::remove_file(iso_path)?;
        }

        let result = Command::new(&self.binary)
            .arg("-as")
            .arg("mkisofs")
            .arg("-dvd-video")
            .arg("-udf")
            .arg("-V")
            .arg(label)
            .arg("-o")
            .arg(iso_path)
            .arg(source_dir)
            .kill_on_drop(true)
            .output()
            .await?;
        if !result.status.success() {
            let code = result.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            if stderr.contains("Unsupported option '-dvd-video'") {
                let fallback = Command::new(&self.fallback_binary)
                    .arg("-dvd-video")
                    .arg("-udf")
                    .arg("-V")
                    .arg(label)
                    .arg("-o")
                    .arg(iso_path)
                    .arg(source_dir)
                    .kill_on_drop(true)
                    .output()
                    .await?;
                if !fallback.status.success() {
                    let fallback_code = fallback.status.code().unwrap_or(-1);
                    let fallback_stderr =
                        String::from_utf8_lossy(&fallback.stderr).trim().to_string();
                    return Err(IsoError::XorrisoFailed {
                        code: fallback_code,
                        stderr: format!(
                            "xorriso no soporta -dvd-video y el fallback falló: {fallback_stderr}"
                        ),
                    });
                }
            } else {
                return Err(IsoError::XorrisoFailed { code, stderr });
            }
        }
        self.validate_dvd_iso(iso_path).await
    }

    pub async fn validate_dvd_iso(&self, iso_path: &Path) -> Result<IsoReport, IsoError> {
        let metadata = std::fs::metadata(iso_path)?;
        if metadata.len() == 0 {
            return Err(IsoError::EmptyIso);
        }
        let result = Command::new(&self.binary)
            .arg("-indev")
            .arg(iso_path)
            .arg("-find")
            .arg("/")
            .arg("-maxdepth")
            .arg("2")
            .arg("-exec")
            .arg("lsdl")
            .kill_on_drop(true)
            .output()
            .await?;
        if !result.status.success() {
            let code = result.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            return Err(IsoError::XorrisoFailed { code, stderr });
        }
        let listing = String::from_utf8_lossy(&result.stdout).into_owned();
        let has_video_ts_ifo = listing.contains(VIDEO_TS_IFO);
        let has_vts_01_ifo = listing.contains(VTS_01_IFO);
        if !has_video_ts_ifo {
            return Err(IsoError::MissingIsoEntry(VIDEO_TS_IFO.to_string()));
        }
        if !has_vts_01_ifo {
            return Err(IsoError::MissingIsoEntry(VTS_01_IFO.to_string()));
        }
        Ok(IsoReport {
            iso_path: iso_path.to_path_buf(),
            size_bytes: metadata.len(),
            has_video_ts_ifo,
            has_vts_01_ifo,
            listing,
        })
    }
}

pub fn sectors_for_bytes(bytes: u64) -> u64 {
    bytes.div_ceil(SECTOR_BYTES)
}

#[derive(Debug, Clone)]
pub struct BdmvIsoReport {
    pub iso_path: PathBuf,
    pub size_bytes: u64,
}

/// Builds a UDF 2.50 image from an already-authored BDMV folder.
///
/// tsMuxeR can create a Blu-ray folder but cannot attach native HDMV IG
/// streams. The render pipeline therefore muxes to a folder, patches the
/// playlist/index/material and only then wraps the final tree in UDF 2.50.
/// The image uses libmkiso's UDF 2.50 metadata partition (Metadata File +
/// Metadata Mirror File), which PS4/PS5 and standalone players expect. The
/// vendored libmkiso includes a fix for the extended metadata file entry.
/// This avoids a dependency on the tsMuxeR ISO writer, which would re-mux and
/// discard the patched files.
#[derive(Debug, Clone)]
pub struct BdmvIsoBuilder {
    pub udf_revision: libmkiso::UdfRevision,
}

impl Default for BdmvIsoBuilder {
    fn default() -> Self {
        Self {
            udf_revision: libmkiso::UdfRevision::V250,
        }
    }
}

impl BdmvIsoBuilder {
    pub async fn build(
        &self,
        source_dir: &Path,
        iso_path: &Path,
        label: &str,
    ) -> Result<BdmvIsoReport, IsoError> {
        self.build_with_cancel(source_dir, iso_path, label, None)
            .await
    }

    /// Builds the UDF image with a cooperative cancellation flag. libmkiso
    /// checks it while scanning and writing, so cancelling a render does not
    /// leave a blocking ISO writer running after the task is aborted.
    pub async fn build_with_cancel(
        &self,
        source_dir: &Path,
        iso_path: &Path,
        label: &str,
        cancel: Option<Arc<AtomicBool>>,
    ) -> Result<BdmvIsoReport, IsoError> {
        let index = source_dir.join("BDMV/index.bdmv");
        let movie_object = source_dir.join("BDMV/MovieObject.bdmv");
        let playlist = source_dir.join("BDMV/PLAYLIST/00000.mpls");
        if !index.is_file() {
            return Err(IsoError::MissingSource(index.display().to_string()));
        }
        if !movie_object.is_file() {
            return Err(IsoError::MissingSource(movie_object.display().to_string()));
        }
        if !playlist.is_file() {
            return Err(IsoError::MissingSource(playlist.display().to_string()));
        }
        if label.trim().is_empty() || !label.is_ascii() {
            return Err(IsoError::InvalidLabel(label.to_string()));
        }
        let label: String = label.chars().take(31).collect();
        if let Some(parent) = iso_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if iso_path.exists() {
            std::fs::remove_file(iso_path)?;
        }

        let source = source_dir.to_path_buf();
        let target = iso_path.to_path_buf();
        let label = label.to_string();
        let revision = self.udf_revision;
        tokio::task::spawn_blocking(move || {
            let options = libmkiso::UdfOptions {
                revision,
                partition: libmkiso::UdfPartition::Metadata { mirror: true },
                allocation: libmkiso::AllocationMode::Long,
                label,
                ..libmkiso::UdfOptions::default()
            };
            let checkpoint = {
                let cancel = cancel.clone();
                move || {
                    if cancel
                        .as_ref()
                        .is_some_and(|flag| flag.load(Ordering::Relaxed))
                    {
                        Err(anyhow::anyhow!("autoría UDF cancelada"))
                    } else {
                        Ok(())
                    }
                }
            };
            libmkiso::write_udf_with_cancel(&source, &target, &options, checkpoint)
                .map_err(|error| IsoError::BdmvAuthoring(error.to_string()))
        })
        .await
        .map_err(|error| IsoError::BdmvAuthoring(format!("tarea UDF abortada: {error}")))??;

        let metadata = std::fs::metadata(iso_path)?;
        if metadata.len() == 0 {
            return Err(IsoError::EmptyIso);
        }
        Ok(BdmvIsoReport {
            iso_path: iso_path.to_path_buf(),
            size_bytes: metadata.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_rounding_is_ceil() {
        assert_eq!(sectors_for_bytes(0), 0);
        assert_eq!(sectors_for_bytes(1), 1);
        assert_eq!(sectors_for_bytes(2_048), 1);
        assert_eq!(sectors_for_bytes(2_049), 2);
    }

    #[tokio::test]
    async fn cancelled_udf_write_does_not_publish_an_image() {
        let root = std::env::temp_dir().join(format!("dvdforge-iso-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("BDMV/PLAYLIST")).unwrap();
        std::fs::write(root.join("BDMV/index.bdmv"), b"INDX0200").unwrap();
        std::fs::write(root.join("BDMV/MovieObject.bdmv"), b"MOBJ0200").unwrap();
        std::fs::write(root.join("BDMV/PLAYLIST/00000.mpls"), b"MPLS0200").unwrap();
        let iso = root.with_extension("cancel.iso");
        let _ = std::fs::remove_file(&iso);
        let cancel = Arc::new(AtomicBool::new(true));
        let result = BdmvIsoBuilder::default()
            .build_with_cancel(&root, &iso, "CANCELLED", Some(cancel))
            .await;
        assert!(result.is_err());
        assert!(!iso.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn builds_a_readable_udf_250_bdmv_image() {
        let root = std::env::temp_dir().join(format!("dvdforge-iso-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("BDMV/PLAYLIST")).unwrap();
        std::fs::create_dir_all(root.join("BDMV/STREAM")).unwrap();
        std::fs::write(root.join("BDMV/index.bdmv"), b"INDX0200").unwrap();
        std::fs::write(root.join("BDMV/MovieObject.bdmv"), b"MOBJ0200").unwrap();
        std::fs::write(root.join("BDMV/PLAYLIST/00000.mpls"), b"MPLS0200").unwrap();
        std::fs::write(root.join("BDMV/STREAM/00000.m2ts"), vec![0u8; 192]).unwrap();
        let iso = root.with_extension("iso");
        let report = BdmvIsoBuilder::default()
            .build(&root, &iso, "DVD_FORGE_TEST")
            .await
            .expect("udf iso");
        assert!(report.size_bytes > 0);
        assert!(iso.is_file());

        let image = std::fs::read(&iso).unwrap();
        assert!(
            image
                .windows(b"*UDF Metadata Partition".len())
                .any(|window| window == b"*UDF Metadata Partition"),
            "falta el mapa de partición de metadatos UDF 2.50"
        );
        // Extended File Entries (tag 266) for the metadata file (type 250)
        // and metadata mirror (type 251) must both be present.
        let mut metadata_entry = false;
        let mut mirror_entry = false;
        for block in image.as_chunks::<2_048>().0.iter().take(600) {
            if block.len() >= 28 && u16::from_le_bytes([block[0], block[1]]) == 266 {
                match block[27] {
                    250 => metadata_entry = true,
                    251 => mirror_entry = true,
                    _ => {}
                }
            }
        }
        assert!(metadata_entry, "falta el Metadata File Entry");
        assert!(mirror_entry, "falta el Metadata Mirror File Entry");

        let reader =
            libmkiso::UdfReader::open(&image, libmkiso::UdfLimits::default()).expect("leer UDF");
        let index = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "BDMV/index.bdmv")
            .expect("BDMV/index.bdmv en la imagen");
        let content = reader.read_entry(index, 1 << 20).unwrap();
        assert_eq!(content, b"INDX0200");

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&iso);
    }
}
