use dvdforge_domain::{
    ButtonAction, ButtonId, ButtonShapeMask, ButtonStyleKind, CanvasSpec, Diagnostic, Episode,
    EpisodeId, FrameRate, LogicalRect, Menu, MenuBackground, MenuButton, MenuId, MenuItem,
    MenuPlacement, MenuRole, MenuTemplateId, MenuTemplatePage, NavOverride, Project,
    SubpictureColor, SubpicturePalette, SubpictureSpec, TextAlign, TextItem, ThumbnailSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MenuTemplateInfo {
    pub id: MenuTemplateId,
    pub name: String,
    pub description: String,
    pub columns: u8,
    pub rows: u8,
    pub episode_capacity: u16,
    pub thumbnail: bool,
    pub palette: SubpicturePalette,
    pub font_family: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateApplyResult {
    pub menus: Vec<Menu>,
    pub root_menu: MenuId,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Error)]
pub enum TemplateError {
    #[error("el proyecto no tiene un menú raíz")]
    MissingRoot,
    #[error("el proyecto no tiene episodios; no se puede aplicar una plantilla")]
    NoEpisodes,
    #[error("plantilla sin capacidad de episodios")]
    EmptyCapacity,
    #[error("el proyecto supera el máximo de 99 páginas de menú")]
    TooManyPages,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuTemplate {
    pub id: MenuTemplateId,
    pub name: &'static str,
    pub description: &'static str,
    pub columns: u8,
    pub rows: u8,
    pub thumbnail: bool,
    pub style_kind: ButtonStyleKind,
    pub shape_mask: ButtonShapeMask,
    pub palette: SubpicturePalette,
    pub font_family: &'static str,
    pub font_size: u16,
    pub font_weight: &'static str,
    pub text_color: &'static str,
    pub fill_color: &'static str,
    pub shadow_color: &'static str,
    pub margin_x: f32,
    pub margin_y: f32,
    pub gap: f32,
    pub header_ratio: f32,
    pub footer_ratio: f32,
    pub dim_percent: u8,
    pub blur_px: u8,
}

impl MenuTemplate {
    pub fn capacity(self) -> u16 {
        u16::from(self.columns) * u16::from(self.rows)
    }

    pub fn info(self) -> MenuTemplateInfo {
        MenuTemplateInfo {
            id: self.id,
            name: self.name.to_string(),
            description: self.description.to_string(),
            columns: self.columns,
            rows: self.rows,
            episode_capacity: self.capacity(),
            thumbnail: self.thumbnail,
            palette: self.palette,
            font_family: self.font_family.to_string(),
        }
    }
}

fn palette(background: [u8; 3], highlight: [u8; 3], accent: [u8; 3]) -> SubpicturePalette {
    SubpicturePalette {
        colors: [
            SubpictureColor::new(background, 0),
            SubpictureColor::new(highlight, 15),
            SubpictureColor::new(accent, 15),
            SubpictureColor::new([0, 0, 0], 15),
        ],
    }
}

fn templates() -> &'static [MenuTemplate] {
    static TEMPLATES: OnceLock<Vec<MenuTemplate>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        vec![
            MenuTemplate {
                id: MenuTemplateId::CinemaDark,
                name: "Cinema Dark",
                description: "Tarjetas grandes, fondo azul noche y acentos dorados.",
                columns: 2,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([9, 14, 24], [255, 255, 255], [234, 179, 8]),
                font_family: "Inter",
                font_size: 22,
                font_weight: "bold",
                text_color: "#FFFFFF",
                fill_color: "#111827",
                shadow_color: "#000000",
                margin_x: 0.07,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.12,
                footer_ratio: 0.12,
                dim_percent: 12,
                blur_px: 0,
            },
            MenuTemplate {
                id: MenuTemplateId::MultiPageTvGrid,
                name: "Multi-Page TV Series Grid",
                description: "Rejilla de seis episodios por página con paginación automática.",
                columns: 3,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([12, 24, 39], [255, 255, 255], [34, 211, 238]),
                font_family: "Inter",
                font_size: 19,
                font_weight: "bold",
                text_color: "#FFFFFF",
                fill_color: "#1E293B",
                shadow_color: "#000000",
                margin_x: 0.055,
                margin_y: 0.075,
                gap: 0.018,
                header_ratio: 0.10,
                footer_ratio: 0.10,
                dim_percent: 8,
                blur_px: 0,
            },
            MenuTemplate {
                id: MenuTemplateId::ClassicCriterion,
                name: "Classic Criterion",
                description: "Lista editorial de cuatro títulos, tipografía serif y marco clásico.",
                columns: 1,
                rows: 4,
                thumbnail: false,
                style_kind: ButtonStyleKind::ClassicFrame,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([20, 16, 12], [250, 245, 235], [212, 175, 55]),
                font_family: "Georgia",
                font_size: 24,
                font_weight: "normal",
                text_color: "#FAF5EB",
                fill_color: "#241E18",
                shadow_color: "#000000",
                margin_x: 0.10,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.14,
                footer_ratio: 0.10,
                dim_percent: 18,
                blur_px: 0,
            },
            MenuTemplate {
                id: MenuTemplateId::RetroCrt,
                name: "Retro CRT",
                description: "Píldoras redondeadas, neón verde y magenta, estética de monitor CRT.",
                columns: 2,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::GlassPill,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([8, 12, 18], [57, 255, 20], [255, 0, 180]),
                font_family: "Courier New",
                font_size: 21,
                font_weight: "bold",
                text_color: "#D9FFE8",
                fill_color: "#101820",
                shadow_color: "#39FF14",
                margin_x: 0.06,
                margin_y: 0.075,
                gap: 0.02,
                header_ratio: 0.11,
                footer_ratio: 0.11,
                dim_percent: 20,
                blur_px: 2,
            },
            MenuTemplate {
                id: MenuTemplateId::Minimalist,
                name: "Minimalist",
                description: "Ocho accesos compactos, blanco y negro, sin miniaturas.",
                columns: 4,
                rows: 2,
                thumbnail: false,
                style_kind: ButtonStyleKind::TextOnly,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([255, 255, 255], [17, 24, 39], [107, 114, 128]),
                font_family: "Inter",
                font_size: 17,
                font_weight: "normal",
                text_color: "#111827",
                fill_color: "#F8FAFC",
                shadow_color: "#FFFFFF",
                margin_x: 0.05,
                margin_y: 0.07,
                gap: 0.015,
                header_ratio: 0.11,
                footer_ratio: 0.10,
                dim_percent: 0,
                blur_px: 0,
            },
        ]
    })
}

