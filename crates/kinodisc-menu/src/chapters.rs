//! Automatic scene-selection ("chapter") menus.
//!
//! For every episode with two or more chapters, Kinodisc builds one or more
//! pages in the style of the root menu's template: a button per chapter
//! (thumbnail at the chapter time), Previous/Next between pages and a Back
//! button. The root menu gets a "Scenes" button: straight to the chapter
//! menu when only one episode has chapters, otherwise to an index page with
//! one button per episode. Running it again replaces the generated menus
//! (role `ChapterSelect` and the index), leaving every other menu alone.

use crate::templates::{self, TemplateApplyResult, TemplateError};
use kinodisc_domain::{
    ButtonAction, Episode, FrameRate, Menu, MenuId, MenuItem, MenuPlacement, MenuRole, Project,
    TextAlign, TextItem, ThumbnailSpec, TvStandard, VideoConfig,
};
use std::collections::HashSet;

/// Name of the generated index page (also used to find it again).
pub const SCENES_INDEX_NAME: &str = "Escenas";
/// Label of the generated link on the root menu.
pub const SCENES_LINK_NAME: &str = "Escenas";

fn disc_fps(project: &Project) -> FrameRate {
    match &project.disc.video {
        VideoConfig::Avchd(config) => config.frame_rate,
        VideoConfig::Dvd(_) if project.disc.standard == TvStandard::Pal => FrameRate::PAL,
        VideoConfig::Dvd(_) => FrameRate::FPS_29_97,
    }
}

/// Menus created by [`apply_chapter_menus`], recognised by structure (not
/// by name, which the interface may translate): every `ChapterSelect` menu,
/// plus index pages whose buttons only lead to those menus or back to root.
pub fn generated_menu_ids(menus: &[Menu], root: MenuId) -> HashSet<MenuId> {
    let chapter_menus: HashSet<MenuId> = menus
        .iter()
        .filter(|menu| menu.role == MenuRole::ChapterSelect)
        .map(|menu| menu.id)
        .collect();
    let mut ids = chapter_menus.clone();
    for menu in menus {
        let is_index = menu.role == MenuRole::Submenu
            && menu.id != root
            && menu
                .buttons
                .iter()
                .any(|button| matches!(button.action, ButtonAction::GotoMenu { menu } if chapter_menus.contains(&menu)))
            && menu.buttons.iter().all(|button| match button.action {
                ButtonAction::GotoMenu { menu } => chapter_menus.contains(&menu) || menu == root,
                _ => false,
            });
        if is_index {
            ids.insert(menu.id);
        }
    }
    ids
}

/// The root-menu link to the scene menus, if any.
pub(crate) fn scenes_link(root: &Menu, generated: &HashSet<MenuId>) -> Option<(String, MenuId)> {
    root.buttons.iter().find_map(|button| match button.action {
        ButtonAction::GotoMenu { menu } if generated.contains(&menu) => {
            Some((button.name.clone(), menu))
        }
        _ => None,
    })
}

/// One entry per scene button: label, action and thumbnail frame.
fn scenes(episode: &Episode) -> Vec<(String, ButtonAction, u64)> {
    let mut list = Vec::new();
    if episode
        .chapters
        .first()
        .is_none_or(|chapter| chapter.at_frame > 0)
    {
        list.push((
            "Inicio".to_string(),
            ButtonAction::PlayEpisode {
                episode: episode.id,
            },
            0,
        ));
    }
    for (index, chapter) in episode.chapters.iter().enumerate() {
        let label = if chapter.title.trim().is_empty() {
            format!("Escena {}", list.len() + 1)
        } else {
            chapter.title.clone()
        };
        list.push((
            label,
            ButtonAction::JumpTitleChapter {
                episode: episode.id,
                chapter: index as u32,
            },
            chapter.at_frame,
        ));
    }
    list
}

