//! `kinodisc-cli inspect <disc.iso|folder> [--json] [--import <dir>]` —
//! what is on a DVD-Video/Blu-ray disc, and optional import of its titles
//! as plain video files.

use std::path::Path;
use std::process::ExitCode;

fn clock(ms: f64) -> String {
    let seconds = (ms / 1000.0).round() as u64;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

pub fn run(args: &[String]) -> ExitCode {
    let json = args.iter().any(|arg| arg == "--json");
    let import = args
        .iter()
        .position(|arg| arg == "--import")
        .and_then(|index| args.get(index + 1));
    let Some(input) = args
        .iter()
        .enumerate()
        .find(|(index, arg)| {
            !arg.starts_with('-') && (*index == 0 || args[index - 1] != "--import")
        })
        .map(|(_, arg)| arg)
    else {
        eprintln!("Uso: kinodisc-cli inspect <disco.iso|carpeta> [--json] [--import <carpeta>]");
        return ExitCode::FAILURE;
    };
    let report = match kinodisc_verify::inspect::inspect(Path::new(input)) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        // Blu-ray folders have no single video format: no trailing " · ".
        let video = if report.video.trim().is_empty() {
            String::new()
        } else {
            format!(" · {}", report.video)
        };
        println!(
            "{:?} «{}» · {} archivos · {:.2} GB{}{}",
            report.kind,
            report.volume_label,
            report.file_count,
            report.total_bytes / 1e9,
            video,
            if report.encrypted { " · CIFRADO" } else { "" }
        );
        for title in &report.titles {
            println!(
                "  Título {:>3}: {} · {} capítulos · {} audio · {} subtítulos · {:.0} MB{}",
                title.number,
                clock(title.duration_ms),
                title.chapters,
                title.audio.len(),
                title.subtitles.len(),
                title.bytes / 1e6,
                if title.duplicate { " (repetido)" } else { "" }
            );
        }
        for note in &report.notes {
            println!("  Nota: {note}");
        }
    }
    if let Some(dest) = import {
        match kinodisc_verify::inspect::import_titles(Path::new(input), Path::new(dest), &|| false)
        {
            Ok(titles) => {
                for title in titles {
                    println!(
                        "Importado: {} ({} capítulos)",
                        title.path,
                        title.chapter_starts_ms.len()
                    );
                }
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
