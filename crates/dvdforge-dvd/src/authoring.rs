use dvdforge_domain::{
    ButtonAction, Chapter, EpisodeId, FrameRate, LanguageCode, MenuId, MenuPlacement, MenuRole,
    Project,
};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct AuthoredButton {
    pub name: String,
    pub action: ButtonAction,
}

#[derive(Debug, Clone)]
pub struct RenderedMenu {
    pub menu_id: MenuId,
    pub placement: MenuPlacement,
    pub role: MenuRole,
    pub mpeg: PathBuf,
    pub buttons: Vec<AuthoredButton>,
}

#[derive(Debug, Clone)]
pub struct AuthoredAudioTrack {
    pub language: String,
    pub format: String,
    pub channels: u8,
}

#[derive(Debug, Clone)]
pub struct AuthoredSubtitleTrack {
    pub language: String,
    pub stream_id: u8,
}

#[derive(Debug, Clone)]
pub struct EpisodeFile {
    pub episode: EpisodeId,
    pub number: u16,
    pub path: PathBuf,
    pub chapters: Vec<Chapter>,
    pub fps: FrameRate,
    pub audio: Vec<AuthoredAudioTrack>,
    pub subtitles: Vec<AuthoredSubtitleTrack>,
}

#[derive(Debug, Clone)]
pub struct AuthoringRequest<'a> {
    pub project: &'a Project,
    pub intro: Option<&'a Path>,
    pub menus: &'a [RenderedMenu],
    pub episodes: &'a [EpisodeFile],
}

#[derive(Debug, Error)]
pub enum AuthorError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("el proyecto no tiene episodios")]
    NoEpisodes,
    #[error("el menú raíz no está en los menús renderizados")]
    MissingRootMenu,
    #[error("el menú {0} no existe en el grafo de authoring")]
    UnknownMenu(MenuId),
    #[error("el episodio {0} no tiene título asociado")]
    UnknownEpisode(EpisodeId),
    #[error("GPRM {0} no válido para dvdauthor; usa 0..12")]
    InvalidGprm(u8),
    #[error("dvdauthor falló con código {code}: {stderr}")]
    DvdauthorFailed { code: i32, stderr: String },
    #[error("VIDEO_TS.IFO no es un VMG válido: {0}")]
    InvalidVmgIfo(String),
}

/// Offset of the prohibited-region mask inside VMGI_MAT (byte 1 of the
/// four-byte `vmg_category` field). Bits 0..5 correspond to regions 1..6.
pub const VMG_REGION_OFFSET: usize = 0x23;

/// Patches the region mask in VIDEO_TS.IFO and its backup.
///
/// dvdauthor does not expose this field in XML. The resulting byte is the
/// *prohibited* mask used by DVD players, so the public `region_mask` is
/// inverted: `1` means only region 1 is allowed (byte `0xFE`), and `0` means
/// region-free (byte `0x00`).
pub fn apply_region_mask(dvd_dir: &Path, region_mask: u8) -> Result<(), AuthorError> {
    let video_ts = dvd_dir.join("VIDEO_TS");
    let ifo = video_ts.join("VIDEO_TS.IFO");
    if !ifo.is_file() {
        return Err(AuthorError::InvalidVmgIfo(ifo.display().to_string()));
    }
    let prohibited = if region_mask == 0 { 0 } else { !region_mask };
    patch_vmg_region(&ifo, prohibited)?;
    let backup = video_ts.join("VIDEO_TS.BUP");
    if backup.is_file() {
        patch_vmg_region(&backup, prohibited)?;
    }
    Ok(())
}

fn patch_vmg_region(path: &Path, prohibited: u8) -> Result<(), AuthorError> {
    let mut bytes = std::fs::read(path)?;
    if bytes.len() <= VMG_REGION_OFFSET || &bytes[..12] != b"DVDVIDEO-VMG" {
        return Err(AuthorError::InvalidVmgIfo(path.display().to_string()));
    }
    bytes[VMG_REGION_OFFSET] = prohibited;
    std::fs::write(path, bytes)?;
    Ok(())
}