pub fn all_templates() -> Vec<MenuTemplateInfo> {
    templates().iter().map(|template| template.info()).collect()
}

pub fn template(id: MenuTemplateId) -> Option<MenuTemplate> {
    templates()
        .iter()
        .copied()
        .find(|template| template.id == id)
}

pub fn apply_template(
    project: &Project,
    id: MenuTemplateId,
) -> Result<TemplateApplyResult, TemplateError> {
    let template = template(id).ok_or(TemplateError::MissingRoot)?;
    if template.capacity() == 0 {
        return Err(TemplateError::EmptyCapacity);
    }
    let root_id = project.root_menu;
    let root = project.menu(root_id).ok_or(TemplateError::MissingRoot)?;
    if project.episodes.is_empty() {
        return Err(TemplateError::NoEpisodes);
    }
    let mut episodes = project.episodes.clone();
    episodes.sort_by_key(|episode| episode.number);
    let capacity = template.capacity();
    let page_count = episodes.len().div_ceil(capacity as usize);
    if page_count > 99 {
        return Err(TemplateError::TooManyPages);
    }
    let mut diagnostics = Vec::new();
    if page_count > 32 {
        diagnostics.push(Diagnostic::warning(
            "W_TEMPLATE_CAPACITY",
            format!(
                "La plantilla requiere {page_count} páginas; se recomienda dividir el proyecto."
            ),
        ));
    }

    let old_pages: Vec<Menu> = {
        let mut pages: Vec<Menu> = project
            .menus
            .iter()
            .filter(|menu| menu.template.is_some())
            .cloned()
            .collect();
        pages.sort_by_key(|menu| {
            menu.template
                .as_ref()
                .map(|page| page.index)
                .unwrap_or(u16::MAX)
        });
        pages
    };
    let old_by_index = |index: usize| {
        old_pages.iter().find(|menu| {
            menu.template
                .as_ref()
                .is_some_and(|page| page.index as usize == index)
        })
    };

    let page_ids: Vec<MenuId> = (0..page_count)
        .map(|index| {
            if index == 0 {
                root_id
            } else {
                old_by_index(index)
                    .map(|menu| menu.id)
                    .unwrap_or_else(MenuId::new)
            }
        })
        .collect();

    let mut selected_episode_buttons: Vec<(EpisodeId, MenuButton)> = Vec::new();
    for episode in &episodes {
        if let Some(button) = find_episode_button(root, &old_pages, episode.id) {
            selected_episode_buttons.push((episode.id, button));
        }
    }
    let selected_ids: HashSet<ButtonId> = selected_episode_buttons
        .iter()
        .map(|(_, button)| button.id)
        .collect();

    let old_extras_all: Vec<Menu> = project
        .menus
        .iter()
        .filter(|menu| menu.name == "Extras" || menu.name.starts_with("Extras ·"))
        .cloned()
        .collect();
    let custom_items: Vec<MenuItem> = old_pages
        .iter()
        .chain(old_extras_all.iter())
        .flat_map(|menu| menu.items.iter())
        .filter(|item| !is_template_item(item))
        .cloned()
        .collect();
    let mut custom_buttons: Vec<MenuButton> = Vec::new();
    for menu in std::iter::once(root)
        .chain(old_pages.iter())
        .chain(old_extras_all.iter())
    {
        for button in &menu.buttons {
            if selected_ids.contains(&button.id)
                || is_generated_nav(button)
                || matches!(button.action, ButtonAction::PlayEpisode { .. })
            {
                continue;
            }
            if !custom_buttons
                .iter()
                .any(|candidate| candidate.id == button.id)
            {
                custom_buttons.push(button.clone());
            }
        }
    }

    let extras_count = custom_buttons.len().div_ceil(capacity as usize);
    if page_count + extras_count > 99 {
        return Err(TemplateError::TooManyPages);
    }
    if extras_count > 8 {
        diagnostics.push(Diagnostic::warning(
            "W_MENU_PAGE_OVERFLOW",
            format!(
                "La plantilla necesita {extras_count} páginas extra para botones personalizados."
            ),
        ));
    }
    let old_extras = old_extras_all;
    let extras_ids: Vec<MenuId> = (0..extras_count)
        .map(|index| {
            old_extras
                .get(index)
                .map(|menu| menu.id)
                .unwrap_or_else(MenuId::new)
        })
        .collect();

    let mut menus = Vec::new();
    for (index, page_episodes) in episodes.chunks(capacity as usize).enumerate() {
        let page_id = page_ids[index];
        let mut menu = if index == 0 {
            root.clone()
        } else {
            old_by_index(index)
                .cloned()
                .unwrap_or_else(|| page_shell(root, page_id, &template))
        };
        menu.id = page_id;
        menu.name = if index == 0 {
            if root.name.trim().is_empty() {
                template.name.to_string()
            } else {
                root.name.clone()
            }
        } else {
            format!("{} · {}", template.name, index + 1)
        };
        menu.role = if index == 0 {
            MenuRole::Root
        } else {
            MenuRole::EpisodeSelect
        };
        menu.placement = if index == 0 {
            MenuPlacement::Vmgm
        } else {
            MenuPlacement::Vtsm { titleset: 1 }
        };
        menu.canvas = root.canvas;
        menu.dim_percent = template.dim_percent;
        menu.blur_px = template.blur_px;
        menu.background = root.background.clone();
        menu.audio = root.audio.clone();
        menu.motion_config = root.motion_config.clone();
        menu.timing = root.timing;
        menu.transition = if index == 0 {
            root.transition.clone()
        } else {
            None
        };
        menu.template = Some(MenuTemplatePage {
            template: id,
            index: index as u16,
            count: page_count as u16,
            episode_capacity: capacity,
        });
        menu.subpicture = SubpictureSpec {
            palette: template.palette,
            force: true,
            ..root.subpicture
        };
        menu.items.retain(|item| !is_template_item(item));
        if index > 0 {
            menu.items.clear();
        }
        menu.items
            .extend(template_items(root, &template, index, page_count));
        if index == 0 {
            menu.items.extend(custom_items.clone());
        }
        menu.buttons.clear();
        let rects = grid_rects(menu.canvas, &template, page_episodes.len(), page_count > 1);
        for (slot, episode) in page_episodes.iter().enumerate() {
            let mut button = selected_episode_buttons
                .iter()
                .find(|(episode_id, _)| *episode_id == episode.id)
                .map(|(_, button)| button.clone())
                .unwrap_or_else(|| default_episode_button(episode, &template, rects[slot]));
            button.rect = rects[slot];
            button.action = ButtonAction::PlayEpisode {
                episode: episode.id,
            };
            apply_template_button_style(
                &mut button,
                &template,
                &episode.title,
                episode.video,
                false,
            );
            menu.buttons.push(button);
        }
        add_navigation(
            &mut menu,
            &template,
            index,
            page_count,
            page_ids.get(index.wrapping_sub(1)).copied(),
            page_ids.get(index + 1).copied(),
        );
        if index == 0 && !extras_ids.is_empty() {
            add_extras_link(&mut menu, &template, extras_ids[0]);
        }
        menu.buttons.sort_by(|a, b| {
            a.rect
                .y
                .total_cmp(&b.rect.y)
                .then_with(|| a.rect.x.total_cmp(&b.rect.x))
        });
        menu.buttons = even_buttons(menu.buttons);
        menus.push(menu);
    }

    for (index, page_buttons) in custom_buttons.chunks(capacity as usize).enumerate() {
        let page_id = extras_ids[index];
        let mut menu = old_extras
            .get(index)
            .cloned()
            .unwrap_or_else(|| page_shell(root, page_id, &template));
        menu.id = page_id;
        menu.name = if extras_count > 1 {
            format!("Extras · {}/{}", index + 1, extras_count)
        } else {
            "Extras".to_string()
        };
        menu.role = MenuRole::Submenu;
        menu.placement = MenuPlacement::Vtsm { titleset: 1 };
        menu.template = None;
        menu.transition = None;
        menu.items.retain(|item| !is_template_item(item));
        menu.items.push(MenuItem::Text(TextItem {
            id: "tpl:title".to_string(),
            text: menu.name.clone(),
            rect: LogicalRect::new(
                18.0,
                18.0,
                (root.canvas.width as f64 - 36.0).max(16.0),
                36.0,
            ),
            font_family: template.font_family.to_string(),
            font_size: template.font_size as f32,
            color: template.text_color.to_string(),
            align: TextAlign::Center,
        }));
        menu.buttons.clear();
        let rects = grid_rects(menu.canvas, &template, page_buttons.len(), extras_count > 1);
        for (slot, custom) in page_buttons.iter().enumerate() {
            let mut button = custom.clone();
            button.rect = rects[slot];
            let title = button.name.clone();
            let asset = button
                .thumbnail
                .as_ref()
                .map(|thumbnail| thumbnail.asset)
                .unwrap_or_else(|| root_background_asset(root));
            apply_template_button_style(&mut button, &template, &title, asset, true);
            menu.buttons.push(button);
        }
        add_navigation(
            &mut menu,
            &template,
            index,
            extras_count,
            extras_ids.get(index.wrapping_sub(1)).copied(),
            extras_ids.get(index + 1).copied(),
        );
        menu.buttons = even_buttons(menu.buttons);
        menus.push(menu);
    }

    for menu in &project.menus {
        if menu.id == root_id || menus.iter().any(|candidate| candidate.id == menu.id) {
            continue;
        }
        if menu.template.is_none() {
            menus.push(menu.clone());
        }
    }
    check_assets(project, root, &menus, &mut diagnostics);
    Ok(TemplateApplyResult {
        menus,
        root_menu: root_id,
        diagnostics,
    })
}

