//! `kinodisc-cli split <project.json> [--per-episode]` — writes one project
//! per disc next to the original (so relative media paths keep working),
//! ready for `render` or `batch`.

use kinodisc_render::split::{split_project, SplitStrategy};
use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

pub fn run(args: &[String]) -> ExitCode {
    let per_episode = args.iter().any(|arg| arg == "--per-episode");
    let Some(input) = args.iter().find(|arg| !arg.starts_with('-')) else {
        eprintln!("Uso: kinodisc-cli split <project.json> [--per-episode]");
        return ExitCode::FAILURE;
    };
    let project = match kinodisc_project::load_and_validate(input) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("No se pudo abrir el proyecto: {error}");
            return ExitCode::FAILURE;
        }
    };
    let strategy = if per_episode {
        SplitStrategy::PerEpisode
    } else {
        SplitStrategy::FitToDisc
    };
    let result = match split_project(&project, &HashMap::new(), strategy) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    for diagnostic in &result.diagnostics {
        println!("Aviso {}: {}", diagnostic.code, diagnostic.message);
    }
    let folder = Path::new(input).parent().unwrap_or(Path::new("."));
    for part in &result.parts {
        let name = part.file_name.trim_end_matches(".iso");
        let path = folder.join(format!("{name}.json"));
        if let Err(error) = kinodisc_project::save_project(&part.project, &path) {
            eprintln!("No se pudo escribir {}: {error}", path.display());
            return ExitCode::FAILURE;
        }
        println!(
            "Disco {}/{}: {} episodio(s) → {}",
            part.index,
            part.count,
            part.project.episodes.len(),
            path.display()
        );
    }
    ExitCode::SUCCESS
}