pub fn build_dvdauthor_xml(request: &AuthoringRequest<'_>) -> Result<String, AuthorError> {
    if request.episodes.is_empty() {
        return Err(AuthorError::NoEpisodes);
    }
    let mode_gprm = request.project.play_all.mode_gprm;
    if mode_gprm > 12 {
        return Err(AuthorError::InvalidGprm(mode_gprm));
    }

    let root_id = request.project.root_menu;
    let root_index = vmgm_menu_index(request, root_id)?;
    let title_index = title_indices(request.episodes);

    let mut xml = String::new();
    xml.push_str("<dvdauthor>\n");
    if request.project.disc.region_mask != 0 {
        xml.push_str(&format!(
            "  <!-- region_mask: {} -->\n",
            request.project.disc.region_mask
        ));
    }
    xml.push_str("  <vmgm>\n");
    xml.push_str("    <menus>\n");

    if let Some(intro) = request.intro {
        xml.push_str("      <pgc>\n");
        xml.push_str(&format!(
            "        <vob file=\"{}\" />\n",
            xml_escape(&intro.display().to_string())
        ));
        xml.push_str(&format!(
            "        <post>jump vmgm menu {root_index};</post>\n"
        ));
        xml.push_str("      </pgc>\n");
    }

    for (index, menu) in ordered_vmgm_menus(request).iter().enumerate() {
        let is_root = menu.menu_id == root_id;
        let entry = if is_root { " entry=\"title\"" } else { "" };
        xml.push_str(&format!("      <pgc{entry}>\n"));
        if is_root {
            if let Some(command) = region_check_command(request.project.disc.region_mask) {
                xml.push_str(&format!("        <pre>{}</pre>\n", xml_escape(&command)));
            }
        }
        xml.push_str(&format!(
            "        <vob file=\"{}\" />\n",
            xml_escape(&menu.mpeg.display().to_string())
        ));
        for button in &menu.buttons {
            let command = button_command(
                &button.action,
                request,
                menu,
                index + 1 + usize::from(request.intro.is_some()),
                root_index,
                &title_index,
                mode_gprm,
            )?;
            xml.push_str(&format!(
                "        <button name=\"{}\">{}</button>\n",
                xml_escape(&button.name),
                xml_escape(&command)
            ));
        }
        let post = if is_root {
            format!("jump menu {root_index};")
        } else {
            format!("jump vmgm menu {root_index};")
        };
        xml.push_str(&format!("        <post>{}</post>\n", xml_escape(&post)));
        xml.push_str("      </pgc>\n");
    }
    xml.push_str("    </menus>\n");
    xml.push_str("  </vmgm>\n");

    let vtsm_by_titleset = vtsm_menus_by_titleset(request);
    if !vtsm_by_titleset.is_empty() || !request.episodes.is_empty() {
        xml.push_str("  <titleset>\n");
        if !vtsm_by_titleset.is_empty() {
            xml.push_str("    <menus>\n");
            for menus in vtsm_by_titleset.values() {
                let mut ptt_assigned = false;
                for (index, menu) in menus.iter().enumerate() {
                    let entry = if index == 0 {
                        " entry=\"root\""
                    } else if menu.role == MenuRole::ChapterSelect && !ptt_assigned {
                        ptt_assigned = true;
                        " entry=\"ptt\""
                    } else {
                        ""
                    };
                    xml.push_str(&format!("      <pgc{entry}>\n"));
                    xml.push_str(&format!(
                        "        <vob file=\"{}\" />\n",
                        xml_escape(&menu.mpeg.display().to_string())
                    ));
                    for button in &menu.buttons {
                        let command = button_command(
                            &button.action,
                            request,
                            menu,
                            index + 1,
                            root_index,
                            &title_index,
                            mode_gprm,
                        )?;
                        xml.push_str(&format!(
                            "        <button name=\"{}\">{}</button>\n",
                            xml_escape(&button.name),
                            xml_escape(&command)
                        ));
                    }
                    xml.push_str(&format!("        <post>jump menu {};</post>\n", index + 1));
                    xml.push_str("      </pgc>\n");
                }
            }
            xml.push_str("    </menus>\n");
        }

        xml.push_str("    <titles>\n");
        if let Some(first) = request.episodes.first() {
            let mut emitted = 0usize;
            for audio in &first.audio {
                let language = language_code(&audio.language);
                xml.push_str(&format!(
                    "      <audio format=\"{}\" channels=\"{}\" lang=\"{}\" />\n",
                    xml_escape(&audio.format),
                    audio.channels,
                    xml_escape(language)
                ));
                emitted += 1;
            }
            if emitted == 0 {
                xml.push_str("      <audio format=\"ac3\" channels=\"2\" lang=\"und\" />\n");
            }
            for subtitle in &first.subtitles {
                xml.push_str(&format!(
                    "      <subpicture lang=\"{}\">\n        <stream mode=\"normal\" id=\"{}\" />\n      </subpicture>\n",
                    xml_escape(language_code(&subtitle.language)),
                    subtitle.stream_id
                ));
            }
        }
        for (index, episode) in request.episodes.iter().enumerate() {
            xml.push_str("      <pgc>\n");
            let chapters = chapter_attribute(episode);
            if chapters.is_empty() {
                xml.push_str(&format!(
                    "        <vob file=\"{}\" />\n",
                    xml_escape(&episode.path.display().to_string())
                ));
            } else {
                xml.push_str(&format!(
                    "        <vob file=\"{}\" chapters=\"{}\" />\n",
                    xml_escape(&episode.path.display().to_string()),
                    xml_escape(&chapters)
                ));
            }
            let post = title_post(request, index, root_index, mode_gprm)?;
            xml.push_str(&format!("        <post>{}</post>\n", xml_escape(&post)));
            xml.push_str("      </pgc>\n");
        }
        xml.push_str("    </titles>\n");
        xml.push_str("  </titleset>\n");
    }

    xml.push_str("</dvdauthor>\n");
    Ok(xml)
}

