//! Stage explicitly supplied GRUB inputs without asserting boot compatibility.
use libmkiso::boot_media::{
    Error, Result,
    optical::OperationContext,
    progress::{Phase, PhaseProgress, ProgressUnit},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub fn prepare(
    command: &crate::args::BootCommand,
    ctx: &mut OperationContext<'_>,
) -> Result<Value> {
    let crate::args::BootCommand::Prepare {
        loader,
        arch,
        kernel,
        initrd: initrds,
        cmdline,
        assets,
        output,
    } = command;
    let cmdline = cmdline.as_deref();
    ctx.cancellation
        .checkpoint()
        .map_err(|_| Error::Cancelled)?;
    if loader != "grub" || !matches!(arch.as_str(), "x86_64" | "arm64") {
        return Err(Error::Unsupported(
            "boot prepare requires --loader grub and --arch x86_64|arm64".into(),
        ));
    }
    if initrds.is_empty() {
        return Err(Error::InvalidInput("supply at least one --initrd; root discovery remains the supplied initramfs responsibility".into()));
    }
    if cmdline.is_some_and(|s| {
        s.bytes()
            .any(|c| c < 32 || matches!(c, b';' | b'{' | b'}' | b'\'' | b'"' | b'\\' | b'$' | b'`'))
    }) {
        return Err(Error::InvalidInput(
            "kernel command line contains GRUB scripting/control characters".into(),
        ));
    }
    if output.exists() {
        return Err(Error::InvalidInput(
            "prepared output must be a new directory".into(),
        ));
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent.canonicalize()?.starts_with(assets.canonicalize()?) {
        return Err(Error::InvalidInput(
            "prepared output cannot be inside the asset bundle".into(),
        ));
    }
    let stage = tempfile::tempdir_in(parent)?;
    let mut inventory = Vec::new();
    let mut bytes = 0u64;
    copy_tree(
        assets,
        assets,
        &stage.path().join("assets"),
        &mut inventory,
        &mut bytes,
        0,
        ctx,
    )?;
    let loader_name = if arch == "x86_64" {
        "bootx64.efi"
    } else {
        "bootaa64.efi"
    };
    if !inventory
        .iter()
        .any(|p: &String| p.to_ascii_lowercase().ends_with(loader_name))
    {
        return Err(Error::InvalidInput(format!(
            "assets must contain explicit architecture loader {loader_name}; signed bytes are preserved"
        )));
    }
    fs::create_dir_all(stage.path().join("boot"))?;
    copy_regular(kernel, &stage.path().join("boot/kernel"), &mut bytes, ctx)?;
    let mut initrd_paths = Vec::new();
    for (i, input) in initrds.iter().enumerate() {
        let path = format!("boot/initrd-{i}");
        copy_regular(input, &stage.path().join(&path), &mut bytes, ctx)?;
        initrd_paths.push(format!("/{path}"));
    }
    fs::create_dir_all(stage.path().join("boot/grub"))?;
    let config = format!(
        "set timeout=5\nmenuentry 'Prepared Linux' {{\n  linux /boot/kernel {}\n  initrd {}\n}}\n",
        cmdline.unwrap_or(""),
        initrd_paths.join(" ")
    );
    let mut file = fs::File::create(stage.path().join("boot/grub/grub.cfg"))?;
    file.write_all(config.as_bytes())?;
    file.sync_all()?;
    // Windows cannot rename the staging directory while a child is open.
    drop(file);
    let mut fingerprints = libmkiso::boot_media::plan::inventory_with_context(
        stage.path(),
        libmkiso::boot_media::plan::InventoryLimits::default(),
        ctx,
    )?;
    let stage_root = stage.path().canonicalize()?;
    for fingerprint in &mut fingerprints {
        fingerprint.path = output.join(
            fingerprint
                .path
                .strip_prefix(&stage_root)
                .map_err(|e| Error::InvalidInput(e.to_string()))?,
        );
    }
    ctx.cancellation
        .checkpoint()
        .map_err(|_| Error::Cancelled)?;
    #[cfg(unix)]
    fs::File::open(stage.path())?.sync_all()?;
    fs::rename(stage.path(), output)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(
        json!({"output":output,"requested_architecture":arch,"asset_architecture_evidence":"filename_only","loader":"grub","assets":inventory,"fingerprints":fingerprints,"payload_bytes":bytes,"firmware_status":"untested","root_discovery":"provided_initramfs","configuration":"boot/grub/grub.cfg"}),
    )
}
fn copy_regular(
    input: &Path,
    output: &Path,
    bytes: &mut u64,
    ctx: &mut OperationContext<'_>,
) -> Result<()> {
    ctx.cancellation
        .checkpoint()
        .map_err(|_| Error::Cancelled)?;
    let metadata = fs::symlink_metadata(input)?;
    if !metadata.is_file() {
        return Err(Error::InvalidInput(format!(
            "{} must be a regular file, without symlinks",
            input.display()
        )));
    }
    *bytes = bytes
        .checked_add(metadata.len())
        .ok_or_else(|| Error::Resource("boot asset size overflow".into()))?;
    if *bytes > 16 * 1024 * 1024 * 1024 {
        return Err(Error::Resource("prepared assets exceed 16 GiB".into()));
    }
    let before =
        libmkiso::boot_media::plan::fingerprint_with_context(input, 16 * 1024 * 1024 * 1024, ctx)?;
    let mut source = fs::File::open(input)?;
    let mut target = fs::File::create(output)?;
    let mut phase = PhaseProgress::start(
        ctx.observer,
        ctx.operation_id,
        Phase::ApplyOverlay,
        Some(metadata.len()),
        ProgressUnit::Bytes,
        Some(input.display().to_string()),
    );
    let mut left = metadata.len();
    let mut buffer = [0u8; 64 * 1024];
    while left > 0 {
        if ctx.cancellation.is_cancelled() {
            let _ = phase.cancel();
            return Err(Error::Cancelled);
        }
        let count = source.read(&mut buffer[..left.min(64 * 1024) as usize])?;
        if count == 0 {
            let _ = phase.fail();
            return Err(Error::Verification(
                "boot input shrank during copying".into(),
            ));
        }
        target.write_all(&buffer[..count])?;
        left -= count as u64;
        phase
            .advance(count as u64)
            .map_err(|e| Error::Resource(e.to_string()))?;
    }
    if source.read(&mut buffer[..1])? != 0 {
        let _ = phase.fail();
        return Err(Error::Verification("boot input grew during copying".into()));
    }
    target.sync_all()?;
    if libmkiso::boot_media::plan::fingerprint(input, 16 * 1024 * 1024 * 1024)? != before
        || libmkiso::boot_media::plan::fingerprint(output, 16 * 1024 * 1024 * 1024)?.sha256
            != before.sha256
    {
        let _ = phase.fail();
        return Err(Error::Verification(
            "boot input changed while staging".into(),
        ));
    }
    phase.finish().map_err(|e| Error::Resource(e.to_string()))?;
    #[cfg(unix)]
    fs::File::open(output.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}
fn copy_tree(
    root: &Path,
    dir: &Path,
    target: &Path,
    inventory: &mut Vec<String>,
    bytes: &mut u64,
    depth: usize,
    ctx: &mut OperationContext<'_>,
) -> Result<()> {
    ctx.cancellation
        .checkpoint()
        .map_err(|_| Error::Cancelled)?;
    if depth > 32 || inventory.len() > 10000 {
        return Err(Error::Resource(
            "boot asset tree exceeds depth/count limits".into(),
        ));
    }
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(Error::InvalidInput(
            "boot assets must be a directory without symlinks".into(),
        ));
    }
    fs::create_dir_all(target)?;
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let to = target.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(root, &path, &to, inventory, bytes, depth + 1, ctx)?;
        } else {
            copy_regular(&path, &to, bytes, ctx)?;
            inventory.push(
                path.strip_prefix(root)
                    .map_err(|e| Error::InvalidInput(e.to_string()))?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    Ok(())
}
