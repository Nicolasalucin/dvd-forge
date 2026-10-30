use dvdforge_domain::{
    AspectRatio, ButtonAction, Chapter, EpisodeId, FrameRate, LanguageCode, LayerBreakMode, MenuId,
    MenuPlacement, MenuRole, PostAction, Project,
};
use dvdforge_toolchain::resolve_binary;
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
    #[error("VTS IFO no es válido: {0}")]
    InvalidVtsIfo(String),
    #[error("el layer break {0} no está alineado a bloques ECC de 16 sectores")]
    UnalignedLayerBreak(u64),
    #[error("no se encontró ningún PGC/celda para el layer break {0}")]
    LayerBreakOutsidePgc(u64),
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

/// Result of marking the layer-break cell in the VTS IFO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerBreakReport {
    pub requested_sector: u64,
    pub relative_sector: u64,
    pub cell_index: usize,
    pub cell_start_sector: u64,
    pub cell_end_sector: u64,
    pub exact: bool,
    pub mode: LayerBreakMode,
}

/// Marks the cell that contains the layer break as seamless (`SPLIP`).
///
/// dvdauthor does not expose a layer-break option. The physical break is set
/// by growisofs, but the VTS IFO must agree: DVD players use the SPLIP bit of
/// the cell category to decide whether the layer change is seamless. This
/// parser walks `VTS_PGCI` and patches the first cell whose VOB sector range
/// contains the requested break. Absolute image sectors are converted to VTS
/// VOB-relative sectors using `VTS_VOB start sector`.
pub fn apply_layer_break(
    dvd_dir: &Path,
    sector: u64,
    mode: LayerBreakMode,
) -> Result<LayerBreakReport, AuthorError> {
    if !sector.is_multiple_of(16) {
        return Err(AuthorError::UnalignedLayerBreak(sector));
    }
    let video_ts = dvd_dir.join("VIDEO_TS");
    let ifo_path = video_ts.join("VTS_01_0.IFO");
    let mut ifo = std::fs::read(&ifo_path)?;
    if ifo.len() < 0x100 || &ifo[..12] != b"DVDVIDEO-VTS" {
        return Err(AuthorError::InvalidVtsIfo(ifo_path.display().to_string()));
    }
    let vob_start = u64::from(be32(&ifo, 0xC4)?);
    let pgci = be32(&ifo, 0xCC)? as usize * 2048;
    if pgci + 8 > ifo.len() {
        return Err(AuthorError::InvalidVtsIfo(
            "puntero VTS_PGCI fuera del IFO".to_string(),
        ));
    }
    let pgc_count = be16(&ifo, pgci)? as usize;
    let relative_sector = sector.saturating_sub(vob_start);
    let mut selected: Option<(usize, usize, u64, u64, bool)> = None;
    let mut nearest: Option<(usize, usize, u64, u64, u64)> = None;

    for pgc_index in 0..pgc_count {
        let entry = pgci + 8 + pgc_index * 8;
        if entry + 8 > ifo.len() {
            break;
        }
        let pgc_offset = be32(&ifo, entry + 4)? as usize;
        let pgc = pgci.saturating_add(pgc_offset);
        if pgc + 0xEC > ifo.len() {
            continue;
        }
        let cell_count = ifo[pgc + 3] as usize;
        let cell_table_offset = be16(&ifo, pgc + 0xE8)? as usize;
        let cell_table = pgc.saturating_add(cell_table_offset);
        for cell_index in 0..cell_count {
            let cell = cell_table + cell_index * 0x18;
            if cell + 0x18 > ifo.len() {
                break;
            }
            let start = u64::from(be32(&ifo, cell + 8)?);
            let end = u64::from(be32(&ifo, cell + 0x14)?);
            let exact = relative_sector >= start && relative_sector <= end;
            let distance = relative_sector.abs_diff(start);
            if exact && selected.is_none() {
                selected = Some((pgc_index, cell_index, start, end, true));
            }
            if nearest
                .as_ref()
                .is_none_or(|(_, _, _, _, best)| distance < *best)
            {
                nearest = Some((pgc_index, cell_index, start, end, distance));
            }
        }
    }

    let (pgc_index, cell_index, cell_start, cell_end, exact) = selected
        .or_else(|| nearest.map(|(p, c, s, e, _)| (p, c, s, e, false)))
        .ok_or(AuthorError::LayerBreakOutsidePgc(sector))?;
    // Recompute the cell byte address for the selected PGC/cell.
    let entry = pgci + 8 + pgc_index * 8;
    let pgc = pgci + be32(&ifo, entry + 4)? as usize;
    let cell_table = pgc + be16(&ifo, pgc + 0xE8)? as usize;
    let cell = cell_table + cell_index * 0x18;
    // Cell category byte 0, bit 3: seamless playback linked in PCI.
    ifo[cell] |= 0x08;

    std::fs::write(&ifo_path, &ifo)?;
    let bup_path = video_ts.join("VTS_01_0.BUP");
    if bup_path.is_file() {
        std::fs::write(&bup_path, &ifo)?;
    }
    Ok(LayerBreakReport {
        requested_sector: sector,
        relative_sector,
        cell_index,
        cell_start_sector: cell_start,
        cell_end_sector: cell_end,
        exact,
        mode,
    })
}