fn root_background_asset(root: &Menu) -> dvdforge_domain::AssetId {
    match &root.background {
        MenuBackground::Static { asset } | MenuBackground::VideoLoop { asset, .. } => *asset,
    }
}

fn find_episode_button(root: &Menu, pages: &[Menu], episode: EpisodeId) -> Option<MenuButton> {
    std::iter::once(root)
        .chain(pages.iter())
        .flat_map(|menu| menu.buttons.iter())
        .find(|button| matches!(button.action, ButtonAction::PlayEpisode { episode: id } if id == episode))
        .cloned()
}

fn is_generated_nav(button: &MenuButton) -> bool {
    matches!(button.action, ButtonAction::GotoMenu { .. })
        && (button.name.starts_with("‹")
            || button.name.starts_with("Siguiente")
            || button.name == "Extras")
}

fn page_shell(root: &Menu, id: MenuId, template: &MenuTemplate) -> Menu {
    let mut menu = root.clone();
    menu.id = id;
    menu.name = template.name.to_string();
    menu.role = MenuRole::EpisodeSelect;
    menu.placement = MenuPlacement::Vtsm { titleset: 1 };
    menu.background = root.background.clone();
    menu.audio = root.audio.clone();
    menu.motion_config = root.motion_config.clone();
    menu.timing = root.timing;
    menu.items = Vec::new();
    menu.buttons = Vec::new();
    menu.transition = None;
    menu.template = None;
    menu
}

