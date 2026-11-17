//! `kinodisc-cli slideshow <photos...> -o <dir> [--seconds 5] [--fade 1]
//! [--music <file>] [--fps 25] [--4x3]` — builds a photo slideshow video
//! that can be added to a project as an episode.

use kinodisc_render::slideshow::{build_slideshow, SlideshowRequest};
use std::path::PathBuf;
use std::process::ExitCode;

pub async fn run(args: &[String]) -> ExitCode {
    let mut request = SlideshowRequest {
        images: Vec::new(),
        seconds_per_image: 5.0,
        crossfade_seconds: 1.0,
        music: None,
        widescreen: true,
        fps: 25.0,
    };
    let mut output = PathBuf::from(".");
    let mut index = 0;
    let number = |value: Option<&String>| value.and_then(|value| value.parse::<f64>().ok());
    while index < args.len() {
        let value = args.get(index + 1);
        match args[index].as_str() {
            "-o" | "--output" => {
                output = value.map(PathBuf::from).unwrap_or(output);
                index += 1;
            }
            "--seconds" => {
                request.seconds_per_image = number(value).unwrap_or(f64::NAN);
                index += 1;
            }
            "--fade" => {
                request.crossfade_seconds = number(value).unwrap_or(f64::NAN);
                index += 1;
            }
            "--fps" => {
                request.fps = number(value).unwrap_or(f64::NAN);
                index += 1;
            }
            "--music" => {
                request.music = value.cloned();
                index += 1;
            }
            "--4x3" => request.widescreen = false,
            other if other.starts_with('-') => {
                eprintln!("Opción desconocida para slideshow: {other}");
                return ExitCode::FAILURE;
            }
            photo => {
                let path = PathBuf::from(photo);
                let absolute = std::fs::canonicalize(&path).unwrap_or(path);
                request.images.push(
                    absolute
                        .display()
                        .to_string()
                        .trim_start_matches(r"\?\")
                        .to_string(),
                );
            }
        }
        index += 1;
    }
    if let Some(music) = request.music.take() {
        let path = PathBuf::from(&music);
        let absolute = std::fs::canonicalize(&path).unwrap_or(path);
        request.music = Some(
            absolute
                .display()
                .to_string()
                .trim_start_matches(r"\?\")
                .to_string(),
        );
    }
    match build_slideshow(&request, &output).await {
        Ok(outcome) => {
            println!(
                "Presentación {}: {} ({:.1} s)",
                if outcome.cached {
                    "reutilizada"
                } else {
                    "creada"
                },
                outcome.path,
                outcome.duration_ms / 1000.0
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
