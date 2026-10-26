mod args;
mod boot;
mod firmware;
mod render;

use args::*;
use clap::Parser;
use libmkiso::boot_media::{
    Error, Result,
    optical::{self, BootPolicy, CreateOptions, OperationContext, OperationLimits},
    progress::CancellationToken,
};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            if error.use_stderr() && std::env::args_os().any(|arg| arg == "--json") {
                println!(
                    "{}",
                    json!({"schema_version":1,"status":"error","error":{"code":"usage","message":error.to_string()}})
                );
            }
            let _ = error.print();
            std::process::exit(error.exit_code());
        }
    };
    let report_safe = cli
        .report
        .as_ref()
        .is_none_or(|report| protect_report(report, &cli.command).is_ok());
    let result = execute(&cli);
    let (mut value, mut code) = match result {
        Ok(value) => (json!({"schema_version":1,"status":"ok","result":value}), 0),
        Err(error) => {
            eprintln!("{error}");
            (
                json!({"schema_version":1,"status":"error","error":{"code":error.code(),"message":error.to_string()}}),
                error.exit_code(),
            )
        }
    };
    // Reports are preflighted before any media operation; publish a new file only.
    if let Some(report) = cli.report.as_ref().filter(|_| report_safe)
        && let Err(error) = save_report(report, &value)
    {
        eprintln!("report: {error}");
        code = error.exit_code();
        value = json!({"schema_version":1,"status":"error","error":{"code":error.code(),"message":error.to_string()}});
    }
    if cli.json {
        println!("{value}");
    } else if code == 0 {
        println!(
            "{}",
            serde_json::to_string_pretty(&value["result"]).unwrap_or_else(|_| value.to_string())
        );
    }
    std::process::exit(code);
}