pub fn write_dvdauthor_xml(request: &AuthoringRequest<'_>, path: &Path) -> Result<(), AuthorError> {
    let xml = build_dvdauthor_xml(request)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, xml)?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DvdAuthor {
    pub binary: PathBuf,
}

impl Default for DvdAuthor {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("dvdauthor"),
        }
    }
}

impl DvdAuthor {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub async fn author(&self, xml: &Path, output_dir: &Path) -> Result<(), AuthorError> {
        if output_dir.exists() {
            std::fs::remove_dir_all(output_dir)?;
        }
        std::fs::create_dir_all(output_dir)?;
        let result = Command::new(&self.binary)
            .arg("-o")
            .arg(output_dir)
            .arg("-x")
            .arg(xml)
            .stdout(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await?;
        if result.status.success() {
            Ok(())
        } else {
            let code = result.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            Err(AuthorError::DvdauthorFailed { code, stderr })
        }
    }
}

fn ordered_vmgm_menus<'a>(request: &AuthoringRequest<'a>) -> Vec<&'a RenderedMenu> {
    let mut menus: Vec<&RenderedMenu> = request
        .menus
        .iter()
        .filter(|menu| matches!(menu.placement, MenuPlacement::Vmgm))
        .collect();
    menus.sort_by_key(|menu| usize::from(menu.menu_id != request.project.root_menu));
    menus
}

fn vmgm_menu_index(request: &AuthoringRequest<'_>, menu_id: MenuId) -> Result<usize, AuthorError> {
    ordered_vmgm_menus(request)
        .iter()
        .position(|menu| menu.menu_id == menu_id)
        .map(|index| index + 1 + usize::from(request.intro.is_some()))
        .ok_or(AuthorError::MissingRootMenu)
}

fn vtsm_menus_by_titleset<'a>(
    request: &AuthoringRequest<'a>,
) -> BTreeMap<u8, Vec<&'a RenderedMenu>> {
    let mut map: BTreeMap<u8, Vec<&RenderedMenu>> = BTreeMap::new();
    for menu in request.menus {
        if let MenuPlacement::Vtsm { titleset } = menu.placement {
            map.entry(titleset).or_default().push(menu);
        }
    }
    for menus in map.values_mut() {
        menus.sort_by_key(|menu| match menu.role {
            MenuRole::EpisodeSelect => 0,
            MenuRole::ChapterSelect => 1,
            MenuRole::Root => 2,
            MenuRole::Submenu => 3,
        });
    }
    map
}

