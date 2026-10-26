//! Bounded, read-only input inventories. Plans disclose unavailable capabilities;
//! building must reject them and recheck all fingerprints before publication.
use crate::boot_media::{
    Error, Result,
    manifest::*,
    optical::OperationContext,
    progress::{CancellationToken, NoProgress, Phase, PhaseProgress, ProgressUnit},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

/// Inventory resource bounds.
#[derive(Debug, Clone, Copy)]
pub struct InventoryLimits {
    pub max_entries: usize,
    pub max_depth: usize,
    pub max_bytes: u64,
}
impl Default for InventoryLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_depth: 64,
            max_bytes: 1024 * 1024 * 1024 * 1024,
        }
    }
}
/// Content and metadata fingerprint for a regular source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputFingerprint {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
    pub kind: InputKind,
    pub metadata: MetadataFingerprint,
    pub identity: Option<(u64, u64)>,
}
/// Files and directories both affect an authored namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    File,
    Directory,
}
/// Identity and metadata whose changes invalidate a planned source tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataFingerprint {
    pub modified_nanos: u128,
    pub unix_identity: Option<(u64, u64)>,
    pub unix_mode: Option<u32>,
    pub unix_owner: Option<(u32, u32)>,
    pub unix_changed: Option<(i64, i64)>,
}
fn metadata_fingerprint(metadata: &fs::Metadata) -> Result<MetadataFingerprint> {
    let modified_nanos = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            Error::Unsupported("pre-epoch source metadata timestamps are not supported".into())
        })?
        .as_nanos();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(MetadataFingerprint {
            modified_nanos,
            unix_identity: Some((metadata.dev(), metadata.ino())),
            unix_mode: Some(metadata.mode()),
            unix_owner: Some((metadata.uid(), metadata.gid())),
            unix_changed: Some((metadata.ctime(), metadata.ctime_nsec())),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(MetadataFingerprint {
            modified_nanos,
            unix_identity: None,
            unix_mode: None,
            unix_owner: None,
            unix_changed: None,
        })
    }
}
/// Fully resolved single-image plan, schema version independent of TOML version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildPlan {
    pub schema_version: u32,
    pub manifest: PathBuf,
    pub source: PathBuf,
    pub output: PathBuf,
    pub label: String,
    pub filesystem: Filesystem,
    pub timestamp: Option<String>,
    pub joliet: bool,
    pub rock_ridge: bool,
    pub inputs: Vec<InputFingerprint>,
    pub payload_bytes: u64,
    /// Exact size is established by the optical layout scheduler during emission.
    pub image_bytes: Option<u64>,
    pub unsupported: Vec<String>,
}
/// Exact-input multiboot entry report, separate from firmware results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedEntry {
    pub id: String,
    pub title: String,
    pub input: InputFingerprint,
    pub destination: String,
    pub status: String,
}
/// Proposed multiboot inventory. No disk sectors are synthesized by this API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultibootPlan {
    pub schema_version: u32,
    pub manifest: PathBuf,
    pub target: MediaTarget,
    pub backend: String,
    pub assets: PathBuf,
    pub asset_inputs: Vec<InputFingerprint>,
    pub entries: Vec<PlannedEntry>,
    pub payload_bytes: u64,
    pub image_bytes: Option<u64>,
    pub unsupported: Vec<String>,
}

