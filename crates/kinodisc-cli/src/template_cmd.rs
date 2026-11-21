//! `kinodisc-cli template <project.json> <plantilla> [-o <out.json>]` — applies
//! a menu template (the same engine as Menu › Templates in the app).
//! `kinodisc-cli template --list` lists the templates.

use std::process::ExitCode;

use kinodisc_domain::MenuTemplateId;

const USAGE: &str = "Uso: kinodisc-cli template <project.json> <plantilla> [-o <salida.json>]\n     kinodisc-cli template --list";

/// The template id as the CLI writes it: kebab case (`multi-page-tv-grid`).
fn cli_name(id: MenuTemplateId) -> String {
    let camel = serde_json::to_value(id)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut name = String::new();
    for character in camel.chars() {
        if character.is_ascii_uppercase() {
            name.push('-');
            name.push(character.to_ascii_lowercase());
        } else {
            name.push(character);
        }
    }
    name
}

/// Accepts `multi-page-tv-grid`, `multiPageTvGrid` or any case of either.
fn parse_template(value: &str) -> Option<MenuTemplateId> {
    let wanted: String = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect();
    kinodisc_menu::templates::all_templates()
        .into_iter()
        .map(|info| info.id)
        .find(|id| cli_name(*id).replace('-', "") == wanted)
}

fn list() -> ExitCode {
    for info in kinodisc_menu::templates::all_templates() {
        println!(
            "{:<22} {} · {}×{} · {} episodios por página{}",
            cli_name(info.id),
            info.name,
            info.columns,
            info.rows,
            info.episode_capacity,
            if info.thumbnail { " · miniaturas" } else { "" }
        );
    }
    ExitCode::SUCCESS
}

pub fn run(args: &[String]) -> ExitCode {
    let mut input = None;
    let mut name = None;
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--list" | "-l" => return list(),
            "-o" | "--output" => {
                index += 1;
                // `-o` with no path (or another option after it) must not
                // fall back to writing over the input (QA 1.2.2).
                match args.get(index) {
                    Some(path) if !path.is_empty() && !path.starts_with('-') => {
                        output = Some(path.clone())
                    }
                    _ => {
                        eprintln!("Falta la ruta después de -o.");
                        eprintln!("{USAGE}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            value if !value.starts_with('-') && input.is_none() => input = Some(value.to_string()),
            value if !value.starts_with('-') && name.is_none() => name = Some(value.to_string()),
            value => {
                eprintln!("Opción desconocida para template: {value}");
                eprintln!("{USAGE}");
                return ExitCode::FAILURE;
            }
        }
        index += 1;
    }
    let (Some(input), Some(name)) = (input, name) else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let Some(id) = parse_template(&name) else {
        eprintln!("No existe la plantilla «{name}». Usa `kinodisc-cli template --list`.");
        return ExitCode::FAILURE;
    };
    let mut project = match kinodisc_project::load_and_validate(&input) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("No se pudo abrir el proyecto: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = match kinodisc_menu::templates::apply_template(&project, id) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("No se pudo aplicar la plantilla: {error}");
            return ExitCode::FAILURE;
        }
    };
    for diagnostic in &result.diagnostics {
        println!("Aviso {}: {}", diagnostic.code, diagnostic.message);
    }
    project.menus = result.menus;
    project.root_menu = result.root_menu;
    // The same check the app runs before saving: never write a broken project.
    let errors: Vec<_> = project
        .validate()
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == kinodisc_domain::Severity::Error)
        .collect();
    if !errors.is_empty() {
        for error in &errors {
            eprintln!("Error {}: {}", error.code, error.message);
        }
        eprintln!("No se guardó el proyecto.");
        return ExitCode::FAILURE;
    }
    let target = output.unwrap_or(input);
    if let Err(error) = kinodisc_project::save_project(&project, &target) {
        eprintln!("No se pudo escribir {target}: {error}");
        return ExitCode::FAILURE;
    }
    let buttons: usize = project.menus.iter().map(|menu| menu.buttons.len()).sum();
    println!(
        "Plantilla {} aplicada: {} menú(s), {} botón(es) → {target}",
        cli_name(id),
        project.menus.len(),
        buttons
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_names_round_trip_in_any_spelling() {
        assert_eq!(
            cli_name(MenuTemplateId::MultiPageTvGrid),
            "multi-page-tv-grid"
        );
        for spelling in [
            "multi-page-tv-grid",
            "multiPageTvGrid",
            "MULTI_PAGE_TV_GRID",
        ] {
            assert_eq!(
                parse_template(spelling),
                Some(MenuTemplateId::MultiPageTvGrid)
            );
        }
        assert_eq!(parse_template("no-existe"), None);
        for info in kinodisc_menu::templates::all_templates() {
            assert_eq!(parse_template(&cli_name(info.id)), Some(info.id));
        }
    }
}
