//! Splits one project into several discs: either "as many episodes as fit
//! on each disc" (using the real bitrate planner) or "one disc per episode".
//!
//! Every part is an ordinary [`Project`] that renders on its own. Template
//! menus are regenerated for the episodes of the part (pagination included);
//! hand-made menus keep their design and lose only the buttons whose episode
//! is on another disc.

use kinodisc_domain::AssetId;
use kinodisc_domain::{
    ButtonAction, ButtonId, Diagnostic, EpisodeId, MediaProbe, Project, ProjectId,
};
use kinodisc_plan::calculate_bitrate_plan;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SplitStrategy {
    /// Fill each disc with as many episodes (in order) as the planner accepts.
    FitToDisc,
    /// One disc per episode.
    PerEpisode,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscPart {
    /// 1-based disc number.
    pub index: u16,
    pub count: u16,
    pub project: Project,
    /// Suggested image file name, e.g. `Mi serie - Disco 2.iso`.
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SplitResult {
    pub parts: Vec<DiscPart>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Splits `project`. Episodes keep their order (by number); an episode that
/// does not fit on an empty disc still gets its own disc and a warning.
pub fn split_project(
    project: &Project,
    probes: &HashMap<AssetId, MediaProbe>,
    strategy: SplitStrategy,
) -> Result<SplitResult, String> {
    if project.episodes.is_empty() {
        return Err("El proyecto no tiene episodios.".to_string());
    }
    let mut ordered = project.episodes.clone();
    ordered.sort_by_key(|episode| episode.number);
    let ids: Vec<EpisodeId> = ordered.iter().map(|episode| episode.id).collect();
    let mut diagnostics = Vec::new();

    let groups: Vec<Vec<EpisodeId>> = match strategy {
        SplitStrategy::PerEpisode => ids.iter().map(|id| vec![*id]).collect(),
        SplitStrategy::FitToDisc => {
            let fits = |group: &[EpisodeId]| {
                let candidate = subset(project, group, 1, 1);
                calculate_bitrate_plan(&candidate.project, probes)
                    .map(|plan| plan.feasible)
                    .unwrap_or(false)
            };
            let mut groups: Vec<Vec<EpisodeId>> = Vec::new();
            let mut current: Vec<EpisodeId> = Vec::new();
            for id in &ids {
                current.push(*id);
                if current.len() > 1 && !fits(&current) {
                    current.pop();
                    groups.push(std::mem::take(&mut current));
                    current.push(*id);
                }
                if current.len() == 1 && !fits(&current) {
                    let name = project
                        .episode(*id)
                        .map(|episode| episode.title.clone())
                        .unwrap_or_default();
                    diagnostics.push(Diagnostic::warning(
                        "W_SPLIT_EPISODE_TOO_LONG",
                        format!("«{name}» no cabe solo en un disco; revisa la calidad o el tipo de disco."),
                    ));
                }
            }
            if !current.is_empty() {
                groups.push(current);
            }
            groups
        }
    };

    let count = u16::try_from(groups.len()).map_err(|_| "Demasiados discos.".to_string())?;
    if count > 99 {
        return Err("La división necesita más de 99 discos.".to_string());
    }
    let custom_menus = project.menus.iter().any(|menu| {
        menu.template.is_none()
            && menu
                .buttons
                .iter()
                .any(|button| matches!(button.action, ButtonAction::PlayEpisode { .. }))
    });
    if custom_menus && count > 1 {
        diagnostics.push(Diagnostic::info(
            "I_SPLIT_CUSTOM_MENU",
            "Los menús hechos a mano conservan su diseño; en cada disco se quitan los botones de episodios que van en otro disco.",
        ));
    }
    let parts = groups
        .iter()
        .enumerate()
        .map(|(index, group)| subset(project, group, index as u16 + 1, count))
        .collect();
    Ok(SplitResult { parts, diagnostics })
}

fn subset(project: &Project, keep: &[EpisodeId], index: u16, count: u16) -> DiscPart {
    let keep_set: HashSet<EpisodeId> = keep.iter().copied().collect();
    let mut part = project.clone();
    part.id = ProjectId::new();
    part.episodes
        .retain(|episode| keep_set.contains(&episode.id));
    part.play_all.order.retain(|id| keep_set.contains(id));
    if count > 1 {
        part.name = format!("{} - Disco {index}", project.name);
        part.disc.volume_label = volume_label(&project.disc.volume_label, index);
    }

    // Template menus: regenerate for the episodes of this disc.
    let template = part
        .menu(part.root_menu)
        .and_then(|root| root.template.as_ref())
        .map(|page| page.template);
    let regenerated =
        template.and_then(|id| kinodisc_menu::templates::apply_template(&part, id).ok());
    if let Some(result) = regenerated {
        part.menus.retain(|menu| menu.template.is_none());
        part.menus
            .retain(|menu| !result.menus.iter().any(|new| new.id == menu.id));
        part.menus.extend(result.menus);
        part.root_menu = result.root_menu;
    }

    // Hand-made menus (and anything the template left): drop buttons and
    // chapter menus that point at episodes of other discs.
    let points_elsewhere = |action: &ButtonAction| match action {
        ButtonAction::PlayEpisode { episode } | ButtonAction::JumpTitleChapter { episode, .. } => {
            !keep_set.contains(episode)
        }
        _ => false,
    };
    let mut emptied: HashSet<kinodisc_domain::MenuId> = HashSet::new();
    for menu in &mut part.menus {
        let removed: HashSet<ButtonId> = menu
            .buttons
            .iter()
            .filter(|button| points_elsewhere(&button.action))
            .map(|button| button.id)
            .collect();
        if removed.is_empty() {
            continue;
        }
        menu.buttons.retain(|button| !removed.contains(&button.id));
        if menu.buttons.is_empty() {
            emptied.insert(menu.id);
        }
        for button in &mut menu.buttons {
            for link in [
                &mut button.nav_override.up,
                &mut button.nav_override.down,
                &mut button.nav_override.left,
                &mut button.nav_override.right,
            ] {
                if link.is_some_and(|target| removed.contains(&target)) {
                    *link = None;
                }
            }
        }
    }
    // A menu whose buttons all went to another disc cannot be navigated:
    // drop it (never the root) together with the buttons that open it.
    let root = part.root_menu;
    part.menus
        .retain(|menu| menu.id == root || !emptied.contains(&menu.id));
    let menu_ids: HashSet<_> = part.menus.iter().map(|menu| menu.id).collect();
    for menu in &mut part.menus {
        menu.buttons.retain(|button| match button.action {
            ButtonAction::GotoMenu { menu } => menu_ids.contains(&menu),
            _ => true,
        });
    }

    // "2 / 3" in the corner of the root menu so viewers know which disc
    // they are watching (language-neutral, regenerated on every split).
    if count > 1 {
        let root = part.root_menu;
        if let Some(menu) = part.menus.iter_mut().find(|menu| menu.id == root) {
            let width = f64::from(menu.canvas.width);
            let height = f64::from(menu.canvas.height);
            let scale = (height / 480.0).max(1.0);
            menu.items.retain(|item| !matches!(item, kinodisc_domain::MenuItem::Text(text) if text.id == "tpl:disc"));
            menu.items
                .push(kinodisc_domain::MenuItem::Text(kinodisc_domain::TextItem {
                    id: "tpl:disc".to_string(),
                    text: format!("{index} / {count}"),
                    rect: kinodisc_domain::LogicalRect::new(
                        (width * 0.70).round(),
                        (height - 52.0 * scale).round(),
                        (width * 0.25).round(),
                        (32.0 * scale).round(),
                    ),
                    font_family: "Arial".to_string(),
                    font_size: (20.0 * scale) as f32,
                    color: "#ffffff".to_string(),
                    align: kinodisc_domain::TextAlign::Right,
                }));
        }
    }

    let base = sanitize_file_name(&project.name);
    let file_name = if count > 1 {
        format!("{base} - Disco {index}.iso")
    } else {
        format!("{base}.iso")
    };
    DiscPart {
        index,
        count,
        project: part,
        file_name,
    }
}

/// `SERIE` + disc 2 → `SERIE_D2`, kept within the 32-byte ASCII limit.
fn volume_label(label: &str, index: u16) -> String {
    let suffix = format!("_D{index}");
    let clean: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let clean = if clean.is_empty() {
        "DISC".to_string()
    } else {
        clean
    };
    let keep = 32usize.saturating_sub(suffix.len()).min(clean.len());
    format!("{}{suffix}", &clean[..keep])
}

fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').to_string();
    if trimmed.is_empty() {
        "Disco".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Project {
        serde_json::from_str(include_str!("../../../fixtures/projects/avchd-dvd5.json"))
            .expect("fixture")
    }

    #[test]
    fn volume_labels_stay_within_limits() {
        assert_eq!(volume_label("MI_SERIE", 2), "MI_SERIE_D2");
        assert_eq!(volume_label("", 1), "DISC_D1");
        let long = "A".repeat(40);
        assert_eq!(volume_label(&long, 12).len(), 32);
        assert_eq!(sanitize_file_name("a/b: c?"), "a_b_ c_");
    }

    #[test]
    fn per_episode_split_keeps_only_its_episode_and_buttons() {
        let mut project = fixture();
        assert!(!project.episodes.is_empty());
        // Ensure at least two episodes.
        if project.episodes.len() < 2 {
            let mut second = project.episodes[0].clone();
            second.id = EpisodeId::new();
            second.number += 1;
            project.episodes.push(second);
        }
        let total = project.episodes.len();
        let result = split_project(&project, &HashMap::new(), SplitStrategy::PerEpisode).unwrap();
        assert_eq!(result.parts.len(), total);
        for part in &result.parts {
            assert_eq!(part.project.episodes.len(), 1);
            assert_eq!(part.count as usize, total);
            let kept = part.project.episodes[0].id;
            for menu in &part.project.menus {
                for button in &menu.buttons {
                    if let ButtonAction::PlayEpisode { episode } = button.action {
                        assert_eq!(episode, kept);
                    }
                }
            }
            assert!(part
                .project
                .disc
                .volume_label
                .ends_with(&format!("_D{}", part.index)));
            assert!(part
                .file_name
                .ends_with(&format!("Disco {}.iso", part.index)));
            // Discs with a menu show "N / M" on it (continuous discs have none).
            if let Some(root) = part.project.menu(part.project.root_menu) {
                assert!(root.items.iter().any(|item| matches!(item,
                    kinodisc_domain::MenuItem::Text(text) if text.text == format!("{} / {}", part.index, part.count))));
            }
            assert_ne!(part.project.id, project.id);
        }
    }

    #[test]
    fn fit_to_disc_covers_every_episode_once_in_order() {
        let mut project = fixture();
        let template = project.episodes[0].clone();
        for number in 2..=6u16 {
            let mut episode = template.clone();
            episode.id = EpisodeId::new();
            episode.number = number;
            project.episodes.push(episode);
        }
        let result = split_project(&project, &HashMap::new(), SplitStrategy::FitToDisc).unwrap();
        let covered: Vec<EpisodeId> = result
            .parts
            .iter()
            .flat_map(|part| part.project.episodes.iter().map(|episode| episode.id))
            .collect();
        let mut expected = project.episodes.clone();
        expected.sort_by_key(|episode| episode.number);
        let expected: Vec<EpisodeId> = expected.iter().map(|episode| episode.id).collect();
        assert_eq!(covered, expected);
        let errors = |project: &Project| {
            let mut codes: Vec<String> = project
                .validate()
                .into_iter()
                .filter(|diagnostic| diagnostic.severity == kinodisc_domain::Severity::Error)
                .map(|diagnostic| diagnostic.code)
                .collect();
            codes.sort();
            codes.dedup();
            codes
        };
        let original = errors(&project);
        for part in &result.parts {
            for code in errors(&part.project) {
                assert!(original.contains(&code), "split introduced {code}");
            }
        }
    }

    #[test]
    fn menus_emptied_by_the_split_are_removed_with_their_links() {
        let mut project: Project =
            serde_json::from_str(include_str!("../../../fixtures/projects/minimal.json")).unwrap();
        let mut second = project.episodes[0].clone();
        second.id = EpisodeId::new();
        second.number += 1;
        project.episodes.push(second.clone());
        let result = split_project(&project, &HashMap::new(), SplitStrategy::PerEpisode).unwrap();
        // Disc 2 holds the episode without a button: the episode menu would
        // be empty, so it disappears and nothing points to it any more.
        let disc2 = &result.parts[1].project;
        assert_eq!(disc2.episodes[0].id, second.id);
        assert!(disc2.menus.iter().all(|menu| !menu.buttons.is_empty()));
        let root = disc2.menu(disc2.root_menu).unwrap();
        assert!(root.items.iter().any(
            |item| matches!(item, kinodisc_domain::MenuItem::Text(text) if text.text == "2 / 2")
        ));
        let ids: HashSet<_> = disc2.menus.iter().map(|menu| menu.id).collect();
        for menu in &disc2.menus {
            for button in &menu.buttons {
                if let ButtonAction::GotoMenu { menu } = button.action {
                    assert!(ids.contains(&menu));
                }
            }
        }
    }
}
