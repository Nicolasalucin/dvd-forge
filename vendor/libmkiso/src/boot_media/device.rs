//! Explicit Linux whole-device writes with identity and readback checks.

use crate::boot_media::{
    Error, Result,
    progress::{CancellationToken, Observer},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Read-only device facts. Identity is derived from sysfs, serial and capacity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub id: String,
    pub capacity: u64,
    pub mounted: bool,
    pub model: Option<String>,
    pub serial: Option<String>,
}
/// Requested readback policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceVerification {
    None,
    Full,
}
/// Destructive authorization must match immediately re-inspected identity.
pub struct DiskWriteOptions {
    pub erase: bool,
    pub expect_device_id: String,
    pub verify: DeviceVerification,
}
/// A successful write covers only the image range, not trailing device space.
#[derive(Debug, Serialize)]
pub struct DeviceWriteReport {
    pub device: DeviceInfo,
    pub bytes_written: u64,
    pub bytes_verified: u64,
}

/// Inspect an explicitly chosen whole block device; regular files are rejected.
pub fn disk_inspect(path: &Path) -> Result<DeviceInfo> {
    #[cfg(target_os = "linux")]
    {
        linux::inspect(path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        Err(Error::Unsupported(
            "device inspection is available on Linux only".into(),
        ))
    }
}

/// Write an explicitly authorized image to an unmounted exclusive whole device.
/// Cancellation or failure can leave the device partially written.
pub fn disk_write(
    image: &Path,
    device: &Path,
    options: &DiskWriteOptions,
    observer: &mut dyn Observer,
    cancellation: &CancellationToken,
) -> Result<DeviceWriteReport> {
    #[cfg(target_os = "linux")]
    {
        linux::write(image, device, options, observer, cancellation)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (image, device, options, observer, cancellation);
        Err(Error::Unsupported(
            "device writing is available on Linux only".into(),
        ))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::boot_media::progress::{Phase, PhaseProgress, ProgressUnit};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
        os::{
            fd::AsRawFd,
            unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        },
    };

    fn optional_text(path: PathBuf) -> Option<String> {
        fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    }
    fn sysfs(path: &Path) -> Result<(PathBuf, u64)> {
        let metadata = fs::metadata(path)?;
        if !metadata.file_type().is_block_device() {
            return Err(Error::InvalidInput(
                "device target must be a block device, not a regular file".into(),
            ));
        }
        let major = libc::major(metadata.rdev());
        let minor = libc::minor(metadata.rdev());
        let sys = fs::canonicalize(format!("/sys/dev/block/{major}:{minor}"))?;
        if sys.join("partition").exists() {
            return Err(Error::InvalidInput(
                "select a whole disk, not a partition".into(),
            ));
        }
        Ok((sys, metadata.rdev()))
    }
    fn device_numbers(sys: &Path) -> Result<Vec<String>> {
        let mut devices = vec![fs::read_to_string(sys.join("dev"))?.trim().to_owned()];
        for entry in fs::read_dir(sys)? {
            let path = entry?.path();
            if path.join("partition").exists() {
                devices.push(fs::read_to_string(path.join("dev"))?.trim().to_owned());
            }
        }
        Ok(devices)
    }
    fn mountinfo_uses_devices(info: &str, devices: &[String]) -> bool {
        info.lines().any(|line| {
            line.split_whitespace()
                .nth(2)
                .is_some_and(|dev| devices.iter().any(|candidate| candidate == dev))
        })
    }
    fn mounted(sys: &Path) -> Result<bool> {
        let devices = device_numbers(sys)?;
        // Check every readable process mount namespace, since our own namespace
        // does not establish that another namespace has no mounted partitions.
        for entry in fs::read_dir("/proc")? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().parse::<u32>().is_err() {
                continue;
            }
            match fs::read_to_string(entry.path().join("mountinfo")) {
                Ok(info) => {
                    if mountinfo_uses_devices(&info, &devices) {
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        if fs::read_dir(sys.join("holders"))?.next().is_some() {
            return Ok(true);
        }
        for entry in fs::read_dir(sys)? {
            let path = entry?.path();
            if path.join("partition").exists()
                && fs::read_dir(path.join("holders"))?.next().is_some()
            {
                return Ok(true);
            }
        }
        // Swap may use a whole disk or a partition without appearing in mountinfo.
        for line in fs::read_to_string("/proc/swaps")?.lines().skip(1) {
            if let Some(path) = line.split_whitespace().next() {
                let metadata = fs::metadata(path)?;
                let dev = format!(
                    "{}:{}",
                    libc::major(metadata.rdev()),
                    libc::minor(metadata.rdev())
                );
                if metadata.file_type().is_block_device() && devices.contains(&dev) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn inspect(path: &Path) -> Result<DeviceInfo> {
        let (sys, _) = sysfs(path)?;
        let capacity = fs::read_to_string(sys.join("size"))?
            .trim()
            .parse::<u64>()
            .map_err(|_| Error::InvalidInput("invalid kernel block capacity".into()))?
            .checked_mul(512)
            .ok_or_else(|| Error::Resource("device capacity overflow".into()))?;
        let serial = optional_text(sys.join("device/serial"));
        let model = optional_text(sys.join("device/model"));
        let mut hash = Sha256::new();
        hash.update(sys.as_os_str().as_encoded_bytes());
        hash.update([0]);
        hash.update(serial.as_deref().unwrap_or("").as_bytes());
        hash.update(capacity.to_le_bytes());
        Ok(DeviceInfo {
            path: fs::canonicalize(path)?,
            id: format!("linux:{}", hex::encode(hash.finalize())),
            capacity,
            mounted: mounted(&sys)?,
            model,
            serial,
        })
    }
    fn checkpoint(token: &CancellationToken) -> Result<()> {
        token.checkpoint().map_err(|_| Error::Cancelled)
    }
    fn accounting(error: crate::boot_media::progress::ProgressError) -> Error {
        Error::Resource(error.to_string())
    }
    pub(super) fn write(
        image: &Path,
        device: &Path,
        options: &DiskWriteOptions,
        observer: &mut dyn Observer,
        cancellation: &CancellationToken,
    ) -> Result<DeviceWriteReport> {
        if !options.erase || options.expect_device_id.is_empty() {
            return Err(Error::InvalidInput(
                "device writes require --erase and a matching --expect-device-id".into(),
            ));
        }
        let info = inspect(device)?;
        if info.id != options.expect_device_id {
            return Err(Error::InvalidInput(
                "device identity does not match the expected identity".into(),
            ));
        }
        if info.serial.is_none() {
            return Err(Error::Unsupported(
                "device lacks a stable serial; destructive writes are unavailable".into(),
            ));
        }
        if info.mounted {
            return Err(Error::InvalidInput(
                "device or its partitions are mounted or in use".into(),
            ));
        }
        let mut source = File::open(image)?;
        let original = source.metadata()?;
        if !original.is_file() {
            return Err(Error::InvalidInput(
                "input image must be a regular file".into(),
            ));
        }
        let size = original.len();
        if size == 0 || size > info.capacity {
            return Err(Error::InvalidInput(
                "image is empty or exceeds device capacity".into(),
            ));
        }
        let (sys, expected_rdev) = sysfs(device)?;
        let sector = fs::read_to_string(sys.join("queue/logical_block_size"))?
            .trim()
            .parse::<u64>()
            .map_err(|_| Error::InvalidInput("invalid logical sector size".into()))?;
        if sector != 512 || !size.is_multiple_of(sector) {
            return Err(Error::Unsupported(
                "device writer currently requires 512-byte logical sectors and aligned image size"
                    .into(),
            ));
        }
        checkpoint(cancellation)?;
        let mut target = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_EXCL | libc::O_NOFOLLOW)
            .open(&info.path)?;
        if target.metadata()?.rdev() != expected_rdev
            || inspect(device)?.id != info.id
            || mounted(&sys)?
        {
            return Err(Error::InvalidInput(
                "device identity or use changed before writing".into(),
            ));
        }
        let mut opened_capacity = 0u64;
        // SAFETY: BLKGETSIZE64 writes one u64 into a valid pointer for an open block-device fd.
        if unsafe {
            libc::ioctl(
                target.as_raw_fd(),
                0x8008_1272_u32 as _,
                &mut opened_capacity,
            )
        } < 0
        {
            return Err(Error::Io(std::io::Error::last_os_error()));
        }
        if opened_capacity != info.capacity {
            return Err(Error::InvalidInput(
                "device capacity changed before writing".into(),
            ));
        }
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut emitted_hash = Sha256::new();
        let mut phase = PhaseProgress::start(
            observer,
            1,
            Phase::CopyDevice,
            Some(size),
            ProgressUnit::Bytes,
            None,
        );
        let copy_result = (|| -> Result<()> {
            let mut remaining = size;
            while remaining > 0 {
                checkpoint(cancellation)?;
                let count = remaining.min(buffer.len() as u64) as usize;
                source.read_exact(&mut buffer[..count])?;
                let mut written = 0;
                while written < count {
                    checkpoint(cancellation)?;
                    match target.write(&buffer[written..count]) {
                        Ok(0) => {
                            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into());
                        }
                        Ok(bytes) => {
                            written += bytes;
                            phase.advance(bytes as u64).map_err(accounting)?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
                emitted_hash.update(&buffer[..count]);
                remaining -= count as u64;
            }
            let after = source.metadata()?;
            if after.len() != size
                || after.dev() != original.dev()
                || after.ino() != original.ino()
                || after.mtime() != original.mtime()
                || after.mtime_nsec() != original.mtime_nsec()
                || after.ctime() != original.ctime()
                || after.ctime_nsec() != original.ctime_nsec()
            {
                return Err(Error::InvalidInput(
                    "input image changed during device write".into(),
                ));
            }
            Ok(())
        })();
        if let Err(error) = copy_result {
            let _ = if matches!(error, Error::Cancelled) {
                phase.cancel()
            } else {
                phase.fail()
            };
            let _ = target.sync_all();
            return Err(error);
        }
        phase.finish().map_err(accounting)?;
        let mut flush = PhaseProgress::start(
            observer,
            2,
            Phase::Flush,
            None,
            ProgressUnit::Operations,
            None,
        );
        if let Err(error) = target.sync_all() {
            let _ = flush.fail();
            return Err(error.into());
        }
        flush.finish().map_err(accounting)?;
        let bytes_verified = match options.verify {
            DeviceVerification::None => 0,
            DeviceVerification::Full => {
                // SAFETY: BLKFLSBUF takes no pointer argument and invalidates
                // cached block pages after sync, so readback reaches the device.
                if unsafe { libc::ioctl(target.as_raw_fd(), 0x1261_u32 as _) } < 0 {
                    return Err(Error::Io(std::io::Error::last_os_error()));
                }
                target.seek(SeekFrom::Start(0))?;
                let mut phase = PhaseProgress::start(
                    observer,
                    3,
                    Phase::Verify,
                    Some(size),
                    ProgressUnit::Bytes,
                    None,
                );
                let verification = (|| -> Result<()> {
                    let mut read_hash = Sha256::new();
                    let mut remaining = size;
                    while remaining > 0 {
                        checkpoint(cancellation)?;
                        let count = remaining.min(buffer.len() as u64) as usize;
                        target.read_exact(&mut buffer[..count])?;
                        read_hash.update(&buffer[..count]);
                        remaining -= count as u64;
                        phase.advance(count as u64).map_err(accounting)?;
                    }
                    if read_hash.finalize() != emitted_hash.finalize() {
                        return Err(Error::Verification(
                            "device readback differs from written image".into(),
                        ));
                    }
                    Ok(())
                })();
                if let Err(error) = verification {
                    let _ = if matches!(error, Error::Cancelled) {
                        phase.cancel()
                    } else {
                        phase.fail()
                    };
                    return Err(error);
                }
                phase.finish().map_err(accounting)?;
                size
            }
        };
        checkpoint(cancellation)?;
        Ok(DeviceWriteReport {
            device: info,
            bytes_written: size,
            bytes_verified,
        })
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn mounted_child_partition_is_detected_from_sysfs_and_mountinfo() {
            let sys = tempfile::tempdir().unwrap();
            fs::write(sys.path().join("dev"), "8:16\n").unwrap();
            let partition = sys.path().join("sdb1");
            fs::create_dir(&partition).unwrap();
            fs::write(partition.join("partition"), "1\n").unwrap();
            fs::write(partition.join("dev"), "8:17\n").unwrap();
            let devices = device_numbers(sys.path()).unwrap();
            assert!(mountinfo_uses_devices(
                "22 1 8:17 / /mnt rw - vfat /dev/sdb1 rw\n",
                &devices
            ));
            assert!(!mountinfo_uses_devices(
                "22 1 8:170 / /mnt rw - vfat /dev/other rw\n",
                &devices
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "linux")]
    fn inspection_rejects_regular_files() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(matches!(
            disk_inspect(file.path()),
            Err(Error::InvalidInput(_))
        ));
    }
    #[test]
    fn write_requires_explicit_destructive_authorization() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let error = disk_write(
            file.path(),
            file.path(),
            &DiskWriteOptions {
                erase: false,
                expect_device_id: String::new(),
                verify: DeviceVerification::Full,
            },
            &mut crate::boot_media::progress::NoProgress,
            &CancellationToken::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidInput(_) | Error::Unsupported(_)
        ));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn authorized_write_still_rejects_regular_target_without_changing_it() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"preserved").unwrap();
        let error = disk_write(
            file.path(),
            file.path(),
            &DiskWriteOptions {
                erase: true,
                expect_device_id: "expected".into(),
                verify: DeviceVerification::Full,
            },
            &mut crate::boot_media::progress::NoProgress,
            &CancellationToken::default(),
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(_)));
        assert_eq!(std::fs::read(file.path()).unwrap(), b"preserved");
    }
}
