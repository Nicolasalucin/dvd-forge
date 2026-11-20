//! On-disc "Audio and subtitles" setup menu, as on retail discs.
//!
//! One submenu in the style of the root menu's template: a button per audio
//! track, a button per subtitle track, "no subtitles" and Back, plus a link
//! on the root menu. Running it again replaces the generated menu and its
//! link, leaving every other menu alone.

use crate::chapters::title_item;
use crate::templates::{self, TemplateApplyResult, TemplateError};
use kinodisc_domain::{
    AuthoringMode, ButtonAction, ButtonStyleKind, Episode, LogicalRect, Menu, MenuId,
    MenuPlacement, MenuRole, Project, SubtitleTrack,
};
use std::collections::HashSet;

/// Name of the generated setup menu.
pub const SETUP_MENU_NAME: &str = "Ajustes";
/// Label of the generated link on the root menu.
pub const SETUP_LINK_NAME: &str = "Ajustes";
/// Label of the button that turns subtitles off.
pub const SUBTITLES_OFF_NAME: &str = "Sin subtítulos";
/// Label of the button that returns to the root menu.
pub const SETUP_BACK_NAME: &str = "Volver";
/// Prefix of the audio buttons ("Audio: Español").
pub const AUDIO_PREFIX: &str = "Audio";
/// Prefix of the subtitle buttons ("Subtítulos: Español").
pub const SUBTITLE_PREFIX: &str = "Subtítulos";

/// DVD-Video allows 8 audio and 32 subpicture streams per title.
const MAX_AUDIO_TRACKS: usize = 8;
const MAX_SUBTITLE_TRACKS: usize = 32;
/// Widest grid the setup menu uses before dropping subtitle buttons.
const MAX_COLUMNS: usize = 3;
/// A DVD menu holds 36 buttons; one is Back.
const MAX_ENTRIES: usize = 35;

/// Menus created by [`apply_setup_menu`], recognised by structure (not by
/// name, which the interface may translate): submenus with at least one
/// audio/subtitle button whose buttons only pick a track or return to root.
pub fn setup_menu_ids(menus: &[Menu], root: MenuId) -> HashSet<MenuId> {
    menus
        .iter()
        .filter(|menu| {
            menu.role == MenuRole::Submenu
                && menu.id != root
                && menu.buttons.iter().any(|button| {
                    matches!(
                        button.action,
                        ButtonAction::SetAudioTrack { .. } | ButtonAction::SetSubtitleTrack { .. }
                    )
                })
                && menu.buttons.iter().all(|button| match button.action {
                    ButtonAction::SetAudioTrack { .. } | ButtonAction::SetSubtitleTrack { .. } => {
                        true
                    }
                    ButtonAction::GotoMenu { menu } => menu == root,
                    _ => false,
                })
        })
        .map(|menu| menu.id)
        .collect()
}

/// The root-menu link to the setup menu, if any.
pub(crate) fn setup_link(root: &Menu, generated: &HashSet<MenuId>) -> Option<(String, MenuId)> {
    root.buttons.iter().find_map(|button| match button.action {
        ButtonAction::GotoMenu { menu } if generated.contains(&menu) => {
            Some((button.name.clone(), menu))
        }
        _ => None,
    })
}

/// Subtitle tracks that end up on the disc, in stream order (the render
/// skips bitmap tracks, except PGS on AVCHD/Blu-ray).
fn disc_subtitles<'a>(project: &Project, episode: &'a Episode) -> Vec<&'a SubtitleTrack> {
    let hd = matches!(project.disc.mode, AuthoringMode::Avchd);
    episode
        .subtitles
        .iter()
        .filter(|track| !track.bitmap || (hd && project.is_pgs_track(episode, track)))
        .take(MAX_SUBTITLE_TRACKS)
        .collect()
}

fn has_choice(project: &Project, episode: &Episode) -> bool {
    episode.audio.len() > 1 || !disc_subtitles(project, episode).is_empty()
}

fn track_name(label: &str, language: &str, number: usize) -> String {
    let (label, language) = (label.trim(), language.trim());
    if !label.is_empty() {
        label.to_string()
    } else if !language.is_empty() {
        language.to_string()
    } else {
        format!("Pista {number}")
    }
}

/// Button names are unique inside a menu (`E_DUPLICATE_BUTTON_NAME`).
fn unique_name(used: &mut HashSet<String>, name: String) -> String {
    if used.insert(name.clone()) {
        return name;
    }
    (2..)
        .map(|number| format!("{name} {number}"))
        .find(|candidate| used.insert(candidate.clone()))
        .unwrap_or(name)
}