/// Builds (or rebuilds) the scene menus. Episodes with fewer than two
/// chapters get none; if no episode qualifies, generated menus are removed
/// and the root link disappears.
pub fn apply_chapter_menus(project: &Project) -> Result<TemplateApplyResult, TemplateError> {
    let root = project
        .menu(project.root_menu)
        .ok_or(TemplateError::MissingRoot)?
        .clone();
    let template_id = root
        .template
        .as_ref()
        .map(|page| page.template)
        .unwrap_or_else(|| templates::all_templates()[0].id);
    let mut template = templates::template(template_id).ok_or(TemplateError::MissingRoot)?;
    template.font_size = templates::scaled_font_size(&template, root.canvas);
    let capacity = template.capacity() as usize;
    if capacity == 0 {
        return Err(TemplateError::EmptyCapacity);
    }
    let fps = disc_fps(project);

    let generated_before = generated_menu_ids(&project.menus, project.root_menu);
    let link_name = scenes_link(&root, &generated_before)
        .map(|(name, _)| name)
        .unwrap_or_else(|| SCENES_LINK_NAME.to_string());
    let mut menus: Vec<Menu> = project
        .menus
        .iter()
        .filter(|menu| !generated_before.contains(&menu.id))
        .cloned()
        .collect();
    // Remove every link to the previous scene menus from the kept menus.
    for menu in &mut menus {
        menu.buttons.retain(|button| {
            !matches!(button.action, ButtonAction::GotoMenu { menu } if generated_before.contains(&menu))
        });
    }

    let mut ordered: Vec<&Episode> = project.episodes.iter().collect();
    ordered.sort_by_key(|episode| episode.number);
    let with_scenes: Vec<&Episode> = ordered
        .into_iter()
        .filter(|episode| scenes(episode).len() >= 2)
        .collect();
    if with_scenes.is_empty() {
        return Ok(TemplateApplyResult {
            menus,
            root_menu: project.root_menu,
            diagnostics: Vec::new(),
        });
    }

    let index_id = (with_scenes.len() > 1).then(MenuId::new);
    let back_target = index_id.unwrap_or(project.root_menu);
    let mut first_pages = Vec::new();
    let mut generated = Vec::new();
    for episode in &with_scenes {
        let entries = scenes(episode);
        let pages: Vec<&[(String, ButtonAction, u64)]> = entries.chunks(capacity).collect();
        let ids: Vec<MenuId> = pages.iter().map(|_| MenuId::new()).collect();
        first_pages.push((episode.title.clone(), episode.video, ids[0]));
        for (page, chunk) in pages.iter().enumerate() {
            let mut menu = templates::page_shell(&root, ids[page], &template);
            menu.role = MenuRole::ChapterSelect;
            menu.placement = MenuPlacement::Vtsm { titleset: 1 };
            menu.name = if pages.len() > 1 {
                format!("Escenas · {} · {}/{}", episode.title, page + 1, pages.len())
            } else {
                format!("Escenas · {}", episode.title)
            };
            menu.items.push(title_item(&root, &template, &menu.name));
            let rects = templates::grid_rects(menu.canvas, &template, chunk.len(), true);
            for (slot, (label, action, frame)) in chunk.iter().enumerate() {
                let mut button =
                    templates::nav_button(label, rects[slot], &template, action.clone());
                button.style_kind = template.style_kind;
                button.shape_mask = template.shape_mask;
                button.font_size = template.font_size;
                button.font_weight = template.font_weight.to_string();
                button.thumbnail = template.thumbnail.then_some(ThumbnailSpec {
                    asset: episode.video,
                    in_frame: *frame,
                    duration_frames: 0,
                    fps,
                });
                menu.buttons.push(button);
            }
            templates::add_navigation(
                &mut menu,
                &template,
                page,
                pages.len(),
                page.checked_sub(1).map(|previous| ids[previous]),
                ids.get(page + 1).copied(),
            );
            templates::add_center_link(&mut menu, &template, "Volver", back_target);
            menu.buttons = templates::even_buttons(menu.buttons);
            generated.push(menu);
        }
    }

    let link_target = match index_id {
        None => first_pages[0].2,
        Some(index_id) => {
            let mut index = templates::page_shell(&root, index_id, &template);
            index.role = MenuRole::Submenu;
            index.placement = MenuPlacement::Vtsm { titleset: 1 };
            index.name = SCENES_INDEX_NAME.to_string();
            index
                .items
                .push(title_item(&root, &template, SCENES_INDEX_NAME));
            // The index lists episodes; more than one page of them is rare,
            // so overflow goes on extra index pages chained with Next.
            let rects = templates::grid_rects(
                index.canvas,
                &template,
                first_pages.len().min(capacity),
                true,
            );
            for (slot, (title, video, target)) in first_pages.iter().take(capacity).enumerate() {
                let mut button = templates::nav_button(
                    title,
                    rects[slot],
                    &template,
                    ButtonAction::GotoMenu { menu: *target },
                );
                button.style_kind = template.style_kind;
                button.shape_mask = template.shape_mask;
                button.font_size = template.font_size;
                button.thumbnail = template.thumbnail.then_some(ThumbnailSpec {
                    asset: *video,
                    in_frame: 0,
                    duration_frames: 0,
                    fps,
                });
                index.buttons.push(button);
            }
            templates::add_center_link(&mut index, &template, "Volver", project.root_menu);
            index.buttons = templates::even_buttons(index.buttons);
            generated.push(index);
            index_id
        }
    };
    if let Some(root_menu) = menus.iter_mut().find(|menu| menu.id == project.root_menu) {
        templates::add_corner_link(root_menu, &template, &link_name, link_target);
    }
    let mut diagnostics = Vec::new();
    if first_pages.len() > capacity {
        diagnostics.push(kinodisc_domain::Diagnostic::warning(
            "W_SCENES_INDEX_OVERFLOW",
            format!(
                "Solo los primeros {capacity} episodios aparecen en el índice de escenas; usa una plantilla con más casillas."
            ),
        ));
    }
    menus.extend(generated);
    Ok(TemplateApplyResult {
        menus,
        root_menu: project.root_menu,
        diagnostics,
    })
}

