mod render;
mod support;

use dvdforge_burn::{platform_burner, BurnRequest, CancellationToken, DeviceId};
use dvdforge_domain::MediaProbe;
use dvdforge_media::probe;
use dvdforge_plan::calculate_bitrate_plan;
use dvdforge_project::load_and_validate;
use dvdforge_toolchain::{detect_toolchain, ToolchainReport};
use std::env;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use support::{collect_probes, format_duration, print_plan_summary};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return ExitCode::SUCCESS;
    };

    match command {
        "doctor" => doctor(&args[1..]),
        "plan" => plan(&args[1..]).await,
        "probe" => probe_command(&args[1..]).await,
        "render" => render::run(&args[1..]).await,
        "verify" => verify_command(&args[1..]).await,
        "devices" => devices_command(&args[1..]).await,
        "burn" => burn_command(&args[1..]).await,
        "version" | "--version" | "-V" => {
            println!("dvdforge-cli {VERSION}");
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        unknown => {
            eprintln!("Comando desconocido: {unknown}");
            eprintln!("Ejecuta `dvdforge-cli help` para ver los comandos disponibles.");
            ExitCode::FAILURE
        }
    }
}

fn doctor(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");

    if args
        .iter()
        .any(|arg| arg != "--json" && arg != "--verbose" && arg != "-v")
    {
        eprintln!("Uso: dvdforge-cli doctor [--json] [--verbose]");
        return ExitCode::FAILURE;
    }

    let report = detect_toolchain();
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => println!("{output}"),
            Err(error) => {
                eprintln!("No se pudo serializar el informe: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_report(&report, verbose);
    }

    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[derive(Debug)]
struct PlanArgs {
    project: PathBuf,
    output: Option<PathBuf>,
    refresh_probes: bool,
}

async fn plan(args: &[String]) -> ExitCode {
    let args = match parse_plan_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    let project = match load_and_validate(&args.project) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("No se pudo cargar el proyecto: {error}");
            return ExitCode::FAILURE;
        }
    };

    let project_dir = args
        .project
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (probes, probe_warnings) = collect_probes(&project, project_dir, args.refresh_probes).await;

    let mut plan = match calculate_bitrate_plan(&project, &probes) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("No se pudo calcular el plan: {error}");
            return ExitCode::FAILURE;
        }
    };
    plan.warnings.extend(probe_warnings);

    let json = match serde_json::to_string_pretty(&plan) {
        Ok(json) => json,
        Err(error) => {
            eprintln!("No se pudo serializar el plan: {error}");
            return ExitCode::FAILURE;
        }
    };

    match &args.output {
        Some(path) if path != Path::new("-") => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    eprintln!("No se pudo crear {}: {error}", parent.display());
                    return ExitCode::FAILURE;
                }
            }
            if let Err(error) = std::fs::write(path, format!("{json}\n")) {
                eprintln!("No se pudo escribir {}: {error}", path.display());
                return ExitCode::FAILURE;
            }
            println!("Plan escrito en {}", path.display());
            print_plan_summary(&plan);
        }
        _ => println!("{json}"),
    }

    if plan.feasible {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn parse_plan_args(args: &[String]) -> Result<PlanArgs, String> {
    let mut project = None;
    let mut output = None;
    let mut refresh_probes = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--probe" => refresh_probes = true,
            "-o" | "--output" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("Falta el valor de -o/--output.".to_string());
                };
                output = Some(PathBuf::from(value));
            }
            value if value.starts_with('-') => {
                return Err(format!("Opción desconocida para plan: {value}"));
            }
            value => {
                if project.is_some() {
                    return Err("Solo se admite un proyecto por invocación.".to_string());
                }
                project = Some(PathBuf::from(value));
            }
        }
        index += 1;
    }

    let project = project.ok_or_else(|| {
        "Uso: dvdforge-cli plan <project.json> [-o <plan.json>] [--probe]".to_string()
    })?;
    Ok(PlanArgs {
        project,
        output,
        refresh_probes,
    })
}