fn vtsm_menu_index(
    request: &AuthoringRequest<'_>,
    menu_id: MenuId,
) -> Result<(u8, usize), AuthorError> {
    let map = vtsm_menus_by_titleset(request);
    for (titleset, menus) in map {
        if let Some(index) = menus.iter().position(|menu| menu.menu_id == menu_id) {
            return Ok((titleset, index + 1));
        }
    }
    Err(AuthorError::UnknownMenu(menu_id))
}

fn language_code(language: &str) -> &'static str {
    LanguageCode::from_code(language)
        .unwrap_or(LanguageCode::Undetermined)
        .iso_639_1()
}

fn title_indices(episodes: &[EpisodeFile]) -> HashMap<EpisodeId, usize> {
    episodes
        .iter()
        .enumerate()
        .map(|(index, episode)| (episode.episode, index + 1))
        .collect()
}

fn button_command(
    action: &ButtonAction,
    request: &AuthoringRequest<'_>,
    menu: &RenderedMenu,
    current_menu_index: usize,
    root_index: usize,
    title_index: &HashMap<EpisodeId, usize>,
    mode_gprm: u8,
) -> Result<String, AuthorError> {
    Ok(match action {
        ButtonAction::PlayEpisode { episode } => {
            let title = title_index
                .get(episode)
                .ok_or(AuthorError::UnknownEpisode(*episode))?;
            format!("g{mode_gprm}=0; jump title {title};")
        }
        ButtonAction::PlayAll => {
            format!("g{mode_gprm}=1; jump title 1;")
        }
        ButtonAction::GotoMenu { menu: target } => target_menu_command(request, *target)?,
        ButtonAction::PlayFirstPlay => "jump vmgm menu 1;".to_string(),
        ButtonAction::JumpTitleChapter { episode, chapter } => {
            let title = title_index
                .get(episode)
                .ok_or(AuthorError::UnknownEpisode(*episode))?;
            format!(
                "g{mode_gprm}=0; jump title {title} chapter {};",
                chapter.saturating_add(1)
            )
        }
        ButtonAction::SetAudioTrack { track } => {
            if matches!(menu.placement, MenuPlacement::Vtsm { .. }) {
                format!("audio={track}; jump menu {current_menu_index};")
            } else {
                format!("jump menu {root_index};")
            }
        }
        ButtonAction::Resume => "resume;".to_string(),
        ButtonAction::Noop => format!("jump menu {current_menu_index};"),
    })
}

fn target_menu_command(
    request: &AuthoringRequest<'_>,
    target: MenuId,
) -> Result<String, AuthorError> {
    let menu = request
        .menus
        .iter()
        .find(|menu| menu.menu_id == target)
        .ok_or(AuthorError::UnknownMenu(target))?;
    match menu.placement {
        MenuPlacement::Vmgm => Ok(format!(
            "jump vmgm menu {};",
            vmgm_menu_index(request, target)?
        )),
        MenuPlacement::Vtsm { titleset } => {
            let (_titleset, index) = vtsm_menu_index(request, target)?;
            if index == 1 {
                Ok(format!("jump titleset {titleset} menu entry root;"))
            } else if menu.role == MenuRole::ChapterSelect {
                Ok(format!("jump titleset {titleset} menu entry ptt;"))
            } else {
                Ok(format!("jump titleset {titleset} menu {index};"))
            }
        }
    }
}

fn title_post(
    request: &AuthoringRequest<'_>,
    title_index: usize,
    root_index: usize,
    mode_gprm: u8,
) -> Result<String, AuthorError> {
    let current = request.episodes[title_index].episode;
    let order: Vec<EpisodeId> = if request.project.play_all.order.is_empty() {
        request
            .episodes
            .iter()
            .map(|episode| episode.episode)
            .collect()
    } else {
        request.project.play_all.order.clone()
    };
    let Some(position) = order.iter().position(|episode| *episode == current) else {
        return Ok(format!("g{mode_gprm}=0; call vmgm menu {root_index};"));
    };
    if let Some(next) = order.get(position + 1) {
        let next_title = request
            .episodes
            .iter()
            .position(|episode| episode.episode == *next)
            .map(|index| index + 1)
            .ok_or(AuthorError::UnknownEpisode(*next))?;
        Ok(format!(
            "if (g{mode_gprm} == 1) {{ jump title {next_title}; }} else {{ call vmgm menu {root_index}; }}"
        ))
    } else {
        Ok(format!("g{mode_gprm}=0; call vmgm menu {root_index};"))
    }
}

