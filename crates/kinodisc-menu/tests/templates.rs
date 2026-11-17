use kinodisc_domain::{ButtonAction, ButtonId, EpisodeId, MenuTemplateId};
use kinodisc_menu::templates::{all_templates, apply_template};
use kinodisc_menu::{
    build_spumux_xml, generate_menu_layers, CompiledButton, MenuCompileRequest, MenuLayerRequest,
};
use kinodisc_project::load_project;
use std::path::PathBuf;

fn project_with_episodes(count: usize) -> kinodisc_domain::Project {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut project = load_project(root.join("fixtures/projects/minimal.json")).expect("fixture");
    let source = project.episodes[0].clone();
    project.episodes = (0..count)
        .map(|index| {
            let mut episode = source.clone();
            episode.id = EpisodeId::new();
            episode.number = (index + 1) as u16;
            episode.title = format!("Episode {:02}", index + 1);
            episode
        })
        .collect();
    project
}

fn episode_buttons(menus: &[kinodisc_domain::Menu]) -> Vec<&kinodisc_domain::MenuButton> {
    menus
        .iter()
        .filter(|menu| menu.template.is_some())
        .flat_map(|menu| menu.buttons.iter())
        .filter(|button| matches!(button.action, ButtonAction::PlayEpisode { .. }))
        .collect()
}

#[test]
fn all_fifteen_templates_paginate_1_6_12_24() {
    let templates = all_templates();
    assert_eq!(templates.len(), 15);
    for template in templates {
        for count in [1usize, 6, 12, 24] {
            let project = project_with_episodes(count);
            let result = apply_template(&project, template.id).expect("template apply");
            let expected_pages = count.div_ceil(template.episode_capacity as usize);
            let actual_pages = result
                .menus
                .iter()
                .filter(|menu| menu.template.is_some())
                .count();
            assert_eq!(
                actual_pages, expected_pages,
                "{} pages for {count} episodes",
                template.name
            );
            let episodes: Vec<_> = result
                .menus
                .iter()
                .filter(|menu| menu.template.is_some())
                .flat_map(|menu| menu.buttons.iter())
                .filter_map(|button| match button.action {
                    ButtonAction::PlayEpisode { episode } => Some(episode),
                    _ => None,
                })
                .collect();
            assert_eq!(episodes.len(), count, "{} episodes", template.name);
            assert_eq!(
                result
                    .diagnostics
                    .iter()
                    .filter(|d| d.code == "W_TEMPLATE_ASSET_MISSING")
                    .count(),
                0
            );
        }
    }
}

#[test]
fn all_templates_produce_a_valid_project() {
    for template in all_templates() {
        let mut project = project_with_episodes(6);
        let root_id = project.root_menu;
        project.menus.retain(|menu| menu.id == root_id);
        project.menus[0].buttons.clear();
        let result = apply_template(&project, template.id).expect("template");
        project.menus = result.menus;
        project.root_menu = result.root_menu;
        project.play_all.order = project.episodes.iter().map(|episode| episode.id).collect();
        let errors: Vec<_> = project
            .validate()
            .into_iter()
            .filter(kinodisc_domain::Diagnostic::is_error)
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect();
        assert!(errors.is_empty(), "{}: {errors:?}", template.name);
    }
}

#[test]
fn all_templates_compile_to_dvd_spumux_xml() {
    let mut base = project_with_episodes(6);
    let root_id = base.root_menu;
    base.menus.retain(|menu| menu.id == root_id);
    base.menus[0].buttons.clear();
    let background = std::env::temp_dir().join(format!(
        "kinodisc-template-background-{}.png",
        std::process::id()
    ));
    image::RgbaImage::from_pixel(720, 480, image::Rgba([24, 24, 24, 255]))
        .save(&background)
        .expect("background png");
    for template in all_templates() {
        let applied = apply_template(&base, template.id).expect("template");
        let mut project = base.clone();
        project.menus = applied.menus;
        project.root_menu = applied.root_menu;
        for (index, menu) in project
            .menus
            .iter()
            .filter(|menu| menu.template.is_some())
            .enumerate()
        {
            let buttons: Vec<CompiledButton> = menu
                .buttons
                .iter()
                .map(|button| CompiledButton {
                    id: button.id,
                    name: button.name.clone(),
                    rect: kinodisc_domain::PixelRect::new(
                        (button.rect.x.round() as u16) & !1,
                        (button.rect.y.round() as u16) & !1,
                        ((button.rect.x + button.rect.w).round() as u16) & !1,
                        ((button.rect.y + button.rect.h).round() as u16) & !1,
                    ),
                    style_kind: button.style_kind,
                    shape_mask: button.shape_mask,
                })
                .collect();
            let output_dir = std::env::temp_dir().join(format!(
                "kinodisc-template-spu-{}-{}-{index}",
                std::process::id(),
                template.id as u8
            ));
            let _ = std::fs::remove_dir_all(&output_dir);
            let layers = generate_menu_layers(&MenuLayerRequest {
                canvas: menu.canvas,
                background_source: background.clone(),
                buttons: buttons.clone(),
                palette: menu.subpicture.palette,
                scale_mode: menu.scale_mode,
                dim_percent: menu.dim_percent,
                blur_px: menu.blur_px,
                output_dir: output_dir.clone(),
            })
            .unwrap_or_else(|error| {
                panic!(
                    "{} layers: {error}; canvas={:?}; buttons={:?}",
                    template.name,
                    menu.canvas,
                    menu.buttons
                        .iter()
                        .map(|button| (button.name.clone(), button.rect))
                        .collect::<Vec<_>>()
                )
            });
            let xml = build_spumux_xml(
                &MenuCompileRequest {
                    menu_id: menu.id,
                    canvas: menu.canvas,
                    background_png: layers.background_png,
                    highlight_png: layers.highlight_png,
                    select_png: layers.select_png,
                    buttons,
                    force: menu.subpicture.force,
                    subpicture_start_ms: 0,
                },
                &menu.navigation(None),
            )
            .unwrap_or_else(|error| panic!("{} xml: {error}", template.name));
            assert!(xml.contains("<subpictures"), "{}", template.name);
            let _ = std::fs::remove_dir_all(output_dir);
        }
    }
    let _ = std::fs::remove_file(background);
}