async fn probe_command(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let paths: Vec<&String> = args.iter().filter(|arg| arg.as_str() != "--json").collect();
    if paths.len() != 1 {
        eprintln!("Uso: dvdforge-cli probe <archivo> [--json]");
        return ExitCode::FAILURE;
    }
    let path = Path::new(paths[0].as_str());
    if is_optical_device_path(path) {
        return probe_optical(path, json).await;
    }
    match probe(path).await {
        Ok(media) => {
            if json {
                match serde_json::to_string_pretty(&media) {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("No se pudo serializar el probe: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                print_probe(&media);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("FFprobe falló: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn devices_command(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    if args.iter().any(|arg| arg != "--json") {
        eprintln!("Uso: dvdforge-cli devices [--json]");
        return ExitCode::FAILURE;
    }
    let burner = platform_burner();
    match burner.list_devices().await {
        Ok(devices) => {
            if json {
                match serde_json::to_string_pretty(&devices) {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("No se pudo serializar la lista: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            } else if devices.is_empty() {
                println!("No se detectaron unidades ópticas.");
            } else {
                for device in &devices {
                    println!(
                        "{}  {} {}  bus={}  grabable={}",
                        device.id, device.vendor, device.model, device.bus, device.writable
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("No se pudieron listar las unidades: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn probe_optical(path: &Path, json: bool) -> ExitCode {
    let device = DeviceId::new(path.to_string_lossy().to_string());
    match platform_burner().probe_media(&device).await {
        Ok(media) => {
            if json {
                match serde_json::to_string_pretty(&media) {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("No se pudo serializar el medio: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                print_optical_media(&media);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("No se pudo inspeccionar {device}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_optical_media(media: &dvdforge_burn::MediaInfo) {
    println!("Tipo: {:?}", media.kind);
    println!("Estado: {:?}", media.status);
    println!("Capacidad: {} bytes", media.capacity_bytes);
    println!("Libre: {} bytes", media.free_bytes);
    println!("Grabable: {}", media.writable);
    println!("Regrabable: {}", media.rewritable);
    if let Some(speed) = media.max_speed_x {
        println!("Velocidad máxima: {speed:.1}x");
    }
}

#[derive(Debug)]
struct VerifyArgs {
    iso: PathBuf,
    deep: bool,
    json: bool,
    strict: bool,
}

async fn verify_command(args: &[String]) -> ExitCode {
    let args = match parse_verify_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    let path = args.iso.clone();
    let deep = args.deep;
    let report =
        match tokio::task::spawn_blocking(move || dvdforge_verify::verify_iso(&path, deep)).await {
            Ok(Ok(report)) => report,
            Ok(Err(error)) => {
                eprintln!("No se pudo verificar el ISO: {error}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("La verificación terminó inesperadamente: {error}");
                return ExitCode::FAILURE;
            }
        };

    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("No se pudo serializar el informe: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("DVD Forge verify · {}", report.iso_path);
        println!(
            "Perfil: {:?} · {} bytes · {:?} · {} ms",
            report.profile, report.size_bytes, report.verdict, report.elapsed_ms
        );
        for check in &report.checks {
            let status = match check.status {
                dvdforge_verify::report::CheckStatus::Pass => "OK  ",
                dvdforge_verify::report::CheckStatus::Warn => "WARN",
                dvdforge_verify::report::CheckStatus::Fail => "FAIL",
                dvdforge_verify::report::CheckStatus::Skipped => "SKIP",
            };
            println!("[{status}] {:<18} {}", check.id, check.message);
        }
    }

    let failed = report
        .checks
        .iter()
        .any(|check| matches!(check.status, dvdforge_verify::report::CheckStatus::Fail));
    let warned = report
        .checks
        .iter()
        .any(|check| matches!(check.status, dvdforge_verify::report::CheckStatus::Warn));
    if failed || (args.strict && warned) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn parse_verify_args(args: &[String]) -> Result<VerifyArgs, String> {
    let mut iso = None;
    let mut deep = false;
    let mut json = false;
    let mut strict = false;
    for arg in args {
        match arg.as_str() {
            "--deep" => deep = true,
            "--json" => json = true,
            "--strict" => strict = true,
            value if value.starts_with('-') => {
                return Err(format!("Opción desconocida para verify: {value}"));
            }
            value => {
                if iso.is_some() {
                    return Err("Solo se admite un ISO por invocación.".to_string());
                }
                iso = Some(PathBuf::from(value));
            }
        }
    }
    Ok(VerifyArgs {
        iso: iso.ok_or_else(|| {
            "Uso: dvdforge-cli verify <iso> [--deep] [--json] [--strict]".to_string()
        })?,
        deep,
        json,
        strict,
    })
}

#[derive(Debug)]
struct BurnArgs {
    iso: PathBuf,
    device: DeviceId,
    speed: Option<u16>,
    verify: bool,
    eject: bool,
    skip_verify: bool,
}

async fn burn_command(args: &[String]) -> ExitCode {
    let args = match parse_burn_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if !args.skip_verify {
        let path = args.iso.clone();
        match tokio::task::spawn_blocking(move || dvdforge_verify::verify_iso(&path, false)).await {
            Ok(Ok(report)) => {
                if report.verdict == dvdforge_verify::report::VerifyVerdict::Fail {
                    eprintln!("La verificación estructural falló; no se graba el ISO.");
                    for check in &report.checks {
                        if matches!(check.status, dvdforge_verify::report::CheckStatus::Fail) {
                            eprintln!("  [{}] {}", check.id, check.message);
                        }
                    }
                    return ExitCode::FAILURE;
                }
                if report.verdict == dvdforge_verify::report::VerifyVerdict::Warn {
                    eprintln!("Verificación estructural con avisos; se continúa.");
                }
            }
            Ok(Err(error)) => {
                eprintln!("No se pudo verificar el ISO antes de grabar: {error}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("La verificación terminó inesperadamente: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
    let burner = Arc::new(platform_burner());
    let cancel = CancellationToken::new();
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nCancelando grabación...");
            signal_cancel.cancel();
        }
    });
    let label = args
        .iso
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("DVD_FORGE")
        .to_string();
    let request = BurnRequest {
        iso_path: args.iso,
        device: args.device,
        speed_x: args.speed,
        verify: args.verify,
        eject: args.eject,
        close_session: true,
        simulation: false,
        label,
    };
    let task_burner = burner.clone();
    let task = tokio::spawn(async move { task_burner.burn(request, sender, cancel).await });
    let mut last_percent = -1.0f32;
    while let Some(progress) = receiver.recv().await {
        if progress.percent - last_percent >= 0.5 || progress.percent >= 100.0 {
            print!(
                "\r{:5.1}%  {} / {} bytes",
                progress.percent, progress.written_bytes, progress.total_bytes
            );
            if let Some(speed) = progress.speed_x {
                print!("  {speed:.1}x");
            }
            if let Some(eta) = progress.eta_secs {
                print!("  ETA {eta}s");
            }
            let _ = std::io::stdout().flush();
            last_percent = progress.percent;
        }
    }
    println!();
    match task.await {
        Ok(Ok(report)) => {
            println!(
                "Grabación completada con {:?}: {} ms, velocidad media {}, verificada={}",
                report.backend,
                report.elapsed_ms,
                report
                    .average_speed_x
                    .map(|speed| format!("{speed:.2}x"))
                    .unwrap_or_else(|| "desconocida".to_string()),
                report.verified
            );
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            eprintln!("Grabación falló: {error}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("La tarea de grabación terminó de forma inesperada: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_burn_args(args: &[String]) -> Result<BurnArgs, String> {
    let mut iso = None;
    let mut device = None;
    let mut speed = None;
    let mut verify = false;
    let mut eject = false;
    let mut skip_verify = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--device" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("Falta el valor de --device.".to_string());
                };
                device = Some(DeviceId::new(value.clone()));
            }
            "--speed" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("Falta el valor de --speed.".to_string());
                };
                let parsed = value
                    .parse::<u16>()
                    .map_err(|_| format!("Velocidad inválida: {value}"))?;
                speed = Some(parsed);
            }
            "--verify" => verify = true,
            "--eject" => eject = true,
            "--skip-verify" => skip_verify = true,
            value if value.starts_with('-') => {
                return Err(format!("Opción desconocida para burn: {value}"));
            }
            value => {
                if iso.is_some() {
                    return Err("Solo se admite un ISO por invocación.".to_string());
                }
                iso = Some(PathBuf::from(value));
            }
        }
        index += 1;
    }
    let iso = iso.ok_or_else(|| {
        "Uso: dvdforge-cli burn <iso> --device <dev> [--speed X] [--verify] [--eject] [--skip-verify]".to_string()
    })?;
    let device = device.ok_or_else(|| "Falta --device.".to_string())?;
    Ok(BurnArgs {
        iso,
        device,
        speed,
        verify,
        eject,
        skip_verify,
    })
}

fn is_optical_device_path(path: &Path) -> bool {
    let value = path.to_string_lossy();
    value.starts_with("/dev/")
        || value.starts_with("\\\\.\\")
        || (value.len() == 2 && value.ends_with(':'))
}

fn print_probe(probe: &MediaProbe) {
    println!("Formato: {}", probe.format_name);
    if let Some(duration) = probe.duration_ms {
        println!("Duración: {}", format_duration(duration));
    }
    if let Some(video) = &probe.video {
        println!(
            "Video: {} {}x{} @ {:.3} fps, SAR {}, DAR {}",
            video.codec,
            video.width,
            video.height,
            video.frame_rate.as_f64(),
            video.sample_aspect_ratio.as_deref().unwrap_or("-"),
            video.display_aspect_ratio.as_deref().unwrap_or("-")
        );
    }
    for audio in &probe.audio {
        println!(
            "Audio #{}: {} {} canales, {} Hz, {} bps, idioma {}",
            audio.index,
            audio.codec,
            audio.channels,
            audio.sample_rate,
            audio
                .bitrate_bps
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            audio.language.as_deref().unwrap_or("-")
        );
    }
}

fn print_report(report: &ToolchainReport, verbose: bool) {
    println!("DVD Forge doctor");
    println!("Plataforma: {}", report.platform);
    println!();
    for tool in &report.tools {
        let status = if tool.found { "OK  " } else { "FALTA" };
        let version = tool.version.as_deref().unwrap_or("versión desconocida");
        let path = tool.path.as_deref().unwrap_or("-");
        println!("[{status}] {:<20} {:<16} {path}", tool.name, version);
        if verbose {
            if let Some(detail) = &tool.detail {
                println!("       {detail}");
            }
            if let Some(error) = &tool.error {
                println!("       error: {error}");
            }
        }
    }

    if !report.missing_required.is_empty() {
        println!();
        println!(
            "Herramientas requeridas ausentes: {}",
            report
                .missing_required
                .iter()
                .map(|id| id.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !report.warnings.is_empty() {
        println!();
        for warning in &report.warnings {
            println!("Aviso: {warning}");
        }
    }
    println!();
    if report.ok {
        println!("Toolchain completo.");
    } else {
        println!("Toolchain incompleto.");
    }
}

fn print_help() {
    println!("DVD Forge CLI {VERSION}");
    println!();
    println!("Uso:");
    println!("  dvdforge-cli doctor [--json] [--verbose]");
    println!("  dvdforge-cli plan <project.json> [-o <plan.json>] [--probe]");
    println!("  dvdforge-cli probe <archivo|/dev/srX> [--json]");
    println!("  dvdforge-cli render <project.json> [-o <dir>] [--mode dvd|avchd] [--avchd-folder] [--no-probe]");
    println!("  dvdforge-cli verify <iso> [--deep] [--json] [--strict]");
    println!("  dvdforge-cli devices [--json]");
    println!(
        "  dvdforge-cli burn <iso> --device <dev> [--speed X] [--verify] [--eject] [--skip-verify]"
    );
    println!("  dvdforge-cli version");
    println!("  dvdforge-cli help");
    println!();
    println!("`plan` usa los metadatos guardados por defecto; --probe los refresca con ffprobe.");
    println!("`probe` usa FFprobe para archivos y dvd+rw-mediainfo para /dev/srX.");
    println!("`doctor` detecta FFmpeg, FFprobe, DVDAuthor, Spumux, xorriso,");
    println!("growisofs, dvd+rw-mediainfo y tsMuxeR.");
}