fn chapter_attribute(episode: &EpisodeFile) -> String {
    episode
        .chapters
        .iter()
        .filter(|chapter| chapter.at_frame > 0)
        .map(|chapter| format_chapter_time(chapter.at_frame, episode.fps))
        .collect::<Vec<_>>()
        .join(",")
}

fn format_chapter_time(frame: u64, fps: FrameRate) -> String {
    if !fps.is_valid() {
        return "00:00:00.000".to_string();
    }
    let total_ms = (frame as f64 * 1000.0 / fps.as_f64()).round() as u64;
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let seconds = (total_ms % 60_000) / 1000;
    let millis = total_ms % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// dvdauthor no expone el byte de región del IFO en su XML. Se inserta una
/// comprobación RCE equivalente usando SPRM20, que es la máscara que entrega
/// el reproductor (bits 0..5 = regiones 1..6).
fn region_check_command(mask: u8) -> Option<String> {
    if mask == 0 {
        None
    } else {
        Some(format!("if ((s20 & {mask}) == 0) {{ exit; }}"))
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvdforge_domain::{ButtonAction, EpisodeId, MenuId, MenuRole, Project};
    use std::path::PathBuf;

    fn fixture_project() -> Project {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/minimal.json");
        dvdforge_project::load_project(path).expect("fixture")
    }

    fn rendered_menus(project: &Project) -> Vec<RenderedMenu> {
        project
            .menus
            .iter()
            .map(|menu| RenderedMenu {
                menu_id: menu.id,
                placement: menu.placement,
                role: menu.role,
                mpeg: PathBuf::from(format!("/tmp/{}.mpg", menu.id)),
                buttons: menu
                    .buttons
                    .iter()
                    .enumerate()
                    .map(|(index, button)| AuthoredButton {
                        name: format!("b{}", index + 1),
                        action: button.action.clone(),
                    })
                    .collect(),
            })
            .collect()
    }

    fn episode_files(project: &Project) -> Vec<EpisodeFile> {
        project
            .episodes
            .iter()
            .map(|episode| EpisodeFile {
                episode: episode.id,
                number: episode.number,
                path: PathBuf::from(format!("/tmp/episode-{}.mpg", episode.number)),
                chapters: episode.chapters.clone(),
                fps: FrameRate::NTSC,
                audio: vec![AuthoredAudioTrack {
                    language: "spa".to_string(),
                    format: "ac3".to_string(),
                    channels: 2,
                }],
                subtitles: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn audio_languages_use_two_letter_iso_codes() {
        assert_eq!(language_code("spa"), "es");
        assert_eq!(language_code("eng"), "en");
        assert_eq!(language_code("fra"), "fr");
        assert_eq!(language_code("fre"), "fr");
        assert_eq!(language_code("deu"), "de");
        assert_eq!(language_code("ger"), "de");
        assert_eq!(language_code("zho"), "zh");
        assert_eq!(language_code("chi"), "zh");
        assert_eq!(language_code("eus"), "eu");
        assert_eq!(language_code("baq"), "eu");
        assert_eq!(language_code("es"), "es");
        assert_eq!(language_code("xyz"), "und");
    }

    #[test]
    fn region_mask_patches_vmg_category_and_backup() {
        let root = std::env::temp_dir().join(format!(
            "dvdforge-region-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let video_ts = root.join("VIDEO_TS");
        std::fs::create_dir_all(&video_ts).expect("dir");
        for name in ["VIDEO_TS.IFO", "VIDEO_TS.BUP"] {
            let mut bytes = vec![0u8; 2_048];
            bytes[..12].copy_from_slice(b"DVDVIDEO-VMG");
            std::fs::write(video_ts.join(name), bytes).expect("ifo");
        }
        apply_region_mask(&root, 0b0000_0101).expect("patch");
        for name in ["VIDEO_TS.IFO", "VIDEO_TS.BUP"] {
            let bytes = std::fs::read(video_ts.join(name)).expect("read");
            assert_eq!(bytes[VMG_REGION_OFFSET], 0xFA);
        }
        apply_region_mask(&root, 0).expect("region free");
        let bytes = std::fs::read(video_ts.join("VIDEO_TS.IFO")).expect("read");
        assert_eq!(bytes[VMG_REGION_OFFSET], 0x00);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn region_mask_injects_rce_command_in_root_menu() {
        let mut project = fixture_project();
        project.disc.region_mask = 0b0000_0101;
        let menus = rendered_menus(&project);
        let episodes = episode_files(&project);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("<!-- region_mask: 5 -->"));
        assert!(xml.contains("if ((s20 &amp; 5) == 0) { exit; }"));
    }

    #[test]
    fn builds_vmgm_and_vtsm_hierarchy() {
        let project = fixture_project();
        let menus = rendered_menus(&project);
        let episodes = episode_files(&project);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("<vmgm>"));
        assert!(xml.contains("<titleset>"));
        assert!(xml.contains("entry=\"title\""));
        assert!(xml.contains("jump titleset 1 menu entry root;"));
        assert!(xml.contains("jump title 1;"));
        assert!(xml.contains("<post>"));
        assert!(xml.contains("<audio format=\"ac3\" channels=\"2\" lang=\"es\" />"));
    }

    #[test]
    fn chapter_menu_gets_ptt_entry_and_chapter_jump() {
        let project = fixture_project();
        let mut menus = rendered_menus(&project);
        let episode = project.episodes[0].id;
        menus.push(RenderedMenu {
            menu_id: MenuId::from_u128(42),
            placement: MenuPlacement::Vtsm { titleset: 1 },
            role: MenuRole::ChapterSelect,
            mpeg: PathBuf::from("/tmp/chapters.mpg"),
            buttons: vec![AuthoredButton {
                name: "b1".to_string(),
                action: ButtonAction::JumpTitleChapter {
                    episode,
                    chapter: 0,
                },
            }],
        });
        let episodes = episode_files(&project);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("entry=\"ptt\""));
        assert!(xml.contains("jump title 1 chapter 1;"));
    }

    #[test]
    fn play_all_chains_titles_with_mode_gprm() {
        let mut project = fixture_project();
        let first = project.episodes[0].id;
        let second = EpisodeId::from_u128(5);
        project.play_all.order = vec![first, second];
        let menus = rendered_menus(&project);
        let episodes = vec![
            EpisodeFile {
                episode: first,
                number: 1,
                path: PathBuf::from("/tmp/episode-1.mpg"),
                chapters: vec![],
                fps: FrameRate::NTSC,
                audio: vec![AuthoredAudioTrack {
                    language: "spa".to_string(),
                    format: "ac3".to_string(),
                    channels: 2,
                }],
                subtitles: Vec::new(),
            },
            EpisodeFile {
                episode: second,
                number: 2,
                path: PathBuf::from("/tmp/episode-2.mpg"),
                chapters: vec![],
                fps: FrameRate::NTSC,
                audio: vec![AuthoredAudioTrack {
                    language: "spa".to_string(),
                    format: "ac3".to_string(),
                    channels: 2,
                }],
                subtitles: Vec::new(),
            },
        ];
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("if (g1 == 1) { jump title 2; }"));
        assert!(xml.contains("g1=0; call vmgm menu 1;"));
    }

    #[test]
    fn intro_becomes_first_play_and_root_is_second_menu() {
        let project = fixture_project();
        let menus = rendered_menus(&project);
        let episodes = episode_files(&project);
        let intro = Path::new("/tmp/intro.mpg");
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: Some(intro),
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("/tmp/intro.mpg"));
        assert!(xml.contains("jump vmgm menu 2;"));
    }

    #[test]
    fn chapter_timestamp_is_generated_from_frames() {
        assert_eq!(format_chapter_time(30_000, FrameRate::NTSC), "00:16:41.000");
    }
}