fn title_item(root: &Menu, template: &templates::MenuTemplate, text: &str) -> MenuItem {
    MenuItem::Text(TextItem {
        id: "tpl:title".to_string(),
        text: text.to_string(),
        rect: templates::title_rect(root.canvas),
        font_family: template.font_family.to_string(),
        font_size: template.font_size as f32,
        color: template.text_color.to_string(),
        align: TextAlign::Center,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kinodisc_domain::{Chapter, ChapterId, EpisodeId};

    fn project_with_chapters(chapters_per_episode: &[usize]) -> Project {
        let mut project = kinodisc_project::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/projects/minimal.json"),
        )
        .unwrap();
        let base = project.episodes[0].clone();
        project.episodes.clear();
        project.play_all.order.clear();
        for (index, count) in chapters_per_episode.iter().enumerate() {
            let mut episode = base.clone();
            if index > 0 {
                episode.id = EpisodeId::new();
            }
            episode.number = index as u16 + 1;
            episode.title = format!("Episodio {}", index + 1);
            episode.chapters = (0..*count)
                .map(|chapter| Chapter {
                    id: ChapterId::new(),
                    title: String::new(),
                    at_frame: chapter as u64 * 1_000,
                    thumbnail: None,
                })
                .collect();
            project.episodes.push(episode);
        }
        project
    }

    fn apply(project: &Project) -> Project {
        let result = apply_chapter_menus(project).unwrap();
        let mut updated = project.clone();
        updated.menus = result.menus;
        updated
    }

    #[test]
    fn one_episode_links_straight_to_its_scenes() {
        let project = apply(&project_with_chapters(&[4]));
        let scenes: Vec<&Menu> = project
            .menus
            .iter()
            .filter(|menu| menu.role == MenuRole::ChapterSelect)
            .collect();
        assert_eq!(scenes.len(), 1);
        let chapter_buttons = scenes[0]
            .buttons
            .iter()
            .filter(|button| matches!(button.action, ButtonAction::JumpTitleChapter { .. }))
            .count();
        assert_eq!(chapter_buttons, 4);
        let root = project.menu(project.root_menu).unwrap();
        let link = root
            .buttons
            .iter()
            .find(|button| button.name == SCENES_LINK_NAME)
            .expect("scenes link");
        assert!(matches!(link.action, ButtonAction::GotoMenu { menu } if menu == scenes[0].id));
        // Back goes to the root menu.
        assert!(scenes[0].buttons.iter().any(|button| button.name == "Volver"
            && matches!(button.action, ButtonAction::GotoMenu { menu } if menu == project.root_menu)));
        let errors: Vec<_> = project
            .validate()
            .into_iter()
            .filter(|d| d.severity == kinodisc_domain::Severity::Error)
            .map(|d| (d.code, d.message))
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn several_episodes_use_an_index_and_rerun_replaces_menus() {
        // Episode 2 has a single chapter at frame 0 → no scene menu.
        let project = apply(&project_with_chapters(&[3, 1, 30]));
        let count = |project: &Project| {
            project
                .menus
                .iter()
                .filter(|menu| menu.role == MenuRole::ChapterSelect)
                .count()
        };
        let pages_for_30 = 30usize.div_ceil(
            templates::template(templates::all_templates()[0].id)
                .unwrap()
                .capacity() as usize,
        );
        assert_eq!(count(&project), 1 + pages_for_30);
        assert!(project
            .menus
            .iter()
            .any(|menu| menu.name == SCENES_INDEX_NAME));
        let again = apply(&project);
        assert_eq!(count(&again), count(&project));
        let root = again.menu(again.root_menu).unwrap();
        assert_eq!(
            root.buttons
                .iter()
                .filter(|b| b.name == SCENES_LINK_NAME)
                .count(),
            1
        );
        // Every GotoMenu target exists.
        let ids: std::collections::HashSet<MenuId> = again.menus.iter().map(|m| m.id).collect();
        for menu in &again.menus {
            for button in &menu.buttons {
                if let ButtonAction::GotoMenu { menu } = button.action {
                    assert!(ids.contains(&menu));
                }
            }
        }
        // Removing the chapters removes the generated menus and the link.
        let mut plain = again.clone();
        for episode in &mut plain.episodes {
            episode.chapters.clear();
        }
        let cleared = apply(&plain);
        assert_eq!(count(&cleared), 0);
        assert!(!cleared
            .menus
            .iter()
            .any(|menu| menu.name == SCENES_INDEX_NAME));
    }

    #[test]
    fn translated_links_are_recognised_and_survive_template_changes() {
        let mut project = apply(&project_with_chapters(&[3, 4]));
        // The interface translates generated names.
        for menu in &mut project.menus {
            for button in &mut menu.buttons {
                if button.name == SCENES_LINK_NAME {
                    button.name = "Scenes".into();
                } else if button.name == "Volver" {
                    button.name = "Back".into();
                }
            }
        }
        let again = apply(&project);
        let root = again.menu(again.root_menu).unwrap();
        let links: Vec<_> = root
            .buttons
            .iter()
            .filter(|button| {
                matches!(button.action, ButtonAction::GotoMenu { menu }
                if generated_menu_ids(&again.menus, again.root_menu).contains(&menu))
            })
            .collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "Scenes");
        assert_eq!(
            again
                .menus
                .iter()
                .filter(|menu| menu.role == MenuRole::ChapterSelect)
                .count(),
            project
                .menus
                .iter()
                .filter(|menu| menu.role == MenuRole::ChapterSelect)
                .count()
        );
        // Re-applying a template keeps the link and never moves it to Extras.
        let template = templates::all_templates()[1].id;
        let result = templates::apply_template(&again, template).unwrap();
        let mut themed = again.clone();
        themed.menus = result.menus;
        let generated = generated_menu_ids(&themed.menus, themed.root_menu);
        let root = themed.menu(themed.root_menu).unwrap();
        assert_eq!(
            root.buttons
                .iter()
                .filter(|button| matches!(button.action, ButtonAction::GotoMenu { menu } if generated.contains(&menu)))
                .count(),
            1
        );
        for menu in themed
            .menus
            .iter()
            .filter(|menu| menu.name.starts_with("Extras"))
        {
            assert!(!menu.buttons.iter().any(|button| matches!(button.action,
                ButtonAction::GotoMenu { menu } if generated.contains(&menu))));
        }
    }
}