fn execute(cli: &Cli) -> Result<Value> {
    #[cfg(not(feature = "progress"))]
    if matches!(cli.progress, ProgressMode::Always) {
        return Err(Error::Usage(
            "--progress always requires rebuilding libmkiso with --features progress".into(),
        ));
    }
    if let Some(report) = &cli.report {
        protect_report(report, &cli.command)?;
    }
    let token = CancellationToken::default();
    let interrupt = token.clone();
    ctrlc::set_handler(move || interrupt.cancel())
        .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
    let mut renderer = render::Renderer::new(cli.progress, cli.json);
    let mut ctx = OperationContext {
        observer: &mut renderer,
        cancellation: &token,
        operation_id: 1,
    };
    let limits = OperationLimits::default();
    match &cli.command {
        Command::Create(create) => {
            if create.profile.is_some() || !create.boot.is_empty() || create.boot_assets.is_some() || create.media.as_deref().is_some_and(|v| v != "optical") || create.partition_table.is_some() || create.arch.is_some() {
                validate_boots(&create.boot)?;
                return Err(Error::Unsupported("boot profiles and hybrid layouts require validated boot assets and firmware gates; use data ISO creation or a prepared optical manifest".into()));
            }
            if create.filesystem != "iso9660" { return Err(Error::Unsupported("create currently supports --filesystem iso9660; UDF profile gates remain pending".into())); }
            let options = options(cli, &create.label, create.joliet, create.rock_ridge)?;
            value(optical::create(&create.root, &create.output, &options, &mut ctx)?)
        }
        Command::Plan { manifest } => value(load_plan(manifest, &mut ctx)?),
        Command::Build { manifest, output } => {
            let plan = load_plan(manifest, &mut ctx)?;
            if !plan.unsupported.is_empty() { return Err(Error::Unsupported(plan.unsupported.join("; "))); }
            libmkiso::boot_media::plan::recheck_inputs_with_context(&plan, &mut ctx)?;
            let destination = output.as_deref().unwrap_or(&plan.output);
            for protected in [manifest.as_path(), plan.manifest.as_path()] {
                if libmkiso::boot_media::plan::same_file(destination, protected)? || libmkiso::boot_media::plan::destination_path(destination)? == libmkiso::boot_media::plan::destination_path(protected)? { return Err(Error::InvalidInput("build output aliases the input manifest or saved plan".into())); }
            }

            if cli.reproducible && cli.timestamp.is_none() && plan.timestamp.is_none() { return Err(Error::InvalidInput("reproducible build requires a timestamp".into())); }
            let mut options = CreateOptions { replace: cli.replace, ..CreateOptions::default() };
            options.iso.volume_label = plan.label.clone(); options.iso.joliet = plan.joliet; options.iso.rock_ridge = plan.rock_ridge;
            if let Some(timestamp) = cli.timestamp.as_ref().or(plan.timestamp.as_ref()) { options.iso.timestamp = parse_timestamp(timestamp)?; }

            value(optical::create(&plan.source, output.as_deref().unwrap_or(&plan.output), &options, &mut ctx)?)
        }
        Command::Inspect { image } => value(optical::inspect(image, &limits, &mut ctx)?),
        Command::Verify { image, manifest } => {
            let report = optical::verify(image, &limits, &mut ctx)?;
            if let Some(manifest) = manifest {
                let plan = load_plan(manifest, &mut ctx)?;
                if !plan.joliet && !plan.rock_ridge { return Err(Error::Unsupported("manifest payload comparison requires Joliet or Rock Ridge to retain source names".into())); }
                let payloads: std::collections::BTreeMap<_,_> = report.entries.iter().filter(|e| !e.directory).map(|e| (e.path.trim_start_matches('/'), e)).collect();
                let inputs: Vec<_> = plan.inputs.iter().filter(|input| input.kind == libmkiso::boot_media::plan::InputKind::File && input.path.starts_with(&plan.source)).collect();
                if payloads.len() != inputs.len() { return Err(Error::Verification("manifest payload count differs from image".into())); }
                for input in inputs {
                    let relative = input.path.strip_prefix(&plan.source).map_err(|e| Error::InvalidInput(e.to_string()))?.to_string_lossy().replace('\\', "/");
                    let entry = payloads.get(relative.as_str()).ok_or_else(|| Error::Verification(format!("missing manifest payload {relative}")))?;
                    if entry.size != input.size || entry.sha256.as_ref() != Some(&input.sha256) { return Err(Error::Verification(format!("payload differs from manifest: {relative}"))); }
                }
            }
            value(report)
        }
        Command::Extract { image, output } => value(optical::extract(image, output, &limits, &mut ctx)?),
        Command::Repack { image, overlay, output, boot } => {
            let policy = match boot.as_deref() {
                Some("remove") => BootPolicy::Remove, Some("preserve") => BootPolicy::Preserve, Some("rebuild") => BootPolicy::Rebuild,
                Some(_) => return Err(Error::InvalidInput("--boot must be preserve, rebuild or remove".into())),
                None => { if optical::inspect(image, &limits, &mut ctx)?.bootable { return Err(Error::InvalidInput("bootable repacking requires explicit --boot preserve|rebuild|remove".into())); } BootPolicy::Remove }
            };
            value(optical::repack(image, overlay, output, &options(cli, "MKISO", true, false)?, policy, &mut ctx)?)
        }
        Command::Multiboot { command: MultibootCommand::Plan { manifest, target } } => value(libmkiso::boot_media::plan::plan_multiboot_with_context(manifest, target_type(target)?, &mut ctx)?),
        Command::Multiboot { command: MultibootCommand::Build { manifest, target, output, size, partition_table, data_filesystem, backend } } => {
            if target_type(target)? != libmkiso::boot_media::manifest::MediaTarget::Usb { return Err(Error::Unsupported("optical multiboot adapters remain unavailable; Ventoy requires a USB disk image".into())); }
            if backend.as_deref().is_some_and(|backend| backend != "ventoy") || data_filesystem.as_deref().is_some_and(|filesystem| filesystem != "exfat") { return Err(Error::Unsupported("this USB provisioning route requires ventoy and exfat".into())); }
            if cli.reproducible || cli.timestamp.is_some() { return Err(Error::Unsupported("the official Ventoy installer/exFAT tooling does not establish reproducible disk-image GUID/serial/timestamp output".into())); }
            libmkiso::boot_media::ventoy::provisioning_environment()?;
            let plan = libmkiso::boot_media::plan::plan_multiboot_with_context(manifest, libmkiso::boot_media::manifest::MediaTarget::Usb, &mut ctx)?;
            let options = libmkiso::boot_media::ventoy::BuildOptions { image_bytes: size.as_deref().map(parse_image_size).transpose()?.unwrap_or(libmkiso::boot_media::ventoy::minimum_image_bytes(plan.payload_bytes, plan.entries.len())?), partition_table: partition_kind(partition_table.as_deref().unwrap_or("gpt"))?, replace: cli.replace };
            value(libmkiso::boot_media::ventoy::build(&plan, output, &options, &mut ctx)?)
        }
        Command::Multiboot { command: MultibootCommand::Verify { image, manifest } } => {
            let manifest = manifest.as_ref().ok_or_else(|| Error::Usage("multiboot verify requires --manifest to bind original ISO hashes and backend assets".into()))?;
            libmkiso::boot_media::ventoy::provisioning_environment()?;
            let plan = libmkiso::boot_media::plan::plan_multiboot_with_context(manifest, libmkiso::boot_media::manifest::MediaTarget::Usb, &mut ctx)?;
            value(libmkiso::boot_media::ventoy::verify(&plan, image, &mut ctx)?)
        }
        Command::Multiboot { command: MultibootCommand::Test { image, firmware: firmware_kind, evidence_dir, entry, backend, bios, ovmf_code, ovmf_vars, swtpm, key, timeout, select_after, memory, accel } } => {
            firmware::run(image, &firmware::Settings { firmware: firmware_kind.clone(), evidence_dir: evidence_dir.clone(), entry: entry.clone(), backend: backend.clone(), bios: bios.clone(), ovmf_code: ovmf_code.clone(), ovmf_vars: ovmf_vars.clone(), swtpm: swtpm.clone(), keys: key.clone(), timeout: *timeout, select_after: *select_after, memory: *memory, accel: accel.clone() }, &mut ctx)
        }
        Command::Multiboot { command: MultibootCommand::Init { manifest } } => {
            let text = "version = 1\n\n[menu]\ntitle = \"Installation and recovery\"\ntimeout_seconds = 15\n\n[boot]\nbackend = \"ventoy\"\nfirmware = [\"bios\", \"uefi\"]\nassets = \"./ventoy-assets\"\n\nentries = []\n";
            // entries must be top-level, before table declarations.
            let text = text.replace("version = 1\n", "version = 1\nentries = []\n").replace("\n\nentries = []\n", "\n");
            write_new(manifest, text.as_bytes())?;
            Ok(json!({"manifest":manifest}))
        }
        Command::Multiboot { command: MultibootCommand::Add { manifest, image, id, title } } => add_entry(manifest, image, id, title),
        Command::Boot { command } => boot::prepare(command, &mut ctx),
        Command::Disk { command: DiskCommand::Inspect { device } } => value(libmkiso::boot_media::device::disk_inspect(device)?),
        Command::Disk { command: DiskCommand::Write { image, device, erase, expect_device_id, verify } } => {
            let verification = match verify.as_deref().unwrap_or("none") {
                "none" => libmkiso::boot_media::device::DeviceVerification::None,
                "full" => libmkiso::boot_media::device::DeviceVerification::Full,
                _ => return Err(Error::InvalidInput("--verify must be none or full".into())),
            };
            let expected = expect_device_id.as_ref().ok_or_else(|| Error::InvalidInput("device writing requires --erase and --expect-device-id from disk inspect".into()))?;
            let options = libmkiso::boot_media::device::DiskWriteOptions { erase: *erase, expect_device_id: expected.clone(), verify: verification };
            value(libmkiso::boot_media::device::disk_write(image, device, &options, &mut renderer, &token)?)
        }
        _ => Err(Error::Unsupported("this boot, USB, device or firmware-test backend is unavailable; no media was written. Supply and validate a pinned backend provisioning route before enabling this capability".into())),
    }
}
fn value(value: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| Error::InvalidInput(e.to_string()))
}
fn options(cli: &Cli, label: &str, joliet: bool, rock_ridge: bool) -> Result<CreateOptions> {
    let mut options = CreateOptions {
        replace: cli.replace,
        ..CreateOptions::default()
    };
    options.iso.volume_label = label.into();
    options.iso.joliet = joliet;
    options.iso.rock_ridge = rock_ridge;
    if cli.reproducible && cli.timestamp.is_none() {
        return Err(Error::InvalidInput(
            "--reproducible requires --timestamp (or manifest reproducibility.timestamp)".into(),
        ));
    }
    if let Some(timestamp) = &cli.timestamp {
        options.iso.timestamp = parse_timestamp(timestamp)?;
    }
    Ok(options)
}
fn parse_timestamp(text: &str) -> Result<libmkiso::IsoTimestamp> {
    let t = libmkiso::boot_media::manifest::validate_timestamp(text)?;
    Ok(libmkiso::IsoTimestamp {
        year: t.year() as u16,
        month: u8::from(t.month()),
        day: t.day(),
        hour: t.hour(),
        minute: t.minute(),
        second: t.second(),
    })
}
fn read_saved_plan(path: &Path) -> Result<libmkiso::boot_media::plan::BuildPlan> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(Error::Resource("saved plan exceeds 16 MiB".into()));
    }
    let doc: Value =
        serde_json::from_reader(file).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let plan: libmkiso::boot_media::plan::BuildPlan =
        serde_json::from_value(doc.get("result").cloned().unwrap_or(doc))
            .map_err(|e| Error::InvalidInput(e.to_string()))?;
    if plan.schema_version != 1 {
        return Err(Error::InvalidInput(
            "unsupported saved plan schema version".into(),
        ));
    }
    Ok(plan)
}
fn load_plan(
    path: &Path,
    ctx: &mut OperationContext<'_>,
) -> Result<libmkiso::boot_media::plan::BuildPlan> {
    if path.extension().is_some_and(|e| e == "json") {
        let plan = read_saved_plan(path)?;
        libmkiso::boot_media::plan::recheck_inputs_with_context(&plan, ctx)?;
        Ok(plan)
    } else {
        libmkiso::boot_media::plan::plan_manifest_with_context(path, ctx)
    }
}
fn planned_paths(path: &Path) -> Result<(PathBuf, PathBuf)> {
    if path.extension().is_some_and(|e| e == "json") {
        let plan = read_saved_plan(path)?;
        Ok((plan.source, plan.output))
    } else {
        let manifest = libmkiso::boot_media::manifest::read_manifest(path)?;
        let base = path
            .canonicalize()?
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();
        Ok((
            base.join(manifest.image.source),
            base.join(manifest.image.output),
        ))
    }
}
fn validate_boots(boots: &[String]) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    for boot in boots {
        let (id, path) = boot
            .split_once('=')
            .ok_or_else(|| Error::InvalidInput("--boot requires KIND=PATH".into()))?;
        if id.is_empty() || path.is_empty() || !ids.insert(id) {
            return Err(Error::InvalidInput(
                "boot entry identifiers must be nonempty and distinct".into(),
            ));
        }
    }
    Ok(())
}
fn target_type(text: &str) -> Result<libmkiso::boot_media::manifest::MediaTarget> {
    match text {
        "usb" => Ok(libmkiso::boot_media::manifest::MediaTarget::Usb),
        "optical" => Ok(libmkiso::boot_media::manifest::MediaTarget::Optical),
        _ => Err(Error::InvalidInput("target must be usb or optical".into())),
    }
}
fn partition_kind(value: &str) -> Result<libmkiso::boot_media::ventoy::PartitionTable> {
    match value {
        "mbr" => Ok(libmkiso::boot_media::ventoy::PartitionTable::Mbr),
        "gpt" => Ok(libmkiso::boot_media::ventoy::PartitionTable::Gpt),
        _ => Err(Error::Usage("partition-table must be mbr or gpt".into())),
    }
}
fn parse_image_size(value: &str) -> Result<u64> {
    let (digits, multiplier) = [
        ("GiB", 1_u64 << 30),
        ("MiB", 1_u64 << 20),
        ("KiB", 1_u64 << 10),
        ("B", 1_u64),
    ]
    .iter()
    .find_map(|(suffix, multiplier)| {
        value
            .strip_suffix(suffix)
            .map(|digits| (digits, *multiplier))
    })
    .unwrap_or((value, 1));
    digits
        .parse::<u64>()
        .ok()
        .and_then(|bytes| bytes.checked_mul(multiplier))
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| {
            Error::Usage("size requires positive bytes or KiB/MiB/GiB, without overflow".into())
        })
}
fn add_entry(manifest: &Path, image: &Path, id: &str, title: &str) -> Result<Value> {
    let mut validated = libmkiso::boot_media::manifest::read_multiboot(manifest)?;
    validated
        .entries
        .push(libmkiso::boot_media::manifest::MultibootEntry {
            id: id.into(),
            title: title.into(),
            image: image.into(),
        });
    validated.validate()?;
    if id.is_empty() || title.is_empty() {
        return Err(Error::InvalidInput(
            "entry id and title must be nonempty".into(),
        ));
    }
    let original = std::fs::read_to_string(manifest)?;
    let mut doc: toml::Value =
        toml::from_str(&original).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let table = doc
        .as_table_mut()
        .ok_or_else(|| Error::InvalidInput("manifest must be a table".into()))?;
    let entries = table
        .entry("entries")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| Error::InvalidInput("entries must be an array".into()))?;
    if entries
        .iter()
        .any(|e| e.get("id").and_then(toml::Value::as_str) == Some(id))
    {
        return Err(Error::InvalidInput(format!("duplicate entry id {id}")));
    }
    let image = image.canonicalize()?;
    let mut entry = toml::map::Map::new();
    entry.insert("id".into(), id.into());
    entry.insert("title".into(), title.into());
    entry.insert("image".into(), image.to_string_lossy().as_ref().into());
    entries.push(toml::Value::Table(entry));
    let text = toml::to_string_pretty(&doc).map_err(|e| Error::InvalidInput(e.to_string()))?;
    let mut temp = tempfile::NamedTempFile::new_in(manifest.parent().unwrap_or(Path::new(".")))?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(manifest).map_err(|e| Error::Io(e.error))?;
    Ok(json!({"manifest":manifest,"entry":id}))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn save_report(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| Error::InvalidInput(e.to_string()))?;
    write_new(path, &text)
}
fn protect_report(report: &Path, command: &Command) -> Result<()> {
    if report.exists() {
        return Err(Error::InvalidInput(
            "report must be a new file; existing report paths are protected".into(),
        ));
    }
    let resolved = resolve_new(report)?;
    let mut paths: Vec<&Path> = Vec::new();
    if let Command::Build { manifest, .. } = command {
        let (source, output) = planned_paths(manifest)?;
        if resolved.starts_with(source.canonicalize()?) || resolved == resolve_new(&output)? {
            return Err(Error::InvalidInput(
                "report overlaps build source or manifest output".into(),
            ));
        }
    }
    match command {
        Command::Create(c) => {
            paths.extend([c.root.as_path(), c.output.as_path()]);
            if let Some(p) = &c.boot_assets {
                paths.push(p);
            }
        }
        Command::Build { manifest, output } => {
            paths.push(manifest);
            if let Some(p) = output {
                paths.push(p);
            } else {
                let (_, planned_output) = planned_paths(manifest)?;
                if resolved == resolve_new(&planned_output)? {
                    return Err(Error::InvalidInput(
                        "report collides with manifest output".into(),
                    ));
                }
            }
        }
        Command::Plan { manifest } => paths.push(manifest),
        Command::Inspect { image } | Command::Verify { image, .. } => paths.push(image),
        Command::Extract { image, output } => paths.extend([image.as_path(), output.as_path()]),
        Command::Repack {
            image,
            overlay,
            output,
            ..
        } => paths.extend([image.as_path(), overlay.as_path(), output.as_path()]),
        Command::Multiboot {
            command:
                MultibootCommand::Init { manifest }
                | MultibootCommand::Add { manifest, .. }
                | MultibootCommand::Plan { manifest, .. }
                | MultibootCommand::Build { manifest, .. },
        } => paths.push(manifest),
        _ => {}
    }
    match command {
        Command::Verify {
            manifest: Some(manifest),
            ..
        } => paths.push(manifest),
        Command::Boot {
            command:
                BootCommand::Prepare {
                    kernel,
                    initrd,
                    assets,
                    output,
                    ..
                },
        } => {
            paths.extend([kernel.as_path(), assets.as_path(), output.as_path()]);
            paths.extend(initrd.iter().map(PathBuf::as_path));
        }
        Command::Disk {
            command: DiskCommand::Inspect { device },
        } => paths.push(device),
        Command::Disk {
            command: DiskCommand::Write { image, device, .. },
        } => paths.extend([image.as_path(), device.as_path()]),
        Command::Multiboot {
            command: MultibootCommand::Build { output, .. },
        } => paths.push(output),
        Command::Multiboot {
            command: MultibootCommand::Add { image, .. },
        } => paths.push(image),
        Command::Multiboot {
            command: MultibootCommand::Verify { image, manifest },
        } => {
            paths.push(image);
            paths.extend(manifest.iter().map(PathBuf::as_path));
        }
        Command::Multiboot {
            command:
                MultibootCommand::Test {
                    image,
                    evidence_dir,
                    bios,
                    ovmf_code,
                    ovmf_vars,
                    swtpm,
                    ..
                },
        } => {
            paths.extend([image.as_path(), evidence_dir.as_path()]);
            paths.extend(
                bios.iter()
                    .chain(ovmf_code.iter())
                    .chain(ovmf_vars.iter())
                    .chain(swtpm.iter())
                    .map(PathBuf::as_path),
            );
            if resolved.starts_with(resolve_new(evidence_dir)?) {
                return Err(Error::InvalidInput(
                    "report must be outside the firmware evidence directory".into(),
                ));
            }
        }
        _ => {}
    }
    let multiboot_manifest = match command {
        Command::Multiboot {
            command:
                MultibootCommand::Plan { manifest, .. }
                | MultibootCommand::Build { manifest, .. }
                | MultibootCommand::Add { manifest, .. },
        } => Some(manifest),
        Command::Multiboot {
            command: MultibootCommand::Verify { manifest, .. },
        } => manifest.as_ref(),
        _ => None,
    };
    if let Some(manifest) = multiboot_manifest.filter(|path| path.exists()) {
        let spec = libmkiso::boot_media::manifest::read_multiboot(manifest)?;
        let base = manifest.parent().unwrap_or(Path::new("."));
        let assets = resolve_new(&base.join(&spec.boot.assets))?;
        if resolved.starts_with(assets) {
            return Err(Error::InvalidInput(
                "report overlaps multiboot assets".into(),
            ));
        }
        for entry in spec.entries {
            if resolved == resolve_new(&base.join(entry.image))? {
                return Err(Error::InvalidInput(
                    "report overlaps multiboot ISO input".into(),
                ));
            }
        }
    }
    for path in paths {
        let path = resolve_new(path)?;
        if resolved == path || (path.is_dir() && resolved.starts_with(&path)) {
            return Err(Error::InvalidInput(
                "report path overlaps an input or primary output".into(),
            ));
        }
    }
    Ok(())
}
fn resolve_new(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(parent.canonicalize()?.join(
        path.file_name()
            .ok_or_else(|| Error::InvalidInput("path has no filename".into()))?,
    ))
}