/// Resolve a destination through an existing parent without following a new leaf.
pub fn destination_path(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(fs::canonicalize(path)?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Error::InvalidInput(format!("invalid destination {}", path.display())))?;
    Ok(fs::canonicalize(parent)?.join(name))
}
/// Compare file identity as well as resolved paths, protecting hard-linked inputs.
pub fn same_file(first: &Path, second: &Path) -> Result<bool> {
    if destination_path(first)? == destination_path(second)? {
        return Ok(true);
    }
    if !first.exists() || !second.exists() {
        return Ok(false);
    }
    let a = path_identity(first)?;
    let b = path_identity(second)?;
    match (a, b) {
        (Some(a), Some(b)) => Ok(a == b),
        _ => Err(Error::Unsupported(
            "file-identity protection is unavailable on this platform".into(),
        )),
    }
}
/// Query the stable volume/file identity of an already-open handle.
pub fn file_identity(file: &fs::File) -> Result<Option<(u64, u64)>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(Some((metadata.dev(), metadata.ino())))
    }
    #[cfg(windows)]
    {
        windows_identity::identity(file)
            .map(Some)
            .map_err(Error::Io)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Ok(None)
    }
}
/// Query identity for a file or directory, following the supplied path.
pub fn path_identity(path: &Path) -> Result<Option<(u64, u64)>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_BACKUP_SEMANTICS allows the same identity query for directories.
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x02000000)
            .open(path)?;
        file_identity(&file)
    }
    #[cfg(not(windows))]
    {
        file_identity(&fs::File::open(path)?)
    }
}
#[cfg(windows)]
mod windows_identity {
    use std::{ffi::c_void, fs::File, io, os::windows::io::AsRawHandle};
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut c_void, information: *mut Information) -> i32;
    }
    pub(super) fn identity(file: &File) -> io::Result<(u64, u64)> {
        let mut information = Information::default();
        // SAFETY: The file owns a live handle and Information matches the Win32
        // BY_HANDLE_FILE_INFORMATION ABI with writable storage for every field.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((
            u64::from(information.volume),
            (u64::from(information.index_high) << 32) | u64::from(information.index_low),
        ))
    }
}
/// Hash a regular file with a bounded buffer and detect metadata changes.
pub fn fingerprint(path: &Path, max_bytes: u64) -> Result<InputFingerprint> {
    without_context(|ctx| fingerprint_with_context(path, max_bytes, ctx))
}
/// Hash with actual byte events and bounded cooperative cancellation checkpoints.
pub fn fingerprint_with_context(
    path: &Path,
    max_bytes: u64,
    ctx: &mut OperationContext<'_>,
) -> Result<InputFingerprint> {
    let path = fs::canonicalize(path)?;
    let mut file = fs::File::open(&path)?;
    let before = file.metadata()?;
    let before_metadata = metadata_fingerprint(&before)?;
    let identity = file_identity(&file)?;
    if !before.is_file() {
        return Err(Error::InvalidInput(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    if before.len() > max_bytes {
        return Err(Error::Resource(format!(
            "{} exceeds input byte limit",
            path.display()
        )));
    }
    checkpoint(ctx)?;
    let id = next_id(ctx)?;
    let token = ctx.cancellation.clone();
    let mut phase = PhaseProgress::start(
        ctx.observer,
        id,
        Phase::HashInputs,
        Some(before.len()),
        ProgressUnit::Bytes,
        None,
    );
    let result = (|| -> Result<InputFingerprint> {
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 128 * 1024];
        let mut size = 0_u64;
        loop {
            if token.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            size = size
                .checked_add(n as u64)
                .ok_or_else(|| Error::Resource("hash byte overflow".into()))?;
            if size > max_bytes {
                return Err(Error::Resource("input grew past byte limit".into()));
            }
            hash.update(&buffer[..n]);
            phase
                .advance(n as u64)
                .map_err(|e| Error::Resource(e.to_string()))?;
        }
        let after = file.metadata()?;
        if size != before.len()
            || before.len() != after.len()
            || before_metadata != metadata_fingerprint(&after)?
        {
            return Err(Error::Verification(format!(
                "input changed while hashing: {}",
                path.display()
            )));
        }
        // Compare the open handle to the path, which may have been replaced while read.
        {
            if identity != file_identity(&file)? || identity != path_identity(&path)? {
                return Err(Error::Verification(format!(
                    "input replaced: {}",
                    path.display()
                )));
            }
        }
        Ok(InputFingerprint {
            path,
            size,
            sha256: hex::encode(hash.finalize()),
            kind: InputKind::File,
            metadata: before_metadata,
            identity,
        })
    })();
    match result {
        Ok(input) => {
            phase.finish().map_err(|e| Error::Resource(e.to_string()))?;
            Ok(input)
        }
        Err(error) => {
            let _ = if matches!(error, Error::Cancelled) {
                phase.cancel()
            } else {
                phase.fail()
            };
            Err(error)
        }
    }
}
/// Inventory a tree in deterministic order; symlinks require a separate metadata inventory.
pub fn inventory(source: &Path, limits: InventoryLimits) -> Result<Vec<InputFingerprint>> {
    without_context(|ctx| inventory_with_context(source, limits, ctx))
}
/// Context-aware planning with cooperative cancellation during input hashing.
pub fn inventory_with_context(
    source: &Path,
    limits: InventoryLimits,
    ctx: &mut OperationContext<'_>,
) -> Result<Vec<InputFingerprint>> {
    checkpoint(ctx)?;
    let scan_id = next_id(ctx)?;
    let mut scanned = 0;
    scan_event(
        ctx,
        scan_id,
        scanned,
        crate::boot_media::progress::ProgressState::Started,
    );
    let result = (|| -> Result<Vec<InputFingerprint>> {
        let source = fs::canonicalize(source)?;
        if !source.is_dir() {
            return Err(Error::InvalidInput(format!(
                "{} is not a source directory",
                source.display()
            )));
        }
        let mut pending = vec![(source.clone(), 0_usize)];
        let mut files = Vec::new();
        let mut entries = 0_usize;
        let mut bytes = 0_u64;
        while let Some((directory, depth)) = pending.pop() {
            checkpoint(ctx)?;
            if depth > limits.max_depth {
                return Err(Error::Resource("source depth limit exceeded".into()));
            }
            let directory_metadata = fs::symlink_metadata(&directory)?;
            if !directory_metadata.is_dir()
                || fs::canonicalize(&directory)? != directory
                || !directory.starts_with(&source)
            {
                return Err(Error::Verification(
                    "source directory identity changed during inventory".into(),
                ));
            }
            let directory_metadata = metadata_fingerprint(&directory_metadata)?;
            let directory_identity = path_identity(&directory)?;
            files.push(InputFingerprint {
                path: directory.clone(),
                size: 0,
                sha256: String::new(),
                kind: InputKind::Directory,
                metadata: directory_metadata.clone(),
                identity: directory_identity,
            });
            let mut children = Vec::new();
            for child in fs::read_dir(&directory)? {
                checkpoint(ctx)?;
                entries = entries
                    .checked_add(1)
                    .ok_or_else(|| Error::Resource("entry count overflow".into()))?;
                if entries > limits.max_entries {
                    return Err(Error::Resource("source entry count limit exceeded".into()));
                }
                children.push(child?.path());
                scanned = entries as u64;
                scan_event(
                    ctx,
                    scan_id,
                    scanned,
                    crate::boot_media::progress::ProgressState::Advanced,
                );
            }
            children.sort();
            for path in children {
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() {
                    pending.push((path, depth + 1));
                } else if metadata.is_file() {
                    let identity = path_identity(&path)?;
                    let input = fingerprint_with_context(
                        &path,
                        limits.max_bytes.saturating_sub(bytes),
                        ctx,
                    )?;
                    if input.path != path
                        || metadata_fingerprint(&metadata)? != input.metadata
                        || input.identity != identity
                    {
                        return Err(Error::Verification(format!(
                            "input replaced during inventory: {}",
                            path.display()
                        )));
                    }
                    bytes = bytes
                        .checked_add(input.size)
                        .ok_or_else(|| Error::Resource("inventory byte overflow".into()))?;
                    files.push(input);
                } else {
                    return Err(Error::Unsupported(format!(
                        "planned inventory requires regular files/directories; symbolic or special input {}",
                        path.display()
                    )));
                }
            }
            if directory_metadata != metadata_fingerprint(&fs::symlink_metadata(&directory)?)?
                || directory_identity != path_identity(&directory)?
            {
                return Err(Error::Verification(
                    "source directory changed during inventory".into(),
                ));
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    })();
    let state = match &result {
        Ok(_) => crate::boot_media::progress::ProgressState::Finished,
        Err(Error::Cancelled) => crate::boot_media::progress::ProgressState::Cancelled,
        Err(_) => crate::boot_media::progress::ProgressState::Failed,
    };
    scan_event(ctx, scan_id, scanned, state);
    result
}
fn relative_to_manifest(manifest: &Path, path: &Path) -> PathBuf {
    manifest.parent().unwrap_or(Path::new(".")).join(path)
}
/// Plan single-image creation; does not create temporary media or run a VM.
pub fn plan_manifest(path: &Path) -> Result<BuildPlan> {
    without_context(|ctx| plan_manifest_with_context(path, ctx))
}
/// Context-aware planning with cooperative cancellation during input hashing.
pub fn plan_manifest_with_context(
    path: &Path,
    ctx: &mut OperationContext<'_>,
) -> Result<BuildPlan> {
    checkpoint(ctx)?;
    let path = fs::canonicalize(path)?;
    let manifest = read_manifest(&path)?;
    let source = fs::canonicalize(relative_to_manifest(&path, &manifest.image.source))?;
    let output = destination_path(&relative_to_manifest(&path, &manifest.image.output))?;
    if output.starts_with(&source) || same_file(&path, &output)? {
        return Err(Error::InvalidInput(
            "output overlaps source tree or manifest".into(),
        ));
    }
    let inputs = inventory_with_context(&source, InventoryLimits::default(), ctx)?;
    for input in &inputs {
        if same_file(&input.path, &output)? {
            return Err(Error::InvalidInput("output aliases a source file".into()));
        }
    }
    let payload_bytes = inputs.iter().try_fold(0_u64, |n, i| {
        n.checked_add(i.size)
            .ok_or_else(|| Error::Resource("payload overflow".into()))
    })?;
    let mut unsupported = Vec::new();
    if manifest.filesystem.kind != Filesystem::Iso9660 {
        unsupported.push(
            "single-image CLI currently supports ISO9660; UDF/bridge profile gates are pending"
                .into(),
        );
    }
    if manifest.filesystem.udf_revision.is_some() {
        unsupported.push("UDF revision requested without an enabled verified UDF profile".into());
    }
    if manifest.image.media != [MediaTarget::Optical] {
        unsupported
            .push("partitioned USB/hybrid profile is not enabled by optical creation".into());
    }
    if let Some(boot) = &manifest.boot {
        for entry in &boot.entries {
            let asset = fs::canonicalize(source.join(&entry.image))?;
            if !asset.starts_with(&source) || !asset.is_file() {
                return Err(Error::InvalidInput(format!(
                    "boot entry {} escapes source tree or is not a regular file",
                    entry.id
                )));
            }
        }
        unsupported.push(format!(
            "boot profile {} requires independent firmware/installer evidence and is not enabled",
            boot.profile
        ));
    }
    if let Some(windows) = &manifest.windows {
        for asset in [&windows.boot_wim, &windows.install_image] {
            let resolved = fs::canonicalize(source.join(asset))?;
            if !resolved.starts_with(&source) || !resolved.is_file() {
                return Err(Error::InvalidInput(format!(
                    "Windows payload {} must be a source-tree regular file",
                    asset.display()
                )));
            }
        }
        unsupported.push("Windows payload inspection/installation gates are not enabled".into());
    }
    let mut inputs = inputs;
    inputs.push(fingerprint_with_context(&path, 1024 * 1024, ctx)?);
    Ok(BuildPlan {
        schema_version: 1,
        manifest: path,
        source,
        output,
        label: manifest.image.label,
        filesystem: manifest.filesystem.kind,
        timestamp: manifest.reproducibility.map(|r| r.timestamp),
        joliet: manifest.filesystem.joliet,
        rock_ridge: manifest.filesystem.rock_ridge,
        inputs,
        payload_bytes,
        image_bytes: None,
        unsupported,
    })
}
/// Re-inventory a saved plan; catches added/removed files and changed content.
pub fn recheck_inputs(plan: &BuildPlan) -> Result<()> {
    without_context(|ctx| recheck_inputs_with_context(plan, ctx))
}
/// Context-aware planning with cooperative cancellation during input hashing.
pub fn recheck_inputs_with_context(plan: &BuildPlan, ctx: &mut OperationContext<'_>) -> Result<()> {
    checkpoint(ctx)?;
    if plan.schema_version != 1 {
        return Err(Error::InvalidInput("unsupported saved-plan schema".into()));
    }
    let current = plan_manifest_with_context(&plan.manifest, ctx)?;
    if current != *plan {
        return Err(Error::Verification(
            "saved-plan inputs or resolved settings changed".into(),
        ));
    }
    Ok(())
}
/// Inventory each original ISO separately and disclose unavailable provisioning.
pub fn plan_multiboot(path: &Path, target: MediaTarget) -> Result<MultibootPlan> {
    without_context(|ctx| plan_multiboot_with_context(path, target, ctx))
}
/// Context-aware planning with cooperative cancellation during input hashing.
pub fn plan_multiboot_with_context(
    path: &Path,
    target: MediaTarget,
    ctx: &mut OperationContext<'_>,
) -> Result<MultibootPlan> {
    checkpoint(ctx)?;
    let path = fs::canonicalize(path)?;
    let manifest = read_multiboot(&path)?;
    let assets = relative_to_manifest(&path, &manifest.boot.assets);
    let mut unsupported = Vec::new();
    let asset_inputs = if assets.exists() {
        inventory_with_context(&assets, InventoryLimits::default(), ctx)?
    } else {
        unsupported.push(format!(
            "required backend assets unavailable: {}",
            assets.display()
        ));
        Vec::new()
    };
    let mut entries = Vec::new();
    let mut payload_bytes = 0_u64;
    for entry in manifest.entries {
        let input = fingerprint_with_context(
            &relative_to_manifest(&path, &entry.image),
            InventoryLimits::default()
                .max_bytes
                .saturating_sub(payload_bytes),
            ctx,
        )?;
        payload_bytes = payload_bytes
            .checked_add(input.size)
            .ok_or_else(|| Error::Resource("multiboot capacity overflow".into()))?;
        let mut image = fs::File::open(&input.path)?;
        crate::iso9660::read_index(&mut image, crate::iso9660::Limits::default()).map_err(|e| {
            Error::InvalidInput(format!("entry {}: invalid ISO9660 image: {e}", entry.id))
        })?;
        entries.push(PlannedEntry {
            destination: format!("/isos/{}.iso", entry.id),
            id: entry.id,
            title: entry.title,
            input,
            status: "untested".into(),
        });
    }
    let mut image_bytes = None;
    if target == MediaTarget::Usb && manifest.boot.backend == "ventoy" {
        match crate::boot_media::ventoy::audit_assets(&assets) {
            Ok(_) => {
                image_bytes = Some(crate::boot_media::ventoy::minimum_image_bytes(
                    payload_bytes,
                    entries.len(),
                )?);
                #[cfg(not(target_os = "linux"))]
                unsupported.push("Ventoy image provisioning currently requires Linux root with disposable loop-device and mount access".into());
            }
            Err(error) => unsupported.push(format!("Ventoy assets: {error}")),
        }
    } else if target == MediaTarget::Usb {
        unsupported.push("USB multiboot provisioning requires the pinned ventoy backend".into());
    } else {
        unsupported.push("optical multiboot requires independently verified per-system root/payload-discovery adapters; none are enabled".into());
    }
    if entries.is_empty() {
        unsupported.push("no multiboot ISO entries supplied".into());
    }
    Ok(MultibootPlan {
        schema_version: 1,
        manifest: path,
        target,
        backend: manifest.boot.backend,
        assets,
        asset_inputs,
        entries,
        payload_bytes,
        image_bytes,
        unsupported,
    })
}

fn without_context<T>(operation: impl FnOnce(&mut OperationContext<'_>) -> Result<T>) -> Result<T> {
    let mut observer = NoProgress;
    let cancellation = CancellationToken::default();
    operation(&mut OperationContext {
        observer: &mut observer,
        cancellation: &cancellation,
        operation_id: 0,
    })
}
fn checkpoint(ctx: &OperationContext<'_>) -> Result<()> {
    if ctx.cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn next_id(ctx: &mut OperationContext<'_>) -> Result<u64> {
    ctx.operation_id = ctx
        .operation_id
        .checked_add(1)
        .ok_or_else(|| Error::Resource("operation identity overflow".into()))?;
    Ok(ctx.operation_id)
}

fn scan_event(
    ctx: &mut OperationContext<'_>,
    id: u64,
    completed: u64,
    state: crate::boot_media::progress::ProgressState,
) {
    ctx.observer
        .observe(&crate::boot_media::progress::ProgressEvent {
            operation_id: id,
            phase: Phase::ScanSource,
            completed,
            total: None,
            unit: ProgressUnit::Entries,
            entry_id: None,
            state,
        });
}
