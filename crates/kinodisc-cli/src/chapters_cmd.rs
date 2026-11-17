//! `kinodisc-cli chapters <project.json> --scenes [--sensitivity 0..1]
//! [-o <out.json>]` — puts a chapter at every scene change of each episode
//! (the "By scenes" button of the app).

use kinodisc_domain::{AuthoringMode, Chapter, ChapterId, TvStandard, VideoConfig};
use kinodisc_media::scenes::{detect_scenes, SceneOptions};
use std::process::ExitCode;

pub async fn run(args: &[String]) -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut scenes = false;
    let mut sensitivity = 0.5f64;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                output = args.get(index).cloned();
            }
            "--scenes" => scenes = true,
            "--sensitivity" => {
                index += 1;
                match args.get(index).and_then(|value| value.parse::<f64>().ok()) {
                    Some(value) if (0.0..=1.0).contains(&value) => sensitivity = value,
                    _ => {
                        eprintln!("--sensitivity espera un número entre 0 y 1.");
                        return ExitCode::FAILURE;
                    }
                }
            }
            value if !value.starts_with('-') && input.is_none() => input = Some(value.to_string()),
            value => {
                eprintln!("Opción desconocida para chapters: {value}");
                return ExitCode::FAILURE;
            }
        }
        index += 1;
    }
    let (Some(input), true) = (input, scenes) else {
        eprintln!("Uso: kinodisc-cli chapters <project.json> --scenes [--sensitivity 0..1] [-o <salida.json>]");
        return ExitCode::FAILURE;
    };
    let mut project = match kinodisc_project::load_and_validate(&input) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("No se pudo abrir el proyecto: {error}");
            return ExitCode::FAILURE;
        }
    };
    let project_dir = std::path::Path::new(&input)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    // Chapter frames are in the disc's rate (as the editor writes them).
    let fps = match &project.disc.video {
        VideoConfig::Avchd(config) => config.frame_rate.as_f64(),
        _ if project.disc.standard == TvStandard::Pal => 25.0,
        _ => 30000.0 / 1001.0,
    };
    let max_chapters = if project.disc.mode == AuthoringMode::DvdVideo {
        99
    } else {
        999
    };
    let threshold = 0.6 - sensitivity * 0.45;
    let mut total = 0;
    let assets = project.assets.clone();
    for episode in &mut project.episodes {
        let Some(asset) = assets.iter().find(|asset| asset.id == episode.video) else {
            continue;
        };
        let path = match &asset.source {
            kinodisc_domain::AssetSource::External { absolute_path } => absolute_path.into(),
            kinodisc_domain::AssetSource::Project => {
                project_dir.join(asset.relative_path.clone().unwrap_or_default())
            }
        };
        let duration = episode.effective_duration_ms();
        let options = SceneOptions {
            threshold,
            min_gap_ms: (duration / 8).clamp(5_000, 30_000),
            start_ms: episode.trim_start_ms.unwrap_or(0),
            end_ms: episode.trim_end_ms,
            max_chapters,
        };
        let offsets = match detect_scenes(&path, options).await {
            Ok(offsets) => offsets,
            Err(error) => {
                eprintln!("Episodio {}: {error}", episode.number);
                return ExitCode::FAILURE;
            }
        };
        episode.chapters = offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| Chapter {
                id: ChapterId::new(),
                title: if index == 0 {
                    "Inicio".to_string()
                } else {
                    format!("Capítulo {}", index + 1)
                },
                at_frame: (*offset as f64 / 1000.0 * fps).round() as u64,
                thumbnail: None,
            })
            .collect();
        println!(
            "Episodio {}: {} capítulos",
            episode.number,
            episode.chapters.len()
        );
        total += episode.chapters.len();
    }
    let target = output.unwrap_or(input);
    if let Err(error) = kinodisc_project::save_project(&project, &target) {
        eprintln!("No se pudo escribir {target}: {error}");
        return ExitCode::FAILURE;
    }
    println!("{total} capítulos → {target}");
    ExitCode::SUCCESS
}