/// One entry per track button: name and action.
fn entries(project: &Project, episode: &Episode) -> Vec<(String, ButtonAction)> {
    let mut used: HashSet<String> = [SUBTITLES_OFF_NAME, SETUP_BACK_NAME]
        .into_iter()
        .map(str::to_string)
        .collect();
    let mut list = Vec::new();
    for (index, track) in episode.audio.iter().take(MAX_AUDIO_TRACKS).enumerate() {
        let name = track_name(&track.label, &track.language, index + 1);
        list.push((
            unique_name(&mut used, format!("{AUDIO_PREFIX}: {name}")),
            ButtonAction::SetAudioTrack { track: index as u8 },
        ));
    }
    let subtitles = disc_subtitles(project, episode);
    for (index, track) in subtitles.iter().enumerate() {
        let name = track_name(&track.label, &track.language, index + 1);
        list.push((
            unique_name(&mut used, format!("{SUBTITLE_PREFIX}: {name}")),
            ButtonAction::SetSubtitleTrack {
                track: Some(index as u8),
            },
        ));
    }
    if !subtitles.is_empty() {
        list.push((
            SUBTITLES_OFF_NAME.to_string(),
            ButtonAction::SetSubtitleTrack { track: None },
        ));
    }
    list
}

/// Builds (or rebuilds) the setup menu from the first episode that offers a
/// choice (more than one audio track, or subtitles). When no episode does,
/// a previously generated setup menu and its link are removed and every
/// other menu is returned as it was.
pub fn apply_setup_menu(project: &Project) -> Result<TemplateApplyResult, TemplateError> {
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

    let generated_before = setup_menu_ids(&project.menus, project.root_menu);
    let link_name = setup_link(&root, &generated_before)
        .map(|(name, _)| name)
        .unwrap_or_else(|| SETUP_LINK_NAME.to_string());
    let mut menus: Vec<Menu> = project
        .menus
        .iter()
        .filter(|menu| !generated_before.contains(&menu.id))
        .cloned()
        .collect();
    // Remove every link to the previous setup menu from the kept menus.
    for menu in &mut menus {
        menu.buttons.retain(|button| {
            !matches!(button.action, ButtonAction::GotoMenu { menu } if generated_before.contains(&menu))
        });
    }

    let mut ordered: Vec<&Episode> = project.episodes.iter().collect();
    ordered.sort_by_key(|episode| episode.number);
    let Some(episode) = ordered
        .into_iter()
        .find(|episode| has_choice(project, episode))
    else {
        return Ok(TemplateApplyResult {
            menus,
            root_menu: project.root_menu,
            diagnostics: Vec::new(),
        });
    };

    // The same id on every run keeps the editor selection stable.
    let setup_id = project
        .menus
        .iter()
        .map(|menu| menu.id)
        .find(|id| generated_before.contains(id))
        .unwrap_or_else(MenuId::new);
    let mut menu = templates::page_shell(&root, setup_id, &template);
    menu.role = MenuRole::Submenu;
    menu.placement = MenuPlacement::Vtsm { titleset: 1 };
    menu.name = SETUP_MENU_NAME.to_string();
    menu.items
        .push(title_item(&root, &template, SETUP_MENU_NAME));

    // Text rows: as many as fit at a readable height, in up to three
    // columns. Rows the list does not use stay empty (the grid is centred).
    let (width, height) = (f64::from(menu.canvas.width), f64::from(menu.canvas.height));
    let k = (height / 480.0).max(1.0);
    let gap = (width.min(height) * f64::from(template.gap)).max(8.0);
    let available = height
        * (1.0
            - 2.0 * f64::from(template.margin_y)
            - f64::from(template.header_ratio)
            - f64::from(template.footer_ratio));
    let max_rows = (((available + gap) / (30.0 * k + gap)).floor() as usize).clamp(1, 12);
    let mut list = entries(project, episode);
    let capacity = (max_rows * MAX_COLUMNS).min(MAX_ENTRIES);
    let mut diagnostics = Vec::new();
    if list.len() > capacity {
        // Keep the last entry ("no subtitles"); drop the tracks before it.
        // Never silently: the existing page-overflow warning says so.
        diagnostics.push(kinodisc_domain::Diagnostic::warning(
            "W_MENU_PAGE_OVERFLOW",
            format!(
                "El menú de ajustes solo tiene sitio para {capacity} de las {} opciones de audio y subtítulos; las últimas pistas no aparecen.",
                list.len()
            ),
        ));
        let last = list.pop();
        list.truncate(capacity - 1);
        list.extend(last);
    }
    let columns = list.len().div_ceil(max_rows).clamp(1, MAX_COLUMNS);
    let rows = list.len().div_ceil(columns);
    let mut layout = template;
    layout.columns = columns as u8;
    // At least five rows of height, so two or three buttons are not huge.
    layout.rows = rows.max(max_rows.min(5)) as u8;
    let rects = templates::grid_rects(menu.canvas, &layout, list.len(), true);
    let style_kind = match template.style_kind {
        ButtonStyleKind::TextOnly | ButtonStyleKind::GlassPill => template.style_kind,
        _ => ButtonStyleKind::ClassicFrame,
    };
    for ((name, action), rect) in list.into_iter().zip(rects) {
        // A single column would span the whole screen: keep it compact.
        let rect = if columns == 1 {
            let w = rect.w.min((width * 0.6).round());
            LogicalRect::new(rect.x + (rect.w - w) / 2.0, rect.y, w, rect.h)
        } else {
            rect
        };
        let mut button = templates::nav_button(&name, rect, &template, action);
        button.style_kind = style_kind;
        menu.buttons.push(button);
    }
    templates::add_center_link(&mut menu, &template, SETUP_BACK_NAME, project.root_menu);
    menu.buttons = templates::even_buttons(menu.buttons);

    if let Some(root_menu) = menus.iter_mut().find(|menu| menu.id == project.root_menu) {
        let mut used: HashSet<String> = root_menu
            .buttons
            .iter()
            .map(|button| button.name.clone())
            .collect();
        let link_name = unique_name(&mut used, link_name);
        templates::add_corner_link(root_menu, &template, &link_name, setup_id);
    }
    menus.push(menu);
    Ok(TemplateApplyResult {
        menus,
        root_menu: project.root_menu,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kinodisc_domain::{Severity, SubtitleStyle};

    fn subtitle(label: &str, language: &str, bitmap: bool) -> SubtitleTrack {
        SubtitleTrack {
            source_stream: 0,
            source_path: Some(format!("subs/{language}.srt")),
            language: language.to_string(),
            label: label.to_string(),
            delay_ms: 0,
            is_forced: false,
            is_default: false,
            bitmap,
            style: SubtitleStyle::default(),
        }
    }

    /// The minimal DVD project with `audio` audio tracks and the given
    /// subtitle tracks on its only episode.
    fn project_with_tracks(audio: usize, subtitles: Vec<SubtitleTrack>) -> Project {
        let mut project = kinodisc_project::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/projects/minimal.json"),
        )
        .unwrap();
        let base = project.episodes[0].audio[0].clone();
        let names = [
            ("Español", "spa"),
            ("English", "eng"),
            ("", "fra"),
            ("", ""),
        ];
        project.episodes[0].audio = (0..audio)
            .map(|index| {
                let mut track = base.clone();
                let (label, language) = names[index % names.len()];
                track.label = label.to_string();
                track.language = language.to_string();
                track.default = index == 0;
                track
            })
            .collect();
        project.episodes[0].subtitles = subtitles;
        project
    }

    fn apply(project: &Project) -> Project {
        let result = apply_setup_menu(project).unwrap();
        assert!(result.diagnostics.is_empty());
        let mut updated = project.clone();
        updated.menus = result.menus;
        updated.root_menu = result.root_menu;
        updated
    }

    fn errors(project: &Project) -> Vec<(String, String)> {
        project
            .validate()
            .into_iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .map(|diagnostic| (diagnostic.code.to_string(), diagnostic.message))
            .collect()
    }

    /// Layout and naming errors only (an HD canvas on the DVD fixture has
    /// unrelated ones).
    fn layout_errors(project: &Project) -> Vec<(String, String)> {
        errors(project)
            .into_iter()
            .filter(|(code, _)| {
                // Prefixes without the "E_" (the i18n check reads quoted
                // E_/W_ strings in Rust as diagnostic codes).
                ["RECT", "BUTTON", "DUPLICATE_BUTTON", "TOO_MANY"]
                    .iter()
                    .any(|prefix| {
                        code.strip_prefix("E_")
                            .is_some_and(|rest| rest.starts_with(prefix))
                    })
            })
            .collect()
    }

    fn setup_menus(project: &Project) -> Vec<&Menu> {
        let ids = setup_menu_ids(&project.menus, project.root_menu);
        project
            .menus
            .iter()
            .filter(|menu| ids.contains(&menu.id))
            .collect()
    }

    /// Name and a compact description of the action of every button.
    fn summary(menu: &Menu) -> Vec<(String, String)> {
        menu.buttons
            .iter()
            .map(|button| {
                let action = match button.action {
                    ButtonAction::SetAudioTrack { track } => format!("audio {track}"),
                    ButtonAction::SetSubtitleTrack { track: Some(track) } => {
                        format!("subtitle {track}")
                    }
                    ButtonAction::SetSubtitleTrack { track: None } => "subtitle off".to_string(),
                    ButtonAction::GotoMenu { .. } => "menu".to_string(),
                    _ => "other".to_string(),
                };
                (button.name.clone(), action)
            })
            .collect()
    }

    #[test]
    fn two_audio_tracks_and_one_subtitle_give_the_expected_buttons() {
        let original = project_with_tracks(2, vec![subtitle("Español", "spa", false)]);
        let project = apply(&original);
        let menus = setup_menus(&project);
        assert_eq!(menus.len(), 1);
        let menu = menus[0];
        assert_eq!(menu.name, SETUP_MENU_NAME);
        assert_eq!(menu.role, MenuRole::Submenu);
        assert!(matches!(
            menu.placement,
            MenuPlacement::Vtsm { titleset: 1 }
        ));
        let expected = [
            ("Audio: Español", "audio 0"),
            ("Audio: English", "audio 1"),
            ("Subtítulos: Español", "subtitle 0"),
            ("Sin subtítulos", "subtitle off"),
            ("Volver", "menu"),
        ]
        .map(|(name, action)| (name.to_string(), action.to_string()));
        assert_eq!(summary(menu), expected);
        // Back goes to the root menu, and the root menu links to the setup menu.
        assert!(menu.buttons.iter().any(|button| button.name == "Volver"
            && matches!(button.action, ButtonAction::GotoMenu { menu } if menu == project.root_menu)));
        let root = project.menu(project.root_menu).unwrap();
        let links: Vec<_> = root
            .buttons
            .iter()
            .filter(|button| matches!(button.action, ButtonAction::GotoMenu { menu: target } if target == menu.id))
            .collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, SETUP_LINK_NAME);
        // Every other menu and root button is still there.
        assert_eq!(project.menus.len(), original.menus.len() + 1);
        assert_eq!(
            root.buttons.len(),
            original.menu(original.root_menu).unwrap().buttons.len() + 1
        );
        assert_eq!(errors(&original), Vec::new());
        assert_eq!(errors(&project), Vec::new());
    }

    #[test]
    fn rerun_replaces_the_setup_menu_and_keeps_a_translated_link() {
        let first = apply(&project_with_tracks(
            2,
            vec![subtitle("Español", "spa", false)],
        ));
        let again = apply(&first);
        assert_eq!(again.menus.len(), first.menus.len());
        assert_eq!(setup_menus(&again).len(), 1);
        assert_eq!(
            summary(setup_menus(&again)[0]),
            summary(setup_menus(&first)[0])
        );
        assert_eq!(setup_menus(&again)[0].id, setup_menus(&first)[0].id);
        for (before, after) in first.menus.iter().zip(&again.menus) {
            assert_eq!(before.id, after.id);
            assert_eq!(summary(before), summary(after));
            let rects = |menu: &Menu| -> Vec<LogicalRect> {
                menu.buttons.iter().map(|button| button.rect).collect()
            };
            assert_eq!(
                format!("{:?}", rects(before)),
                format!("{:?}", rects(after))
            );
        }
        assert_eq!(errors(&again), Vec::new());

        // The interface translates generated names; the link keeps its name
        // and the tracks follow the project.
        let mut translated = again.clone();
        for menu in &mut translated.menus {
            menu.name = menu.name.replace(SETUP_MENU_NAME, "Setup");
            for button in &mut menu.buttons {
                if button.name == SETUP_LINK_NAME {
                    button.name = "Setup".into();
                }
            }
        }
        translated.episodes[0]
            .subtitles
            .push(subtitle("English", "eng", false));
        let updated = apply(&translated);
        assert_eq!(setup_menus(&updated).len(), 1);
        assert_eq!(updated.menus.len(), first.menus.len());
        let setup = setup_menus(&updated)[0];
        assert!(setup
            .buttons
            .iter()
            .any(|button| button.name == "Subtítulos: English"
                && matches!(
                    button.action,
                    ButtonAction::SetSubtitleTrack { track: Some(1) }
                )));
        let root = updated.menu(updated.root_menu).unwrap();
        let links: Vec<_> = root
            .buttons
            .iter()
            .filter(|button| matches!(button.action, ButtonAction::GotoMenu { menu } if menu == setup.id))
            .collect();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].name, "Setup");
        assert_eq!(errors(&updated), Vec::new());
    }

    #[test]
    fn nothing_to_choose_leaves_the_project_unchanged() {
        // One audio track; a bitmap subtitle is not authored on DVD.
        let original = project_with_tracks(1, vec![subtitle("PGS", "spa", true)]);
        let result = apply_setup_menu(&original).unwrap();
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.root_menu, original.root_menu);
        assert_eq!(
            format!("{:?}", result.menus),
            format!("{:?}", original.menus)
        );

        // Tracks removed after a first run: the setup menu and its link go.
        let with_menu = apply(&project_with_tracks(2, Vec::new()));
        assert_eq!(setup_menus(&with_menu).len(), 1);
        let mut plain = with_menu.clone();
        plain.episodes[0].audio.truncate(1);
        let cleared = apply(&plain);
        assert!(setup_menus(&cleared).is_empty());
        assert_eq!(
            format!("{:?}", cleared.menus),
            format!("{:?}", original.menus)
        );
    }

    #[test]
    fn audio_only_discs_get_no_subtitle_buttons() {
        let project = apply(&project_with_tracks(2, Vec::new()));
        let expected = [
            ("Audio: Español", "audio 0"),
            ("Audio: English", "audio 1"),
            ("Volver", "menu"),
        ]
        .map(|(name, action)| (name.to_string(), action.to_string()));
        assert_eq!(summary(setup_menus(&project)[0]), expected);
        assert_eq!(errors(&project), Vec::new());
    }

    #[test]
    fn long_track_lists_stay_valid_on_every_template_and_canvas() {
        // Same label twice, empty labels, more tracks than rows: unique
        // names, no overlap, everything inside the canvas.
        let subtitles: Vec<SubtitleTrack> = (0..14)
            .map(|index| subtitle(if index % 2 == 0 { "Español" } else { "" }, "spa", false))
            .collect();
        for template in templates::all_templates() {
            for hd in [false, true] {
                let mut project = project_with_tracks(4, subtitles.clone());
                if hd {
                    for menu in &mut project.menus {
                        menu.canvas.width = 1920;
                        menu.canvas.height = 1080;
                    }
                }
                let applied = templates::apply_template(&project, template.id).unwrap();
                project.menus = applied.menus;
                project.root_menu = applied.root_menu;
                let before = layout_errors(&project);
                assert_eq!(before, Vec::new(), "{:?} hd={hd}", template.id);
                let result = apply_setup_menu(&project).unwrap();
                project.menus = result.menus;
                let menu = setup_menus(&project)[0];
                // Templates with little room drop the last subtitle tracks,
                // with a warning; audio, "off" and Back are always there.
                let all = 4 + 14 + 2;
                assert!(menu.buttons.len() <= all.min(36), "{:?}", template.id);
                assert_eq!(
                    menu.buttons.len() < all,
                    result
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "W_MENU_PAGE_OVERFLOW"),
                    "{:?} hd={hd}",
                    template.id
                );
                assert_eq!(
                    result.diagnostics.len(),
                    usize::from(menu.buttons.len() < all)
                );
                let names = summary(menu);
                for action in ["audio 0", "audio 3", "subtitle 0", "subtitle off", "menu"] {
                    assert!(names.iter().any(|(_, found)| found == action), "{action}");
                }
                let (w, h) = (f64::from(menu.canvas.width), f64::from(menu.canvas.height));
                for button in &menu.buttons {
                    let r = button.rect;
                    assert!(
                        r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= w && r.y + r.h <= h,
                        "{:?} hd={hd}: {} {r:?}",
                        template.id,
                        button.name
                    );
                }
                assert_eq!(layout_errors(&project), before, "{:?} hd={hd}", template.id);
            }
        }
    }

    #[test]
    fn template_changes_keep_the_setup_link_on_the_root_menu() {
        let project = apply(&project_with_tracks(
            2,
            vec![subtitle("Español", "spa", false)],
        ));
        let template = templates::all_templates()[1].id;
        let result = templates::apply_template(&project, template).unwrap();
        let mut themed = project.clone();
        themed.menus = result.menus;
        themed.root_menu = result.root_menu;
        let generated = setup_menu_ids(&themed.menus, themed.root_menu);
        assert_eq!(generated.len(), 1);
        let is_link = |button: &&kinodisc_domain::MenuButton| matches!(button.action, ButtonAction::GotoMenu { menu } if generated.contains(&menu));
        let root = themed.menu(themed.root_menu).unwrap();
        assert_eq!(root.buttons.iter().filter(is_link).count(), 1);
        for menu in themed.menus.iter().filter(|menu| menu.id != root.id) {
            assert_eq!(menu.buttons.iter().filter(is_link).count(), 0);
        }
        assert_eq!(errors(&themed), Vec::new());
    }
}