#[test]
fn template_apply_preserves_custom_buttons_and_is_idempotent() {
    let mut project = project_with_episodes(6);
    let root = project.root_menu;
    let menu = project.menu(root).unwrap().clone();
    let custom = {
        let mut button = menu.buttons[0].clone();
        button.id = ButtonId::new();
        button.name = "Custom".to_string();
        button.action = ButtonAction::PlayAll;
        button
    };
    project
        .menus
        .iter_mut()
        .find(|menu| menu.id == root)
        .unwrap()
        .buttons
        .push(custom);
    let first = apply_template(&project, MenuTemplateId::CinemaDark).expect("first");
    assert!(first
        .menus
        .iter()
        .flat_map(|menu| menu.buttons.iter())
        .any(|button| button.name == "Custom"));
    let second_project = kinodisc_domain::Project {
        menus: first.menus.clone(),
        ..project
    };
    let second = apply_template(&second_project, MenuTemplateId::CinemaDark).expect("second");
    let first_pages: Vec<_> = first.menus.iter().map(|menu| menu.id).collect();
    let second_pages: Vec<_> = second.menus.iter().map(|menu| menu.id).collect();
    assert_eq!(first_pages, second_pages);
    assert!(second
        .menus
        .iter()
        .flat_map(|menu| menu.buttons.iter())
        .any(|button| button.name == "Custom"));
}

#[test]
fn template_switch_preserves_episode_thumbnails_and_actions() {
    let project = project_with_episodes(6);
    let cinema = apply_template(&project, MenuTemplateId::CinemaDark).expect("cinema");
    let switched_project = kinodisc_domain::Project {
        menus: cinema.menus.clone(),
        ..project
    };
    let minimalist =
        apply_template(&switched_project, MenuTemplateId::Minimalist).expect("minimalist");
    let episodes = episode_buttons(&minimalist.menus);
    assert_eq!(episodes.len(), 6);
    assert!(episodes.iter().all(|button| button.thumbnail.is_none()));
    assert!(minimalist.menus.iter().any(|menu| {
        menu.template
            .as_ref()
            .is_some_and(|page| page.template == MenuTemplateId::Minimalist)
    }));
}

fn title_of(menu: &kinodisc_domain::Menu) -> Option<String> {
    menu.items.iter().find_map(|item| match item {
        kinodisc_domain::MenuItem::Text(text) if text.id == "tpl:title" => Some(text.text.clone()),
        _ => None,
    })
}

/// 1.1 walkthrough: templates wrote their own name ("Retro CRT") as the
/// menu title, which then got burned onto the user's disc.
#[test]
fn template_title_is_the_project_name_and_keeps_a_typed_title() {
    let mut project = project_with_episodes(3);
    project.name = "Vacaciones 2026".to_string();
    let applied = apply_template(&project, MenuTemplateId::RetroCrt).expect("template");
    let mut next = project.clone();
    next.menus = applied.menus.clone();
    let root = next
        .menus
        .iter()
        .find(|menu| menu.id == next.root_menu)
        .expect("root");
    assert_eq!(title_of(root).as_deref(), Some("Vacaciones 2026"));

    // The user types a title, then switches template: the title stays.
    for menu in &mut next.menus {
        for item in &mut menu.items {
            if let kinodisc_domain::MenuItem::Text(text) = item {
                if text.id == "tpl:title" {
                    text.text = "Verano en la costa".to_string();
                }
            }
        }
    }
    let switched = apply_template(&next, MenuTemplateId::CinemaDark).expect("template");
    let root = switched
        .menus
        .iter()
        .find(|menu| menu.id == next.root_menu)
        .expect("root");
    assert_eq!(title_of(root).as_deref(), Some("Verano en la costa"));
}

/// One-episode discs (split "one per episode") left the only card in the
/// top-left corner; partially filled grids are centred now.
#[test]
fn a_single_episode_card_is_centred() {
    let project = project_with_episodes(1);
    for template in all_templates() {
        let applied = apply_template(&project, template.id).expect("template");
        let root = applied
            .menus
            .iter()
            .find(|menu| menu.id == project.root_menu)
            .expect("root");
        let button = episode_buttons(std::slice::from_ref(root))[0];
        let centre = button.rect.x + button.rect.w / 2.0;
        let middle = f64::from(root.canvas.width) / 2.0;
        assert!(
            (centre - middle).abs() <= 4.0,
            "{:?}: card centre {centre} vs {middle}",
            template.id
        );
    }
}