fn be16(bytes: &[u8], offset: usize) -> Result<u16, AuthorError> {
    bytes
        .get(offset..offset + 2)
        .map(|slice| u16::from_be_bytes([slice[0], slice[1]]))
        .ok_or_else(|| AuthorError::InvalidVtsIfo(format!("offset {offset} fuera del IFO")))
}

fn be32(bytes: &[u8], offset: usize) -> Result<u32, AuthorError> {
    bytes
        .get(offset..offset + 4)
        .map(|slice| u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
        .ok_or_else(|| AuthorError::InvalidVtsIfo(format!("offset {offset} fuera del IFO")))
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
    // Without jumppads dvdauthor rejects VMGM chapter jumps and numbered
    // `jump titleset T menu N` targets ("That form of jumping is not allowed").
    xml.push_str("<dvdauthor jumppad=\"1\">\n");
    if request.project.disc.region_mask != 0 {
        xml.push_str(&format!(
            "  <!-- region_mask: {} -->\n",
            request.project.disc.region_mask
        ));
    }
    xml.push_str("  <vmgm>\n");
    if request.intro.is_some() {
        // The implicit first-play jumps to the `entry="title"` root menu and
        // would skip the intro PGC.
        xml.push_str("    <fpc>jump vmgm menu 1;</fpc>\n");
    }
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
                xml.push_str(&format!(
                    "      <audio format=\"{}\" channels=\"{}\"{} />\n",
                    xml_escape(&audio.format),
                    audio.channels,
                    lang_attribute(&audio.language)
                ));
                emitted += 1;
            }
            if emitted == 0 {
                xml.push_str("      <audio format=\"ac3\" channels=\"2\" />\n");
            }
            let widescreen = request.project.disc.aspect == AspectRatio::SixteenNine;
            for subtitle in &first.subtitles {
                // `normal` is illegal for 16:9 video: dvdauthor drops the stream.
                let streams = if widescreen {
                    format!(
                        "        <stream mode=\"widescreen\" id=\"{0}\" />\n        <stream mode=\"letterbox\" id=\"{0}\" />\n",
                        subtitle.stream_id
                    )
                } else {
                    format!(
                        "        <stream mode=\"normal\" id=\"{}\" />\n",
                        subtitle.stream_id
                    )
                };
                xml.push_str(&format!(
                    "      <subpicture{}>\n{streams}      </subpicture>\n",
                    lang_attribute(&subtitle.language)
                ));
            }
        }
        for (index, episode) in request.episodes.iter().enumerate() {
            xml.push_str("      <pgc>\n");
            let chapters = chapter_attribute(episode, request.project.disc.standard.frame_rate());
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
            binary: resolve_binary("dvdauthor"),
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
        let result = Command::from(dvdforge_toolchain::background_command(&self.binary))
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

/// dvdauthor only accepts two-letter ISO 639-1 codes in `lang`; `und` aborts
/// authoring, so undetermined tracks omit the attribute ("not specified").
fn lang_attribute(language: &str) -> String {
    let code = language_code(language);
    if code.len() == 2 {
        format!(" lang=\"{}\"", xml_escape(code))
    } else {
        String::new()
    }
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
            let first = request
                .project
                .play_all
                .order
                .first()
                .and_then(|episode| title_index.get(episode))
                .copied()
                .unwrap_or(1);
            format!("g{mode_gprm}=1; jump title {first};")
        }
        ButtonAction::GotoMenu { menu: target } => target_menu_command(request, *target)?,
        ButtonAction::PlayFirstPlay => "jump vmgm menu 1;".to_string(),
        ButtonAction::JumpTitleChapter { episode, chapter } => {
            let title = title_index
                .get(episode)
                .ok_or(AuthorError::UnknownEpisode(*episode))?;
            let chapter = dvd_chapter_number(request, *episode, *chapter);
            if matches!(menu.placement, MenuPlacement::Vmgm) {
                format!("g{mode_gprm}=0; jump titleset 1 title {title} chapter {chapter};")
            } else {
                format!("g{mode_gprm}=0; jump title {title} chapter {chapter};")
            }
        }
        ButtonAction::SetAudioTrack { track } => {
            format!("audio={track}; jump menu {current_menu_index};")
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
            // Only the first chapter menu after the root carries entry="ptt".
            let ptt_index = vtsm_menus_by_titleset(request)
                .get(&titleset)
                .and_then(|menus| {
                    menus
                        .iter()
                        .skip(1)
                        .position(|candidate| candidate.role == MenuRole::ChapterSelect)
                })
                .map(|position| position + 2);
            if index == 1 {
                Ok(format!("jump titleset {titleset} menu entry root;"))
            } else if Some(index) == ptt_index {
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
    let post_action = request
        .project
        .episode(current)
        .map(|episode| episode.post_action)
        .unwrap_or_default();
    let fallback = match post_action {
        PostAction::ReturnToMenu => format!("call vmgm menu {root_index};"),
        PostAction::NextEpisode => match request.episodes.get(title_index + 1) {
            Some(_) => format!("jump title {};", title_index + 2),
            None => format!("call vmgm menu {root_index};"),
        },
        PostAction::Stop => "exit;".to_string(),
    };
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
        return Ok(format!("g{mode_gprm}=0; {fallback}"));
    };
    if let Some(next) = order.get(position + 1) {
        let next_title = request
            .episodes
            .iter()
            .position(|episode| episode.episode == *next)
            .map(|index| index + 1)
            .ok_or(AuthorError::UnknownEpisode(*next))?;
        Ok(format!(
            "if (g{mode_gprm} == 1) {{ jump title {next_title}; }} else {{ {fallback} }}"
        ))
    } else {
        Ok(format!("g{mode_gprm}=0; {fallback}"))
    }
}

/// `Chapter::at_frame` is authored in disc-rate frames (29.97/25, the rate of
/// the encoded title), not in source frames.
fn chapter_attribute(episode: &EpisodeFile, fps: FrameRate) -> String {
    episode
        .chapters
        .iter()
        .filter(|chapter| chapter.at_frame > 0)
        .map(|chapter| format_chapter_time(chapter.at_frame, fps))
        .collect::<Vec<_>>()
        .join(",")
}

/// Maps an index into `Episode::chapters` to the authored DVD chapter number:
/// the start of the title is always chapter 1 and marks at frame 0 are not
/// emitted, so count the positive marks up to the target.
fn dvd_chapter_number(request: &AuthoringRequest<'_>, episode: EpisodeId, index: u32) -> usize {
    request
        .episodes
        .iter()
        .find(|file| file.episode == episode)
        .and_then(|file| {
            let target = file.chapters.get(index as usize)?;
            Some(
                1 + file
                    .chapters
                    .iter()
                    .filter(|chapter| chapter.at_frame > 0 && chapter.at_frame <= target.at_frame)
                    .count(),
            )
        })
        .unwrap_or(index as usize + 1)
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn vmgm_chapter_and_audio_actions_use_full_title_commands() {
        let project = fixture_project();
        let mut menus = rendered_menus(&project);
        let episode = project.episodes[0].id;
        menus[0].buttons = vec![
            AuthoredButton {
                name: "chapter".to_string(),
                action: ButtonAction::JumpTitleChapter {
                    episode,
                    chapter: 0,
                },
            },
            AuthoredButton {
                name: "audio".to_string(),
                action: ButtonAction::SetAudioTrack { track: 2 },
            },
        ];
        let episodes = episode_files(&project);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("g1=0; jump titleset 1 title 1 chapter 1;"));
        assert!(xml.contains("audio=2; jump menu 1;"));
    }

    #[test]
    fn title_post_honours_episode_post_action() {
        let mut project = fixture_project();
        let first = project.episodes[0].id;
        project.play_all.order = vec![first];
        project.episodes[0].post_action = PostAction::Stop;
        let menus = rendered_menus(&project);
        let mut episodes = episode_files(&project);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("g1=0; exit;"));

        project.episodes[0].post_action = PostAction::NextEpisode;
        let mut second = episodes[0].clone();
        second.episode = EpisodeId::from_u128(99);
        second.number = 2;
        second.path = PathBuf::from("/tmp/episode-2.mpg");
        episodes.push(second);
        let xml = build_dvdauthor_xml(&AuthoringRequest {
            project: &project,
            intro: None,
            menus: &menus,
            episodes: &episodes,
        })
        .expect("xml");
        assert!(xml.contains("g1=0; jump title 2;"));
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

    #[test]
    fn layer_break_marks_the_containing_vts_cell_and_backup() {
        fn put16(bytes: &mut [u8], offset: usize, value: u16) {
            bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
        }
        fn put32(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        }
        let root = std::env::temp_dir().join(format!(
            "dvdforge-layer-break-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let video_ts = root.join("VIDEO_TS");
        std::fs::create_dir_all(&video_ts).expect("video ts");
        let mut ifo = vec![0u8; 2048 * 5];
        ifo[..12].copy_from_slice(b"DVDVIDEO-VTS");
        put32(&mut ifo, 0xC4, 0);
        put32(&mut ifo, 0xCC, 1);
        let pgci = 2048;
        put16(&mut ifo, pgci, 1);
        put32(&mut ifo, pgci + 4, 0x30);
        put32(&mut ifo, pgci + 8 + 4, 0x10);
        let pgc = pgci + 0x10;
        ifo[pgc + 3] = 1;
        put16(&mut ifo, pgc + 0xE8, 0x20);
        let cell = pgc + 0x20;
        put32(&mut ifo, cell + 8, 100);
        put32(&mut ifo, cell + 0x14, 200);
        std::fs::write(video_ts.join("VTS_01_0.IFO"), &ifo).expect("ifo");
        std::fs::write(video_ts.join("VTS_01_0.BUP"), &ifo).expect("bup");

        let report = apply_layer_break(&root, 112, LayerBreakMode::Otp).expect("layer break");
        assert!(report.exact);
        assert_eq!(report.cell_index, 0);
        let patched = std::fs::read(video_ts.join("VTS_01_0.IFO")).expect("patched");
        assert_eq!(patched[cell] & 0x08, 0x08);
        assert_eq!(
            patched,
            std::fs::read(video_ts.join("VTS_01_0.BUP")).expect("patched bup")
        );
        assert!(matches!(
            apply_layer_break(&root, 113, LayerBreakMode::Otp),
            Err(AuthorError::UnalignedLayerBreak(113))
        ));
        let _ = std::fs::remove_dir_all(root);
    }
}
