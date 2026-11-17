//! `kinodisc-cli bundle export|import|info`.

use kinodisc_project::bundle::{
    export_bundle, import_bundle, read_bundle_manifest, BUNDLE_EXTENSION,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("Uso:");
    eprintln!("  kinodisc-cli bundle export <project.json> [-o <archivo.{BUNDLE_EXTENSION}>]");
    eprintln!("  kinodisc-cli bundle import <archivo.{BUNDLE_EXTENSION}> -o <carpeta-vacía>");
    eprintln!("  kinodisc-cli bundle info <archivo.{BUNDLE_EXTENSION}> [--json]");
    ExitCode::FAILURE
}

fn output_flag(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .position(|arg| arg == "-o" || arg == "--output")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

pub fn run(args: &[String]) -> ExitCode {
    let (Some(action), Some(input)) = (args.first(), args.get(1)) else {
        return usage();
    };
    let input = Path::new(input);
    match action.as_str() {
        "export" => {
            let project = match kinodisc_project::load_project(input) {
                Ok(project) => project,
                Err(error) => {
                    eprintln!("Error: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let project_dir = input
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let output =
                output_flag(args).unwrap_or_else(|| input.with_extension(BUNDLE_EXTENSION));
            match export_bundle(&project, project_dir, &output) {
                Ok(report) => {
                    for warning in &report.warnings {
                        eprintln!("Aviso {}: {}", warning.code, warning.message);
                    }
                    println!(
                        "Paquete creado: {} ({} archivos, {})",
                        report.path,
                        report.manifest.files.len(),
                        megabytes(report.manifest.total_bytes())
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("Error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        "import" => {
            let Some(destination) = output_flag(args) else {
                return usage();
            };
            match import_bundle(input, &destination) {
                Ok(imported) => {
                    println!(
                        "Proyecto «{}» restaurado: {} ({} archivos verificados)",
                        imported.manifest.project_name,
                        imported.project_path,
                        imported.manifest.files.len()
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("Error: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        "info" => match read_bundle_manifest(input) {
            Ok(manifest) => {
                if args.iter().any(|arg| arg == "--json") {
                    match serde_json::to_string_pretty(&manifest) {
                        Ok(json) => println!("{json}"),
                        Err(error) => {
                            eprintln!("Error: {error}");
                            return ExitCode::FAILURE;
                        }
                    }
                } else {
                    println!("Proyecto: {}", manifest.project_name);
                    println!("Creado con: {}", manifest.created_by);
                    println!(
                        "Formato {} · esquema {} · {} archivos · {}",
                        manifest.format_version,
                        manifest.schema_version,
                        manifest.files.len(),
                        megabytes(manifest.total_bytes())
                    );
                    for file in &manifest.files {
                        println!("  {:>10}  {}", megabytes(file.bytes), file.path);
                    }
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Error: {error}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}
