//! Bounded optical operations. Firmware compatibility is never inferred from a write.
use crate::boot_media::{
    Error, Result,
    progress::{CancellationToken, Observer, Phase, ProgressEvent, ProgressState, ProgressUnit},
};
use crate::{
    IsoOptions,
    iso9660::{IsoReader, Limits, Namespace, ReadOptions},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
};

/// Per-operation resource ceilings, applied before extraction.
#[derive(Debug, Clone)]
pub struct OperationLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: u64,
}
impl Default for OperationLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_depth: 64,
            max_metadata_bytes: 16 << 20,
            max_output_bytes: 8 << 40,
        }
    }
}
/// Deterministic writer settings and explicit output replacement.
#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub iso: IsoOptions,
    pub replace: bool,
    pub limits: OperationLimits,
}
/// Explicit boot disposition when editing an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootPolicy {
    Remove,
    Preserve,
    Rebuild,
}
/// Event and cancellation routing independent of terminal rendering.
pub struct OperationContext<'a> {
    pub observer: &'a mut dyn Observer,
    pub cancellation: &'a CancellationToken,
    pub operation_id: u64,
}
impl OperationContext<'_> {
    fn checkpoint(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn event(
        &mut self,
        phase: Phase,
        completed: u64,
        total: Option<u64>,
        unit: ProgressUnit,
        state: ProgressState,
    ) {
        // Each phase invocation has its own identity, including repeated verification phases.
        if state == ProgressState::Started {
            self.operation_id = self
                .operation_id
                .checked_add(1)
                .unwrap_or(self.operation_id);
        }
        self.observer.observe(&ProgressEvent {
            operation_id: self.operation_id,
            phase,
            completed,
            total,
            unit,
            entry_id: None,
            state,
        });
    }
}
/// Logical payload report; hashes establish contents, not bootability or installation.
#[derive(Debug, Clone, Serialize)]
pub struct EntryReport {
    pub path: String,
    pub directory: bool,
    pub size: u64,
    pub sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ImageReport {
    pub image: PathBuf,
    pub image_bytes: u64,
    pub entries: Vec<EntryReport>,
    pub bootable: bool,
    pub sha256: Option<String>,
}
fn optical(error: crate::iso9660::Error) -> Error {
    match error {
        crate::iso9660::Error::Io(e) if e.kind() == io::ErrorKind::Interrupted => Error::Cancelled,
        crate::iso9660::Error::Io(e) => Error::Io(e),
        crate::iso9660::Error::Unsupported(e) => Error::Unsupported(e),
        crate::iso9660::Error::ResourceLimit(e) => Error::Resource(e.into()),
        e => Error::Optical(e.to_string()),
    }
}
fn safe_path(name: &str, limits: &OperationLimits) -> Result<PathBuf> {
    if name.contains('\\') || name.contains(':') || name.contains('\0') {
        return Err(Error::InvalidInput(format!("unsafe image path: {name}")));
    }
    let path = Path::new(name);
    let count = path.components().count();
    if count == 0
        || count > limits.max_depth
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(Error::InvalidInput(format!(
            "unsafe or too deep image path: {name}"
        )));
    }
    Ok(path.to_owned())
}
fn check_entries(reader: &IsoReader<File>, limits: &OperationLimits) -> Result<u64> {
    let mut seen = HashSet::new();
    let mut bytes = 0u64;
    for entry in reader.entries() {
        safe_path(&entry.name, limits)?;
        if !seen.insert(entry.name.clone()) {
            return Err(Error::InvalidInput(format!(
                "duplicate image path: {}",
                entry.name
            )));
        }
        if entry.link_target.is_some()
            || entry.unix.as_ref().is_some_and(|m| {
                m.mode & 0o170000 != if entry.directory { 0o040000 } else { 0o100000 }
            })
        {
            return Err(Error::Unsupported(format!(
                "symlink or special-file extraction: {}",
                entry.name
            )));
        }
        bytes = bytes
            .checked_add(entry.size)
            .ok_or_else(|| Error::Resource("payload size overflow".into()))?;
        if bytes > limits.max_output_bytes {
            return Err(Error::Resource("payload byte budget".into()));
        }
    }
    Ok(bytes)
}
fn regular(path: &Path) -> Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() {
        return Err(Error::InvalidInput(format!(
            "expected regular file: {}",
            path.display()
        )));
    }
    Ok(m)
}
fn same_identity(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        false
    }
}
fn unchanged(path: &Path, original: &fs::Metadata) -> Result<()> {
    let current = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if current.mode() != original.mode()
            || current.uid() != original.uid()
            || current.gid() != original.gid()
        {
            return Err(Error::InvalidInput(format!(
                "input metadata changed: {}",
                path.display()
            )));
        }
    }
    if current.is_dir() != original.is_dir()
        || current.len() != original.len()
        || current.modified()? != original.modified()?
        || (cfg!(unix) && !same_identity(&current, original))
    {
        return Err(Error::InvalidInput(format!(
            "input changed during operation: {}",
            path.display()
        )));
    }
    Ok(())
}
fn destination(output: &Path, replace: bool, protected: &[&Path]) -> Result<PathBuf> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = fs::canonicalize(parent)?;
    let name = output
        .file_name()
        .ok_or_else(|| Error::InvalidInput("output requires a filename".into()))?;
    let output = parent.join(name);
    let existing = match fs::symlink_metadata(&output) {
        Ok(m) => {
            if !m.is_file() {
                return Err(Error::InvalidInput("output is not a regular file".into()));
            }
            if !replace {
                return Err(Error::InvalidInput("output exists; use --replace".into()));
            }
            Some(m)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    for path in protected {
        let source = fs::canonicalize(path)?;
        let m = fs::metadata(&source)?;
        if output == source
            || (m.is_dir() && output.starts_with(&source))
            || (existing.is_some() && crate::boot_media::plan::same_file(&output, &source)?)
        {
            return Err(Error::InvalidInput(format!(
                "output aliases protected input: {}",
                path.display()
            )));
        }
    }
    Ok(output)
}
fn boot_record(file: &mut File) -> Result<bool> {
    // Descriptor sequence is bounded independently of the filesystem index.
    let mut sector = [0; 2048];
    for block in 16..272u64 {
        file.seek(SeekFrom::Start(block * 2048))?;
        file.read_exact(&mut sector)?;
        if &sector[1..6] != b"CD001" {
            return Err(Error::Optical("invalid volume descriptor sequence".into()));
        }
        if sector[0] == 0 {
            return Ok(true);
        }
        if sector[0] == 255 {
            return Ok(false);
        }
    }
    Err(Error::Resource("volume descriptor count".into()))
}
fn open(
    image: &Path,
    limits: &OperationLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<(IsoReader<File>, ImageReport, Snapshot)> {
    ctx.checkpoint()?;
    ctx.event(
        Phase::OpenImage,
        0,
        None,
        ProgressUnit::Entries,
        ProgressState::Started,
    );
    let opened = (|| {
        let original = regular(image)?;
        let mut file = File::open(image)?;
        let input_identity = crate::boot_media::plan::file_identity(&file)?;
        let bootable = boot_record(&mut file)?;
        let reader = IsoReader::open_with_options(
            file,
            ReadOptions {
                namespace: Namespace::PreferRockRidge,
                limits: Limits {
                    max_entries: limits.max_entries as u64,
                    max_metadata_bytes: limits.max_metadata_bytes as u64,
                    max_nesting_depth: limits.max_depth,
                },
            },
        )
        .map_err(optical)?;
        let entries = reader
            .entries()
            .iter()
            .map(|e| EntryReport {
                path: e.name.clone(),
                directory: e.directory,
                size: e.size,
                sha256: None,
            })
            .collect();
        ctx.checkpoint()?;
        Ok((reader, entries, original, bootable, input_identity))
    })();
    let (reader, entries, original, bootable, input_identity) = match opened {
        Ok(value) => value,
        Err(e) => {
            ctx.event(
                Phase::OpenImage,
                0,
                None,
                ProgressUnit::Entries,
                if matches!(e, Error::Cancelled) {
                    ProgressState::Cancelled
                } else {
                    ProgressState::Failed
                },
            );
            return Err(e);
        }
    };
    ctx.checkpoint()?;
    ctx.event(
        Phase::OpenImage,
        reader.entries().len() as u64,
        None,
        ProgressUnit::Entries,
        ProgressState::Finished,
    );
    Ok((
        reader,
        ImageReport {
            image: image.to_owned(),
            image_bytes: original.len(),
            entries,
            bootable,
            sha256: None,
        },
        Snapshot {
            path: image.to_owned(),
            identity: input_identity,
            metadata: original,
            digest: [0; 32],
        },
    ))
}
/// Index supported ISO9660/Joliet/Rock Ridge metadata without reading every payload.
pub fn inspect(
    image: &Path,
    limits: &OperationLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<ImageReport> {
    let (_, report, original) = open(image, limits, ctx)?;
    check_snapshot(&original)?;
    Ok(report)
}
// A separate lifetime-free writer closure avoids sharing mutable observers between operations.
struct CheckedWriter<'a, 'b, W> {
    output: W,
    digest: Sha256,
    ctx: &'a mut OperationContext<'b>,
    completed: &'a mut u64,
    total: u64,
    phase: Phase,
}
impl<W: Write> Write for CheckedWriter<'_, '_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.ctx.cancellation.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = self.output.write(bytes)?;
        self.digest.update(&bytes[..n]);
        *self.completed = self
            .completed
            .checked_add(n as u64)
            .ok_or_else(|| io::Error::other("counter overflow"))?;
        self.ctx.event(
            self.phase,
            *self.completed,
            Some(self.total),
            ProgressUnit::Bytes,
            ProgressState::Advanced,
        );
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}
/// Verify every indexed regular payload is readable and report SHA-256 per file.
/// There is no embedded expected checksum in ISO9660; this does not attest authenticity.
pub fn verify(
    image: &Path,
    limits: &OperationLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<ImageReport> {
    let (mut reader, mut report, original) = open(image, limits, ctx)?;
    let total = check_entries(&reader, limits)?;
    let mut completed = 0;
    ctx.event(
        Phase::Verify,
        0,
        Some(total),
        ProgressUnit::Bytes,
        ProgressState::Started,
    );
    let result = (|| {
        for (i, entry) in report.entries.iter_mut().enumerate() {
            if entry.directory {
                continue;
            }
            let mut writer = CheckedWriter {
                output: io::sink(),
                digest: Sha256::new(),
                ctx,
                completed: &mut completed,
                total,
                phase: Phase::Verify,
            };
            reader.extract(i, &mut writer).map_err(optical)?;
            entry.sha256 = Some(hex::encode(writer.digest.finalize()));
        }
        check_snapshot(&original)?;
        Ok(())
    })();
    ctx.event(
        Phase::Verify,
        completed,
        Some(total),
        ProgressUnit::Bytes,
        match &result {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    result?;
    report.sha256 = Some(hex::encode(hash_image(image, ctx)?));
    check_snapshot(&original)?;
    Ok(report)
}
fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| Error::InvalidInput("NUL in staging path".into()))?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| Error::InvalidInput("NUL in output path".into()))?;
        // Both paths remain alive and NUL-terminated throughout the call. RENAME_NOREPLACE
        // prevents a concurrent creator from having its destination overwritten.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            #[link_name = "MoveFileExW"]
            fn move_file_ex_w(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let wide = |path: &Path| -> Result<Vec<u16>> {
            let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
            if value.contains(&0) {
                return Err(Error::InvalidInput("NUL in publication path".into()));
            }
            value.push(0);
            Ok(value)
        };
        let source = wide(source)?;
        let destination = wide(destination)?;
        // Both UTF-16 strings remain live and terminated for the call. Flags
        // exclude replacement and cross-volume copying; std::fs::rename may
        // replace an empty destination directory on Windows.
        if unsafe { move_file_ex_w(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| Error::InvalidInput("NUL in staging path".into()))?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| Error::InvalidInput("NUL in output path".into()))?;
        // The live, NUL-terminated paths are valid for this call. RENAME_EXCL
        // atomically rejects an existing destination, including an empty directory.
        if unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) }
            != 0
        {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (source, destination);
        Err(Error::Unsupported(
            "atomic no-replace directory publication is unavailable on this host".into(),
        ))
    }
}
/// Extract to a new directory via a private sibling. Existing directories are rejected.
/// Symlinks and special files are rejected; POSIX metadata is not restored.
pub fn extract(
    image: &Path,
    output: &Path,
    limits: &OperationLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<ImageReport> {
    let (mut reader, report, original) = open(image, limits, ctx)?;
    let total = check_entries(&reader, limits)?;
    let output = destination(output, false, &[image])?;
    let parent = output
        .parent()
        .ok_or_else(|| Error::InvalidInput("missing extraction parent".into()))?;
    let staging = tempfile::tempdir_in(parent)?;
    let mut completed = 0;
    ctx.event(
        Phase::ApplyOverlay,
        0,
        Some(total),
        ProgressUnit::Bytes,
        ProgressState::Started,
    );
    let result = (|| {
        for (i, entry) in reader.entries().to_vec().iter().enumerate() {
            ctx.checkpoint()?;
            let target = staging.path().join(safe_path(&entry.name, limits)?);
            if entry.directory {
                fs::create_dir_all(target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = File::options().write(true).create_new(true).open(target)?;
            let mut writer = CheckedWriter {
                output: file,
                digest: Sha256::new(),
                ctx,
                completed: &mut completed,
                total,
                phase: Phase::ApplyOverlay,
            };
            reader.extract(i, &mut writer).map_err(optical)?;
            writer.output.sync_all()?;
        }
        check_snapshot(&original)?;
        ctx.checkpoint()?;
        publish_directory(staging.path(), &output)?;
        #[cfg(unix)]
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    ctx.event(
        Phase::ApplyOverlay,
        completed,
        Some(total),
        ProgressUnit::Bytes,
        match &result {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    result?;
    Ok(report)
}
#[derive(Clone)]
struct Snapshot {
    path: PathBuf,
    metadata: fs::Metadata,
    digest: [u8; 32],
    identity: Option<(u64, u64)>,
}
fn check_snapshot(snapshot: &Snapshot) -> Result<()> {
    unchanged(&snapshot.path, &snapshot.metadata)?;
    if crate::boot_media::plan::path_identity(&snapshot.path)? != snapshot.identity {
        return Err(Error::InvalidInput(format!(
            "input file identity changed: {}",
            snapshot.path.display()
        )));
    }
    Ok(())
}
fn hash_image(path: &Path, ctx: &mut OperationContext<'_>) -> Result<[u8; 32]> {
    let total = regular(path)?.len();
    let mut completed = 0u64;
    ctx.event(
        Phase::HashInputs,
        0,
        Some(total),
        ProgressUnit::Bytes,
        ProgressState::Started,
    );
    let mut silent = crate::boot_media::progress::NoProgress;
    let token = ctx.cancellation;
    let read_ctx = OperationContext {
        observer: &mut silent,
        cancellation: token,
        operation_id: ctx.operation_id,
    };
    let result = hash_file_with_progress(path, &read_ctx, &mut |n| {
        completed = completed
            .checked_add(n)
            .ok_or_else(|| Error::Resource("hash byte counter overflow".into()))?;
        ctx.event(
            Phase::HashInputs,
            completed,
            Some(total),
            ProgressUnit::Bytes,
            ProgressState::Advanced,
        );
        Ok(())
    });
    ctx.event(
        Phase::HashInputs,
        completed,
        Some(total),
        ProgressUnit::Bytes,
        match &result {
            Ok(_) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    result
}
fn hash_file_with_progress(
    path: &Path,
    ctx: &OperationContext<'_>,
    advanced: &mut impl FnMut(u64) -> Result<()>,
) -> Result<[u8; 32]> {
    let original = regular(path)?;
    let mut file = File::open(path)?;
    let identity = crate::boot_media::plan::file_identity(&file)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 65536];
    let mut completed = 0u64;
    while completed < original.len() {
        ctx.checkpoint()?;
        let amount = (original.len() - completed).min(bytes.len() as u64) as usize;
        let n = file.read(&mut bytes[..amount])?;
        if n == 0 {
            return Err(Error::InvalidInput("input shortened during hashing".into()));
        }
        digest.update(&bytes[..n]);
        completed = completed
            .checked_add(n as u64)
            .ok_or_else(|| Error::Resource("hash byte counter overflow".into()))?;
        advanced(n as u64)?;
    }
    unchanged(path, &original)?;
    if crate::boot_media::plan::path_identity(path)? != identity {
        return Err(Error::InvalidInput("input replaced during hashing".into()));
    }
    Ok(digest.finalize().into())
}
fn inventory(
    root: &Path,
    limits: &OperationLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<Vec<Snapshot>> {
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut snapshots = vec![Snapshot {
        path: root.to_owned(),
        metadata: fs::symlink_metadata(root)?,
        digest: [0; 32],
        identity: crate::boot_media::plan::path_identity(root)?,
    }];
    let mut count = 0usize;
    let mut size = 0u64;
    ctx.event(
        Phase::ScanSource,
        0,
        None,
        ProgressUnit::Entries,
        ProgressState::Started,
    );
    let scan_result = (|| {
        while let Some((dir, depth)) = pending.pop() {
            ctx.checkpoint()?;
            if depth > limits.max_depth {
                return Err(Error::Resource("source path depth".into()));
            }
            for entry in fs::read_dir(dir)? {
                ctx.checkpoint()?;
                let path = entry?.path();
                let metadata = fs::symlink_metadata(&path)?;
                count = count
                    .checked_add(1)
                    .ok_or_else(|| Error::Resource("source entry count".into()))?;
                if count > limits.max_entries {
                    return Err(Error::Resource("source entry count".into()));
                }
                if metadata.is_dir() {
                    pending.push((path.clone(), depth + 1));
                    snapshots.push(Snapshot {
                        identity: crate::boot_media::plan::path_identity(&path)?,
                        path,
                        metadata,
                        digest: [0; 32],
                    });
                } else if metadata.is_file() {
                    size = size
                        .checked_add(metadata.len())
                        .ok_or_else(|| Error::Resource("source size".into()))?;
                    if size > limits.max_output_bytes {
                        return Err(Error::Resource("source byte budget".into()));
                    }
                    let digest = [0; 32];
                    snapshots.push(Snapshot {
                        identity: crate::boot_media::plan::path_identity(&path)?,
                        path,
                        metadata,
                        digest,
                    });
                } else {
                    return Err(Error::Unsupported(format!(
                        "source symlink or special file: {}",
                        path.display()
                    )));
                }
                ctx.event(
                    Phase::ScanSource,
                    count as u64,
                    None,
                    ProgressUnit::Entries,
                    ProgressState::Advanced,
                );
            }
        }
        Ok(())
    })();
    ctx.event(
        Phase::ScanSource,
        count as u64,
        None,
        ProgressUnit::Entries,
        match &scan_result {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    scan_result?;
    ctx.event(
        Phase::HashInputs,
        0,
        Some(size),
        ProgressUnit::Bytes,
        ProgressState::Started,
    );
    let mut completed = 0u64;
    let result = (|| {
        for snapshot in &mut snapshots {
            if snapshot.metadata.is_dir() {
                continue;
            }
            let token = ctx.cancellation;
            let operation_id = ctx.operation_id;
            let mut sink = crate::boot_media::progress::NoProgress;
            let read_ctx = OperationContext {
                observer: &mut sink,
                cancellation: token,
                operation_id,
            };
            snapshot.digest = hash_file_with_progress(&snapshot.path, &read_ctx, &mut |n| {
                completed = completed
                    .checked_add(n)
                    .ok_or_else(|| Error::Resource("hash byte counter overflow".into()))?;
                ctx.event(
                    Phase::HashInputs,
                    completed,
                    Some(size),
                    ProgressUnit::Bytes,
                    ProgressState::Advanced,
                );
                Ok(())
            })?;
        }
        Ok(())
    })();
    ctx.event(
        Phase::HashInputs,
        completed,
        Some(size),
        ProgressUnit::Bytes,
        match &result {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    result?;
    Ok(snapshots)
}
/// Create a data ISO through the existing native writer, verify it, then atomically publish.
/// Boot and hybrid profiles require independent validation and are deliberately rejected here.
pub fn create(
    source: &Path,
    output: &Path,
    options: &CreateOptions,
    ctx: &mut OperationContext<'_>,
) -> Result<ImageReport> {
    ctx.checkpoint()?;
    if !fs::symlink_metadata(source)?.is_dir() {
        return Err(Error::InvalidInput("source must be a directory".into()));
    }
    if options.iso.hybrid.is_some()
        || options.iso.boot.bios.is_some()
        || options.iso.boot.efi.is_some()
        || !options.iso.advanced_boot.entries.is_empty()
    {
        return Err(Error::Unsupported(
            "boot/hybrid creation requires a validated profile".into(),
        ));
    }
    let output = destination(output, options.replace, &[source])?;
    let snapshots = inventory(source, &options.limits, ctx)?;
    if output.exists() {
        for snapshot in &snapshots {
            if crate::boot_media::plan::same_file(&output, &snapshot.path)? {
                return Err(Error::InvalidInput(
                    "output hard-links a source payload".into(),
                ));
            }
        }
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::InvalidInput("missing output parent".into()))?;
    let staging = tempfile::tempdir_in(parent)?;
    let image = staging.path().join("image.iso");
    let mut iso = options.iso.clone();
    iso.max_entries = iso.max_entries.min(options.limits.max_entries);
    iso.max_metadata_bytes = iso
        .max_metadata_bytes
        .min(options.limits.max_metadata_bytes);
    iso.max_image_bytes = iso.max_image_bytes.min(options.limits.max_output_bytes);
    let mut emitted = 0u64;
    let mut planned = None;
    let token = ctx.cancellation;
    let result = crate::write_iso9660_with_options_and_progress(
        source,
        &image,
        &iso,
        || {
            if token.is_cancelled() {
                Err(crate::iso9660::Error::Io(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "cancelled",
                )))
            } else {
                Ok(())
            }
        },
        |completed, total| {
            let state = if planned.is_none() {
                ProgressState::Started
            } else {
                ProgressState::Advanced
            };
            planned = Some(total);
            emitted = completed;
            ctx.event(
                Phase::EmitImage,
                completed,
                Some(total),
                ProgressUnit::Bytes,
                state,
            );
            if token.is_cancelled() {
                Err(crate::iso9660::Error::Io(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "cancelled",
                )))
            } else {
                Ok(())
            }
        },
    )
    .map_err(optical);
    if planned.is_some() {
        ctx.event(
            Phase::EmitImage,
            emitted,
            planned,
            ProgressUnit::Bytes,
            match &result {
                Ok(()) => ProgressState::Finished,
                Err(Error::Cancelled) => ProgressState::Cancelled,
                Err(_) => ProgressState::Failed,
            },
        );
    }
    result?;
    for snapshot in &snapshots {
        check_snapshot(snapshot)?;
    }
    // Re-index the source to catch added or removed inputs as well as modifications.
    let after = inventory(source, &options.limits, ctx)?;
    let before_paths: HashSet<_> = snapshots.iter().map(|s| &s.path).collect();
    let after_paths: HashSet<_> = after.iter().map(|s| &s.path).collect();
    let after_by_path: HashMap<_, _> = after
        .iter()
        .map(|snapshot| (&snapshot.path, snapshot))
        .collect();
    if before_paths != after_paths
        || snapshots.iter().any(|old| {
            after_by_path
                .get(&old.path)
                .is_some_and(|new| new.digest != old.digest || new.identity != old.identity)
        })
    {
        return Err(Error::InvalidInput(
            "source file set changed during creation".into(),
        ));
    }
    let mut report = verify(&image, &options.limits, ctx)?;
    ctx.event(
        Phase::Flush,
        0,
        Some(1),
        ProgressUnit::Operations,
        ProgressState::Started,
    );
    let flushed = (|| {
        // FlushFileBuffers on Windows requires a writable handle.
        File::options()
            .read(true)
            .write(true)
            .open(&image)?
            .sync_all()?;
        ctx.checkpoint()?;
        destination(&output, options.replace, &[source])?;
        if options.replace {
            fs::rename(&image, &output)?;
        } else {
            fs::hard_link(&image, &output)?;
        }
        #[cfg(unix)]
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    ctx.event(
        Phase::Flush,
        u64::from(flushed.is_ok()),
        Some(1),
        ProgressUnit::Operations,
        match &flushed {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    flushed?;
    report.image = output;
    Ok(report)
}
/// Repack a plain data image using bounded temporary staging and an additions/replacements overlay.
/// Source-backed extent reuse and preservation of Rock Ridge metadata are unavailable.
/// Supplied options explicitly define new timestamps, volume metadata and filesystem policy.
pub fn repack(
    image: &Path,
    overlay: &Path,
    output: &Path,
    options: &CreateOptions,
    policy: BootPolicy,
    ctx: &mut OperationContext<'_>,
) -> Result<ImageReport> {
    let (reader, report, original) = open(image, &options.limits, ctx)?;
    if report.bootable && policy != BootPolicy::Remove {
        return Err(Error::Unsupported(
            "boot preservation/rebuild has no validated relocation adapter".into(),
        ));
    }
    if policy == BootPolicy::Rebuild {
        return Err(Error::Unsupported(
            "boot rebuilding requires a validated profile".into(),
        ));
    }
    if reader
        .entries()
        .iter()
        .any(|entry| entry.unix.is_some() || entry.link_target.is_some())
    {
        return Err(Error::Unsupported(
            "metadata-preserving Rock Ridge repack is unavailable".into(),
        ));
    }
    if !fs::symlink_metadata(overlay)?.is_dir() {
        return Err(Error::InvalidInput("overlay must be a directory".into()));
    }
    let output = destination(output, options.replace, &[image, overlay])?;
    let parent = output
        .parent()
        .ok_or_else(|| Error::InvalidInput("missing output parent".into()))?;
    let temporary = tempfile::tempdir_in(parent)?;
    let tree = temporary.path().join("tree");
    let mut silent = crate::boot_media::progress::NoProgress;
    let mut staging_ctx = OperationContext {
        observer: &mut silent,
        cancellation: ctx.cancellation,
        operation_id: ctx.operation_id,
    };
    extract(image, &tree, &options.limits, &mut staging_ctx)?;
    let overlays = inventory(overlay, &options.limits, &mut staging_ctx)?;
    if output.exists() {
        for snapshot in &overlays {
            if crate::boot_media::plan::same_file(&output, &snapshot.path)? {
                return Err(Error::InvalidInput(
                    "output hard-links an overlay input".into(),
                ));
            }
        }
    }
    ctx.event(
        Phase::ApplyOverlay,
        0,
        Some(overlays.len() as u64),
        ProgressUnit::Entries,
        ProgressState::Started,
    );
    let mut overlay_completed = 0u64;
    let applied = (|| {
        for (i, snapshot) in overlays.iter().enumerate() {
            ctx.checkpoint()?;
            let relative = snapshot
                .path
                .strip_prefix(overlay)
                .map_err(|_| Error::InvalidInput("overlay path outside root".into()))?;
            let target = tree.join(relative);
            if snapshot.metadata.is_dir() {
                fs::create_dir_all(&target)?;
                overlay_completed = (i + 1) as u64;
                ctx.event(
                    Phase::ApplyOverlay,
                    (i + 1) as u64,
                    Some(overlays.len() as u64),
                    ProgressUnit::Entries,
                    ProgressState::Advanced,
                );
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if target.is_dir() {
                return Err(Error::InvalidInput(format!(
                    "overlay file conflicts with directory: {}",
                    relative.display()
                )));
            }
            let mut input = File::open(&snapshot.path)?;
            let mut file = File::create(&target)?;
            let mut digest = Sha256::new();
            let mut bytes = [0; 65536];
            let mut copied = 0u64;
            while copied < snapshot.metadata.len() {
                ctx.checkpoint()?;
                let amount = (snapshot.metadata.len() - copied).min(bytes.len() as u64) as usize;
                let n = input.read(&mut bytes[..amount])?;
                if n == 0 {
                    return Err(Error::InvalidInput("overlay shortened during copy".into()));
                }
                copied = copied
                    .checked_add(n as u64)
                    .ok_or_else(|| Error::Resource("overlay byte counter overflow".into()))?;
                file.write_all(&bytes[..n])?;
                digest.update(&bytes[..n]);
            }
            check_snapshot(snapshot)?;
            let actual: [u8; 32] = digest.finalize().into();
            if actual != snapshot.digest {
                return Err(Error::InvalidInput(
                    "overlay source changed during copying".into(),
                ));
            }
            overlay_completed = (i + 1) as u64;
            ctx.event(
                Phase::ApplyOverlay,
                (i + 1) as u64,
                Some(overlays.len() as u64),
                ProgressUnit::Entries,
                ProgressState::Advanced,
            );
        }
        Ok(())
    })();
    ctx.event(
        Phase::ApplyOverlay,
        overlay_completed,
        Some(overlays.len() as u64),
        ProgressUnit::Entries,
        match &applied {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    applied?;
    let staged_image = temporary.path().join("repacked.iso");
    let mut staged_options = options.clone();
    staged_options.replace = false;
    let mut built = create(&tree, &staged_image, &staged_options, ctx)?;
    ctx.event(
        Phase::Flush,
        0,
        Some(1),
        ProgressUnit::Operations,
        ProgressState::Started,
    );
    let published = (|| {
        check_snapshot(&original)?;
        ctx.checkpoint()?;
        destination(&output, options.replace, &[image, overlay])?;
        if options.replace {
            fs::rename(&staged_image, &output)?;
        } else {
            fs::hard_link(&staged_image, &output)?;
        }
        #[cfg(unix)]
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    ctx.event(
        Phase::Flush,
        u64::from(published.is_ok()),
        Some(1),
        ProgressUnit::Operations,
        match &published {
            Ok(()) => ProgressState::Finished,
            Err(Error::Cancelled) => ProgressState::Cancelled,
            Err(_) => ProgressState::Failed,
        },
    );
    published?;
    built.image = output;
    Ok(built)
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod publication_tests {
    use super::*;

    #[test]
    fn directory_publication_never_replaces_a_concurrent_destination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("staging");
        let destination = temp.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("payload"), b"staged").unwrap();
        fs::create_dir(&destination).unwrap();
        assert!(publish_directory(&source, &destination).is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert_eq!(fs::read(source.join("payload")).unwrap(), b"staged");
        fs::remove_dir(&destination).unwrap();
        fs::write(&destination, b"concurrent").unwrap();
        assert!(publish_directory(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"concurrent");
        assert_eq!(fs::read(source.join("payload")).unwrap(), b"staged");
    }
}
