//! `kinodisc-cli scenes <project.json> [-o <out.json>]` — adds (or
//! refreshes) the automatic scene-selection menus built from the chapters.

use std::process::ExitCode;

pub fn run(args: &[String]) -> ExitCode {
    let mut input = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-o" | "--output" => {
                index += 1;
                output = args.get(index).cloned();
            }
            value if !value.starts_with('-') && input.is_none() => input = Some(value.to_string()),
            value => {
                eprintln!("Opción desconocida para scenes: {value}");
                return ExitCode::FAILURE;
            }
        }
        index += 1;
    }
    let Some(input) = input else {
        eprintln!("Uso: kinodisc-cli scenes <project.json> [-o <salida.json>]");
        return ExitCode::FAILURE;
    };
    let mut project = match kinodisc_project::load_and_validate(&input) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("No se pudo abrir el proyecto: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match kinodisc_menu::chapters::apply_chapter_menus(&project) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    for diagnostic in &result.diagnostics {
        println!("Aviso {}: {}", diagnostic.code, diagnostic.message);
    }
    project.menus = result.menus;
    let pages = project
        .menus
        .iter()
        .filter(|menu| menu.role == kinodisc_domain::MenuRole::ChapterSelect)
        .count();
    let target = output.unwrap_or(input);
    if let Err(error) = kinodisc_project::save_project(&project, &target) {
        eprintln!("No se pudo escribir {target}: {error}");
        return ExitCode::FAILURE;
    }
    println!("{pages} página(s) de escenas → {target}");
    ExitCode::SUCCESS
}
