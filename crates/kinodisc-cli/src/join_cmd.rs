//! `kinodisc-cli join <a> <b> [...] -o <joined.mp4> [--4x3] [--fps N]` —
//! joins clips into one video (same filters as "Join videos" in the app)
//! and prints where each clip starts (chapters).

use kinodisc_render::join::{build_join, JoinRequest};
use std::process::ExitCode;

pub async fn run(args: &[String]) -> ExitCode {
    let mut clips = Vec::new();
    let mut output = None;
    let mut widescreen = true;
    let mut fps = 25.0f64;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                output = args.get(index).cloned();
            }
            "--4x3" => widescreen = false,
            "--fps" => {
                index += 1;
                match args.get(index).and_then(|value| value.parse::<f64>().ok()) {
                    Some(value) => fps = value,
                    None => {
                        eprintln!("--fps espera un número (25 o 29.97).");
                        return ExitCode::FAILURE;
                    }
                }
            }
            value if !value.starts_with('-') => clips.push(value.to_string()),
            value => {
                eprintln!("Opción desconocida para join: {value}");
                return ExitCode::FAILURE;
            }
        }
        index += 1;
    }
    let Some(output) = output else {
        eprintln!("Uso: kinodisc-cli join <vídeo> <vídeo> [...] -o <unido.mp4> [--4x3] [--fps N]");
        return ExitCode::FAILURE;
    };
    let staging = std::env::temp_dir().join(format!("kinodisc-join-{}", std::process::id()));
    let request = JoinRequest {
        clips,
        widescreen,
        fps,
    };
    let outcome = match build_join(&request, &staging).await {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = std::fs::rename(&outcome.path, &output)
        .or_else(|_| std::fs::copy(&outcome.path, &output).map(|_| ()))
    {
        eprintln!("No se pudo escribir {output}: {error}");
        return ExitCode::FAILURE;
    }
    let _ = std::fs::remove_dir_all(&staging);
    for (number, start) in outcome.clip_starts_ms.iter().enumerate() {
        println!("Capítulo {}: {:.3} s", number + 1, start / 1000.0);
    }
    println!("{:.1} s → {output}", outcome.duration_ms / 1000.0);
    ExitCode::SUCCESS
}