fn default_episode_button(
    episode: &Episode,
    template: &MenuTemplate,
    rect: LogicalRect,
) -> MenuButton {
    MenuButton {
        id: ButtonId::new(),
        name: episode.title.clone(),
        rect,
        z: 0,
        action: ButtonAction::PlayEpisode {
            episode: episode.id,
        },
        thumbnail: template.thumbnail.then_some(ThumbnailSpec {
            asset: episode.video,
            in_frame: 0,
            duration_frames: 0,
            fps: FrameRate::FPS_23_976,
        }),
        label: None,
        style_kind: template.style_kind,
        font_family: template.font_family.to_string(),
        font_size: template.font_size,
        font_weight: template.font_weight.to_string(),
        text_color: template.text_color.to_string(),
        shadow_color: template.shadow_color.to_string(),
        fill_color: template.fill_color.to_string(),
        shape_mask: template.shape_mask,
        motion_preview: None,
        nav_override: NavOverride::default(),
        enabled: true,
    }
}

fn apply_template_button_style(
    button: &mut MenuButton,
    template: &MenuTemplate,
    title: &str,
    asset: dvdforge_domain::AssetId,
    preserve_thumbnail: bool,
) {
    button.style_kind = template.style_kind;
    button.shape_mask = template.shape_mask;
    button.font_family = template.font_family.to_string();
    button.font_size = template.font_size;
    button.font_weight = template.font_weight.to_string();
    button.text_color = template.text_color.to_string();
    button.shadow_color = template.shadow_color.to_string();
    button.fill_color = template.fill_color.to_string();
    if button.name.trim().is_empty() {
        button.name = title.to_string();
    }
    if template.thumbnail && button.thumbnail.is_none() {
        button.thumbnail = Some(ThumbnailSpec {
            asset,
            in_frame: 0,
            duration_frames: 0,
            fps: FrameRate::FPS_23_976,
        });
    } else if !template.thumbnail && !preserve_thumbnail {
        button.thumbnail = None;
    }
}

