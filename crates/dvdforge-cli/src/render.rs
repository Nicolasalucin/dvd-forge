use dvdforge_domain::{AuthoringMode, JobId};
use dvdforge_jobs::JobEventKind;
use dvdforge_plan::calculate_bitrate_plan;
use dvdforge_project::load_and_validate;
use dvdforge_render::{render_project_with_options, RenderError, RenderOptions};
use dvdforge_toolchain::EncoderPreference;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::support::{collect_probes, print_plan_summary};

#[derive(Debug)]
struct RenderArgs {
    project: PathBuf,
    output: PathBuf,
    no_probe: bool,
    mode: Option<AuthoringMode>,
    avchd_folder: bool,
}

pub async fn run(args: &[String]) -> ExitCode {
    let args = match parse_render_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match render(&args).await {
        Ok(iso) => {
            println!();
            println!("Render completado: {}", iso.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Render falló: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn render(args: &RenderArgs) -> Result<PathBuf, String> {
    let project_path = args
        .project
        .canonicalize()
        .map_err(|error| format!("No se pudo resolver {}: {error}", args.project.display()))?;
    let mut project = load_and_validate(&project_path).map_err(|error| error.to_string())?;
    let project_dir = project_path
        .parent()
        .ok_or_else(|| "El proyecto no tiene directorio padre".to_string())?
        .to_path_buf();
    let output = absolute_output(&args.output)?;
    if let Some(requested) = args.mode {
        if requested != project.disc.mode {
            return Err(format!(
                "El proyecto declara modo {:?}, pero se solicitó {:?}.",
                project.disc.mode, requested
            ));
        }
    }
    if args.avchd_folder && project.disc.mode != AuthoringMode::Avchd {
        return Err("--avchd-folder solo se aplica a proyectos AVCHD.".to_string());
    }

    // Keep the CLI plan summary, but delegate every authoring step to the
    // unified renderer so DVD, AVCHD, Blu-ray, motion menus, HDMV IG menus,
    // subtitles and region masks behave exactly like the desktop app.
    let (probes, probe_warnings) = collect_probes(&project, &project_dir, !args.no_probe).await;
    // Hand these probes to the renderer so it neither re-runs FFprobe over
    // every source nor plans with different metadata than printed here.
    for asset in &mut project.assets {
        if let Some(probe) = probes.get(&asset.id) {
            asset.probe = probe.clone();
        }
    }
    let mut plan = calculate_bitrate_plan(&project, &probes).map_err(|error| error.to_string())?;
    plan.warnings.extend(probe_warnings);
    print_plan_summary(&plan);
    if !plan.feasible {
        return Err("El plan de bitrate no es viable.".to_string());
    }

    let mode_label = match project.disc.mode {
        AuthoringMode::DvdVideo => "dvd",
        AuthoringMode::Avchd if project.disc.capacity.is_blu_ray() => "bluray",
        AuthoringMode::Avchd => "avchd",
    };
    let region_mask =
        matches!(project.disc.mode, AuthoringMode::DvdVideo).then_some(project.disc.region_mask);
    println!();
    println!(
        "Salida: {} ({} · {})",
        output.display(),
        mode_label,
        project.disc.capacity.label()
    );

    let (sender, mut receiver) = mpsc::channel(256);
    let drain = tokio::spawn(async move {
        while let Some(kind) = receiver.recv().await {
            match kind {
                JobEventKind::StageStarted { label, .. } => println!("{label}..."),
                JobEventKind::Warning { code, message } => {
                    eprintln!("Aviso {code}: {message}");
                }
                JobEventKind::Completed { summary } => println!("{summary}"),
                JobEventKind::Failed { message, .. } => eprintln!("Error: {message}"),
                JobEventKind::Cancelled { .. } => eprintln!("Generación cancelada."),
                _ => {}
            }
        }
    });

    // Cancel like the desktop app: raise the flag (libmkiso polls it) and drop
    // the render future (kill_on_drop children, TempIso removes the .part).
    let cancel = Arc::new(AtomicBool::new(false));
    let render = Box::pin(render_project_with_options(
        project,
        project_dir,
        output.clone(),
        JobId::new(),
        RenderOptions {
            probe_assets: false,
            composited_backgrounds: None,
            cancel: Some(Arc::clone(&cancel)),
            hardware_encoding: EncoderPreference::Auto,
        },
        sender,
    ));
    let result = tokio::select! {
        result = render => result,
        _ = tokio::signal::ctrl_c() => {
            cancel.store(true, Ordering::SeqCst);
            eprintln!("\nCancelando render...");
            Err(RenderError::Cancelled)
        }
    };
    // Flush every queued warning/error before propagating the result.
    let _ = drain.await;
    let outcome = result.map_err(|error| error.to_string())?;

    let iso_path = PathBuf::from(&outcome.iso_path);
    let iso_bytes = std::fs::metadata(&iso_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let folder_path = if args.avchd_folder {
        let bdmv_dir = output.join("avchd");
        if bdmv_dir.join("BDMV/index.bdmv").is_file() {
            println!("Carpeta BDMV generada: {}", bdmv_dir.display());
            Some(bdmv_dir)
        } else {
            None
        }
    } else {
        None
    };
    let summary = serde_json::json!({
        "mode": mode_label,
        "iso": outcome.iso_path,
        "isoBytes": iso_bytes,
        "elapsedMs": outcome.elapsed_ms,
        "regionMask": region_mask,
        "folder": folder_path.as_ref().map(|path| path.display().to_string()),
        "warnings": outcome.warnings.iter().map(|warning| warning.code.clone()).collect::<Vec<_>>(),
    });
    let summary_path = output.join("render.json");
    std::fs::write(
        &summary_path,
        serde_json::to_string_pretty(&summary)
            .map_err(|error| format!("No se pudo serializar el resumen: {error}"))?,
    )
    .map_err(|error| format!("No se pudo escribir {}: {error}", summary_path.display()))?;
    println!("Resumen: {}", summary_path.display());
    Ok(iso_path)
}

fn parse_render_args(args: &[String]) -> Result<RenderArgs, String> {
    let mut project = None;
    let mut output = PathBuf::from("build/out");
    let mut no_probe = false;
    let mut mode = None;
    let mut avchd_folder = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("Falta el valor de -o/--output.".to_string());
                };
                output = PathBuf::from(value);
            }
            "--no-probe" => no_probe = true,
            "--avchd-folder" => avchd_folder = true,
            "--mode" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("Falta el valor de --mode.".to_string());
                };
                mode = Some(match value.to_ascii_lowercase().as_str() {
                    "dvd" | "dvdvideo" | "dvd-video" => AuthoringMode::DvdVideo,
                    "avchd" => AuthoringMode::Avchd,
                    _ => {
                        return Err(format!("Modo desconocido: {value}. Usa dvd o avchd."));
                    }
                });
            }
            value if value.starts_with('-') => {
                return Err(format!("Opción desconocida para render: {value}"));
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
    Ok(RenderArgs {
        project: project.ok_or_else(|| {
            "Uso: dvdforge-cli render <project.json> [-o <dir>] [--mode dvd|avchd] [--avchd-folder] [--no-probe]"
                .to_string()
        })?,
        output,
        no_probe,
        mode,
        avchd_folder,
    })
}

fn absolute_output(output: &Path) -> Result<PathBuf, String> {
    if output.is_absolute() {
        return Ok(output.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(output))
        .map_err(|error| format!("No se pudo resolver el directorio de salida: {error}"))
}