fn is_template_item(item: &MenuItem) -> bool {
    match item {
        MenuItem::Text(item) => item.id.starts_with("tpl:"),
        MenuItem::Image(item) => item.id.starts_with("tpl:"),
        MenuItem::VideoThumbnail(item) => item.id.starts_with("tpl:"),
        MenuItem::Shape(item) => item.id.starts_with("tpl:"),
    }
}

fn template_items(
    root: &Menu,
    template: &MenuTemplate,
    page: usize,
    count: usize,
) -> Vec<MenuItem> {
    let width = root.canvas.width as f64;
    vec![MenuItem::Text(TextItem {
        id: "tpl:title".to_string(),
        text: if count > 1 {
            format!("{} · {}/{}", template.name, page + 1, count)
        } else {
            template.name.to_string()
        },
        rect: LogicalRect::new(18.0, 18.0, (width - 36.0).max(16.0), 36.0),
        font_family: template.font_family.to_string(),
        font_size: template.font_size as f32,
        color: template.text_color.to_string(),
        align: TextAlign::Center,
    })]
}

fn grid_rects(
    canvas: CanvasSpec,
    template: &MenuTemplate,
    slots: usize,
    has_footer: bool,
) -> Vec<LogicalRect> {
    let width = canvas.width as f64;
    let height = canvas.height as f64;
    let margin_x = width * template.margin_x as f64;
    let margin_y = height * template.margin_y as f64;
    let gap = (width.min(height) * template.gap as f64).max(8.0);
    let header = height * template.header_ratio as f64;
    let footer = if has_footer {
        height * template.footer_ratio as f64
    } else {
        0.0
    };
    let columns = template.columns as usize;
    let rows = template.rows as usize;
    let cell_w = ((width - margin_x * 2.0 - gap * (columns.saturating_sub(1) as f64))
        / columns as f64)
        .max(16.0);
    let cell_h =
        ((height - margin_y * 2.0 - header - footer - gap * (rows.saturating_sub(1) as f64))
            / rows as f64)
            .max(16.0);
    (0..slots)
        .map(|index| {
            let column = index % columns;
            let row = index / columns;
            LogicalRect::new(
                margin_x + column as f64 * (cell_w + gap),
                margin_y + header + row as f64 * (cell_h + gap),
                cell_w,
                cell_h,
            )
        })
        .collect()
}

fn add_navigation(
    menu: &mut Menu,
    template: &MenuTemplate,
    page: usize,
    count: usize,
    previous: Option<MenuId>,
    next: Option<MenuId>,
) {
    if count <= 1 {
        return;
    }
    let width = menu.canvas.width as f64;
    let height = menu.canvas.height as f64;
    if let Some(target) = previous {
        menu.buttons.push(nav_button(
            "‹ Anterior",
            LogicalRect::new(18.0, height - 54.0, 180.0, 34.0),
            template,
            ButtonAction::GotoMenu { menu: target },
        ));
    }
    if let Some(target) = next {
        menu.buttons.push(nav_button(
            "Siguiente ›",
            LogicalRect::new(width - 198.0, height - 54.0, 180.0, 34.0),
            template,
            ButtonAction::GotoMenu { menu: target },
        ));
    }
    let _ = page;
}

fn add_extras_link(menu: &mut Menu, template: &MenuTemplate, extras: MenuId) {
    let width = menu.canvas.width as f64;
    let height = menu.canvas.height as f64;
    menu.buttons.push(nav_button(
        "Extras",
        LogicalRect::new((width - 140.0) / 2.0, height - 54.0, 140.0, 34.0),
        template,
        ButtonAction::GotoMenu { menu: extras },
    ));
}

fn nav_button(
    name: &str,
    rect: LogicalRect,
    template: &MenuTemplate,
    action: ButtonAction,
) -> MenuButton {
    MenuButton {
        id: ButtonId::new(),
        name: name.to_string(),
        rect,
        z: 0,
        action,
        thumbnail: None,
        label: None,
        style_kind: ButtonStyleKind::TextOnly,
        font_family: template.font_family.to_string(),
        font_size: 16,
        font_weight: "bold".to_string(),
        text_color: template.text_color.to_string(),
        shadow_color: template.shadow_color.to_string(),
        fill_color: template.fill_color.to_string(),
        shape_mask: ButtonShapeMask::Rectangle,
        motion_preview: None,
        nav_override: NavOverride::default(),
        enabled: true,
    }
}

fn even_buttons(buttons: Vec<MenuButton>) -> Vec<MenuButton> {
    buttons
        .into_iter()
        .map(|mut button| {
            button.rect = LogicalRect::new(
                (button.rect.x / 2.0).floor() * 2.0,
                (button.rect.y / 2.0).floor() * 2.0,
                (button.rect.w / 2.0).floor() * 2.0,
                (button.rect.h / 2.0).floor() * 2.0,
            );
            button
        })
        .collect()
}

fn check_assets(project: &Project, root: &Menu, menus: &[Menu], diagnostics: &mut Vec<Diagnostic>) {
    if project.asset(root_background_asset(root)).is_none() {
        diagnostics.push(Diagnostic::warning(
            "W_TEMPLATE_ASSET_MISSING",
            "La plantilla no encuentra el fondo del menú raíz.",
        ));
    }
    if let Some(audio) = &root.audio {
        if project.asset(audio.asset).is_none() {
            diagnostics.push(Diagnostic::warning(
                "W_TEMPLATE_ASSET_MISSING",
                "La plantilla no encuentra la música del menú raíz.",
            ));
        }
    }
    for menu in menus {
        for button in &menu.buttons {
            if let Some(thumbnail) = &button.thumbnail {
                if project.asset(thumbnail.asset).is_none() {
                    diagnostics.push(Diagnostic::warning(
                        "W_TEMPLATE_ASSET_MISSING",
                        format!(
                            "El botón «{}» referencia una miniatura inexistente.",
                            button.name
                        ),
                    ));
                }
            }
        }
    }
}
