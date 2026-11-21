use kinodisc_domain::{
    ButtonAction, ButtonId, ButtonShapeMask, ButtonStyleKind, CanvasSpec, Diagnostic, Episode,
    EpisodeId, FrameRate, LogicalRect, Menu, MenuBackground, MenuButton, MenuId, MenuItem,
    MenuPlacement, MenuRole, MenuTemplateId, MenuTemplatePage, NavOverride, Project, ShapeItem,
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
    /// Colour of the page title (differs from the buttons' text when the
    /// title sits on a band).
    pub title_color: &'static str,
    pub fill_color: &'static str,
    pub shadow_color: &'static str,
    pub margin_x: f32,
    pub margin_y: f32,
    pub gap: f32,
    pub header_ratio: f32,
    pub footer_ratio: f32,
    pub dim_percent: u8,
    pub blur_px: u8,
    /// Decorative shapes drawn over the template's background colour.
    pub decor: Decor,
}

/// Template decoration, drawn as shape items (`tpl:decor-*`) so the editor,
/// the disc and the CLI compositor all show the same thing. Sizes are
/// fractions of the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decor {
    None,
    /// A thin frame inset from the edges.
    Frame {
        color: &'static str,
    },
    /// Four L-shaped corner brackets.
    Corners {
        color: &'static str,
    },
    /// A band across the top (behind the title).
    TopBand {
        color: &'static str,
    },
    /// Dark bands with sprocket holes at the top and bottom.
    Filmstrip {
        band: &'static str,
        hole: &'static str,
    },
    /// Small coloured squares scattered around the title.
    Confetti {
        colors: [&'static str; 4],
    },
    /// A ground band along the bottom.
    Horizon {
        color: &'static str,
    },
    /// Three vertical accent stripes on the left.
    Stripes {
        color: &'static str,
    },
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
                title_color: "#FFFFFF",
                fill_color: "#111827",
                shadow_color: "#000000",
                margin_x: 0.07,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.12,
                footer_ratio: 0.12,
                dim_percent: 12,
                blur_px: 0,
                decor: Decor::None,
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
                title_color: "#FFFFFF",
                fill_color: "#1E293B",
                shadow_color: "#000000",
                margin_x: 0.055,
                margin_y: 0.075,
                gap: 0.018,
                header_ratio: 0.10,
                footer_ratio: 0.10,
                dim_percent: 8,
                blur_px: 0,
                decor: Decor::None,
            },
            MenuTemplate {
                id: MenuTemplateId::ClassicCriterion,
                name: "Classic Editorial",
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
                title_color: "#FAF5EB",
                fill_color: "#241E18",
                shadow_color: "#000000",
                margin_x: 0.10,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.14,
                footer_ratio: 0.10,
                dim_percent: 18,
                blur_px: 0,
                decor: Decor::Frame { color: "#D4AF37" },
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
                title_color: "#D9FFE8",
                fill_color: "#101820",
                shadow_color: "#39FF14",
                margin_x: 0.06,
                margin_y: 0.075,
                gap: 0.02,
                header_ratio: 0.11,
                footer_ratio: 0.11,
                dim_percent: 20,
                blur_px: 2,
                decor: Decor::None,
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
                title_color: "#111827",
                fill_color: "#F8FAFC",
                shadow_color: "#FFFFFF",
                margin_x: 0.05,
                margin_y: 0.07,
                gap: 0.015,
                header_ratio: 0.11,
                footer_ratio: 0.10,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::None,
            },
            MenuTemplate {
                id: MenuTemplateId::Wedding,
                name: "Boda",
                description: "Marfil y dorado, tipografía serif y esquinas finas.",
                columns: 1,
                rows: 4,
                thumbnail: true,
                style_kind: ButtonStyleKind::ClassicFrame,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([245, 240, 230], [59, 47, 47], [176, 141, 87]),
                font_family: "Georgia",
                font_size: 21,
                font_weight: "normal",
                text_color: "#3B2F2F",
                title_color: "#3B2F2F",
                fill_color: "#FFFFFF",
                shadow_color: "#FFFFFF",
                margin_x: 0.16,
                margin_y: 0.09,
                gap: 0.03,
                header_ratio: 0.14,
                footer_ratio: 0.11,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Corners { color: "#B08D57" },
            },
            MenuTemplate {
                id: MenuTemplateId::Birthday,
                name: "Cumpleaños",
                description: "Violeta festivo con confeti de colores.",
                columns: 3,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([43, 16, 85], [255, 255, 255], [255, 209, 102]),
                font_family: "Trebuchet MS",
                font_size: 19,
                font_weight: "bold",
                text_color: "#FFFFFF",
                title_color: "#FFFFFF",
                fill_color: "#3C1A78",
                shadow_color: "#000000",
                margin_x: 0.055,
                margin_y: 0.08,
                gap: 0.02,
                header_ratio: 0.13,
                footer_ratio: 0.1,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Confetti {
                    colors: ["#FFD166", "#EF476F", "#06D6A0", "#4CC9F0"],
                },
            },
            MenuTemplate {
                id: MenuTemplateId::Travel,
                name: "Viaje",
                description: "Azul petróleo y arena, como un horizonte de playa.",
                columns: 3,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([14, 59, 67], [255, 255, 255], [233, 196, 106]),
                font_family: "Verdana",
                font_size: 17,
                font_weight: "bold",
                text_color: "#FFFFFF",
                title_color: "#FFFFFF",
                fill_color: "#0B2E34",
                shadow_color: "#000000",
                margin_x: 0.055,
                margin_y: 0.075,
                gap: 0.02,
                header_ratio: 0.12,
                footer_ratio: 0.12,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Horizon { color: "#E9C46A" },
            },
            MenuTemplate {
                id: MenuTemplateId::Sports,
                name: "Deportes",
                description: "Negro y rojo intenso, letra condensada y franjas.",
                columns: 2,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([16, 20, 24], [255, 255, 255], [230, 57, 70]),
                font_family: "Impact",
                font_size: 22,
                font_weight: "normal",
                text_color: "#FFFFFF",
                title_color: "#FFFFFF",
                fill_color: "#1B2128",
                shadow_color: "#000000",
                margin_x: 0.1,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.12,
                footer_ratio: 0.11,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Stripes { color: "#E63946" },
            },
            MenuTemplate {
                id: MenuTemplateId::Holidays,
                name: "Vacaciones",
                description: "Claro y soleado, con una franja naranja.",
                columns: 3,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([255, 247, 230], [38, 70, 83], [255, 140, 66]),
                font_family: "Trebuchet MS",
                font_size: 18,
                font_weight: "bold",
                text_color: "#FFFFFF",
                title_color: "#264653",
                fill_color: "#264653",
                shadow_color: "#000000",
                margin_x: 0.055,
                margin_y: 0.08,
                gap: 0.02,
                header_ratio: 0.14,
                footer_ratio: 0.1,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::TopBand { color: "#FF8C42" },
            },
            MenuTemplate {
                id: MenuTemplateId::OldFilm,
                name: "Cine antiguo",
                description: "Sepia oscuro con bordes de película.",
                columns: 1,
                rows: 4,
                thumbnail: true,
                style_kind: ButtonStyleKind::ClassicFrame,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([26, 22, 18], [245, 230, 200], [200, 160, 90]),
                font_family: "Georgia",
                font_size: 21,
                font_weight: "normal",
                text_color: "#F5E6C8",
                title_color: "#F5E6C8",
                fill_color: "#2A231C",
                shadow_color: "#000000",
                margin_x: 0.16,
                margin_y: 0.15,
                gap: 0.025,
                header_ratio: 0.12,
                footer_ratio: 0.1,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Filmstrip {
                    band: "#0A0806",
                    hole: "#3A3128",
                },
            },
            MenuTemplate {
                id: MenuTemplateId::Nature,
                name: "Naturaleza",
                description: "Verde bosque con una franja de pradera.",
                columns: 2,
                rows: 2,
                thumbnail: true,
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([31, 58, 43], [255, 255, 255], [149, 213, 178]),
                font_family: "Palatino Linotype",
                font_size: 21,
                font_weight: "bold",
                text_color: "#FFFFFF",
                title_color: "#FFFFFF",
                fill_color: "#183024",
                shadow_color: "#000000",
                margin_x: 0.08,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.12,
                footer_ratio: 0.12,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Horizon { color: "#2D6A4F" },
            },
            MenuTemplate {
                id: MenuTemplateId::Documentary,
                name: "Documental",
                description: "Lista sobria de seis títulos sobre gris claro.",
                columns: 1,
                rows: 6,
                thumbnail: false,
                style_kind: ButtonStyleKind::TextOnly,
                shape_mask: ButtonShapeMask::Rectangle,
                palette: palette([242, 242, 242], [29, 53, 87], [230, 57, 70]),
                font_family: "Arial",
                font_size: 18,
                font_weight: "bold",
                text_color: "#1D3557",
                title_color: "#FFFFFF",
                fill_color: "#F2F2F2",
                shadow_color: "#FFFFFF",
                margin_x: 0.12,
                margin_y: 0.06,
                gap: 0.012,
                header_ratio: 0.17,
                footer_ratio: 0.09,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::TopBand { color: "#1D3557" },
            },
            MenuTemplate {
                id: MenuTemplateId::Kids,
                name: "Infantil",
                description: "Celeste con confeti y botones redondos amarillos.",
                columns: 2,
                rows: 3,
                thumbnail: false,
                style_kind: ButtonStyleKind::PlayBadge,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([76, 201, 240], [29, 53, 87], [255, 255, 255]),
                font_family: "Trebuchet MS",
                font_size: 20,
                font_weight: "bold",
                text_color: "#1D3557",
                title_color: "#1D3557",
                fill_color: "#FFFFFF",
                shadow_color: "#FFFFFF",
                margin_x: 0.08,
                margin_y: 0.08,
                gap: 0.02,
                header_ratio: 0.15,
                footer_ratio: 0.09,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Confetti {
                    colors: ["#FFD166", "#EF476F", "#FFFFFF", "#06D6A0"],
                },
            },
            MenuTemplate {
                id: MenuTemplateId::Elegant,
                name: "Elegante",
                description: "Negro profundo, píldoras de cristal y un marco dorado.",
                columns: 1,
                rows: 4,
                thumbnail: false,
                style_kind: ButtonStyleKind::GlassPill,
                shape_mask: ButtonShapeMask::RoundedRect,
                palette: palette([11, 11, 15], [245, 222, 179], [212, 175, 55]),
                font_family: "Palatino Linotype",
                font_size: 22,
                font_weight: "normal",
                text_color: "#F5DEB3",
                title_color: "#F5DEB3",
                fill_color: "#1A1A22",
                shadow_color: "#000000",
                margin_x: 0.2,
                margin_y: 0.08,
                gap: 0.025,
                header_ratio: 0.15,
                footer_ratio: 0.1,
                dim_percent: 0,
                blur_px: 0,
                decor: Decor::Frame { color: "#D4AF37" },
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
    // Template typography is designed for the 720x480 DVD canvas; HD menus
    // (1920x1080/1280x720) scale it so text keeps the same visual size.
    let mut template = template;
    template.font_size = scaled_font_size(&template, root.canvas);
    if project.episodes.is_empty() {
        return Err(TemplateError::NoEpisodes);
    }
    let disc_title = disc_title(project, root);
    let bare = bare_background(project, root);
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
    // Links to the generated scene menus are re-created, never moved to Extras.
    let scene_menus = crate::chapters::generated_menu_ids(&project.menus, root_id);
    let scenes_link = crate::chapters::scenes_link(root, &scene_menus);
    // Same for the link to the generated audio/subtitle setup menu.
    let setup_menus = crate::setup::setup_menu_ids(&project.menus, root_id);
    let setup_link = crate::setup::setup_link(root, &setup_menus);
    let mut custom_buttons: Vec<MenuButton> = Vec::new();
    for menu in std::iter::once(root)
        .chain(old_pages.iter())
        .chain(old_extras_all.iter())
    {
        for button in &menu.buttons {
            if selected_ids.contains(&button.id)
                || is_generated_nav(button)
                || matches!(button.action, ButtonAction::GotoMenu { menu } if scene_menus.contains(&menu))
                || (setup_link.is_some()
                    && matches!(button.action, ButtonAction::GotoMenu { menu } if setup_menus.contains(&menu)))
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
        if bare {
            menu.items.extend(backdrop_items(menu.canvas, &template));
        }
        menu.items.extend(template_items(
            root,
            &template,
            &disc_title,
            index,
            page_count,
        ));
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
        if index == 0 {
            if let Some((name, target)) = &scenes_link {
                add_corner_link(&mut menu, &template, name, *target);
            }
            if let Some((name, target)) = &setup_link {
                add_corner_link(&mut menu, &template, name, *target);
            }
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
            rect: title_rect(root.canvas),
            font_family: template.font_family.to_string(),
            font_size: template.font_size as f32,
            color: template.title_color.to_string(),
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

fn root_background_asset(root: &Menu) -> kinodisc_domain::AssetId {
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
        && (button.name.starts_with('‹')
            || button.name.ends_with('›')
            || button.name.starts_with("Siguiente")
            || button.name == "Extras")
}

/// Template typography is designed for the 720x480 DVD canvas; HD menus
/// (1920x1080/1280x720) scale it so text keeps the same visual size.
pub(crate) fn scaled_font_size(template: &MenuTemplate, canvas: CanvasSpec) -> u16 {
    let canvas_scale = (f64::from(canvas.height) / 480.0).max(1.0);
    (f64::from(template.font_size) * canvas_scale).round() as u16
}

/// Text link centred in the footer (Back, Extras...).
pub(crate) fn add_center_link(
    menu: &mut Menu,
    template: &MenuTemplate,
    name: &str,
    target: MenuId,
) {
    let width = menu.canvas.width as f64;
    let height = menu.canvas.height as f64;
    let k = (height / 480.0).max(1.0);
    menu.buttons.push(nav_button(
        name,
        LogicalRect::new(
            (width - 140.0 * k) / 2.0,
            height - 54.0 * k,
            140.0 * k,
            34.0 * k,
        ),
        template,
        ButtonAction::GotoMenu { menu: target },
    ));
}

/// Text link in the bottom-left footer corner (free on the first page,
/// which has no Previous button).
/// A place for a link button that touches no other button or title: the
/// preferred spot when it is free, then the corners of the action-safe
/// area, then anywhere in it (from the bottom up), shrinking if needed.
fn free_spot(menu: &Menu, base_w: f64, base_h: f64, preferred: Option<LogicalRect>) -> LogicalRect {
    let height = menu.canvas.height as f64;
    let width = menu.canvas.width as f64;
    // Inside the action-safe area (5 % on each side), in the first corner
    // that does not touch another button or the title (QA 1.1.3: it landed
    // on a 2x2 card and the disc failed with E_BUTTON_OVERLAP).
    let taken: Vec<LogicalRect> = menu
        .buttons
        .iter()
        .map(|button| button.rect)
        .chain(menu.items.iter().filter_map(|item| match item {
            MenuItem::Text(text) => Some(text.rect),
            _ => None,
        }))
        .collect();
    let free = |rect: &LogicalRect| {
        taken.iter().all(|other| {
            rect.x + rect.w + 6.0 <= other.x
                || other.x + other.w + 6.0 <= rect.x
                || rect.y + rect.h + 6.0 <= other.y
                || other.y + other.h + 6.0 <= rect.y
        })
    };
    let even = |value: f64| (value / 2.0).round() * 2.0;
    let sizes =
        [1.0, 0.8, 0.65, 0.5, 0.4].map(|scale| (even(base_w * scale), even(base_h * scale)));
    let (safe_left, safe_top) = (even(width * 0.06), even(height * 0.06));
    // Corners first (where people look for it); if all four are taken (a
    // centred third card, QA 1.2.2), any free place in the safe area, from
    // the bottom up, before giving up.
    let corners = sizes.iter().flat_map(|&(w, h)| {
        let (right, bottom) = (even(width * 0.94 - w), even(height * 0.94 - h));
        [
            (safe_left, bottom),
            (right, bottom),
            (right, safe_top),
            (safe_left, safe_top),
        ]
        .map(|(x, y)| LogicalRect::new(x, y, w, h))
    });
    let anywhere = sizes.iter().flat_map(|&(w, h)| {
        let (right, bottom) = (even(width * 0.94 - w), even(height * 0.94 - h));
        // Every 4 px, plus the exact edges (a fixed step missed the gap
        // right next to the last card).
        let steps = |from: f64, to: f64| {
            let count = ((to - from) / 4.0).max(0.0) as usize;
            (0..=count)
                .map(move |step| from + step as f64 * 4.0)
                .chain(std::iter::once(to))
        };
        steps(safe_top, bottom)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .flat_map(move |y| steps(safe_left, right).map(move |x| LogicalRect::new(x, y, w, h)))
    });
    preferred
        .into_iter()
        .chain(corners)
        .chain(anywhere)
        .find(|rect| free(rect))
        .unwrap_or_else(|| {
            LogicalRect::new(
                (width * 0.06).round(),
                (height * 0.94 - base_h).round(),
                base_w,
                base_h,
            )
        })
}

pub(crate) fn add_corner_link(
    menu: &mut Menu,
    template: &MenuTemplate,
    name: &str,
    target: MenuId,
) {
    let k = (menu.canvas.height as f64 / 480.0).max(1.0);
    let spot = free_spot(menu, 200.0 * k, 40.0 * k, None);
    let mut link = nav_button(
        name,
        spot,
        template,
        ButtonAction::GotoMenu { menu: target },
    );
    // The way to the scene menus is a main entry, not a footnote: full-size
    // text in a framed button (it was small text-only in a corner).
    link.style_kind = ButtonStyleKind::ClassicFrame;
    link.font_size = link.font_size.max(template.font_size);
    menu.buttons.push(link);
}

pub(crate) fn page_shell(root: &Menu, id: MenuId, template: &MenuTemplate) -> Menu {
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
    asset: kinodisc_domain::AssetId,
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

/// The title a template writes on its pages: a title the user already typed
/// over a previous template's, otherwise the project name. The template's
/// own name ("Retro CRT") is never burned onto someone's disc.
fn disc_title(project: &Project, root: &Menu) -> String {
    let typed = root.items.iter().find_map(|item| match item {
        MenuItem::Text(text) if text.id == "tpl:title" => Some(text.text.as_str()),
        _ => None,
    });
    if let Some(typed) = typed {
        let base = typed.split(" · ").next().unwrap_or(typed).trim();
        let generated = base.is_empty() || templates().iter().any(|template| template.name == base);
        if !generated {
            return base.to_string();
        }
    }
    let name = project.name.trim();
    if name.is_empty() {
        "DVD".to_string()
    } else {
        name.to_string()
    }
}

/// The template's own background colour and decoration, under the title.
/// Skipped when the user chose a picture or video as the menu background.
fn backdrop_items(canvas: CanvasSpec, template: &MenuTemplate) -> Vec<MenuItem> {
    let width = f64::from(canvas.width);
    let height = f64::from(canvas.height);
    let rgb = template.palette.colors[0].rgb;
    let mut items = vec![shape(
        "tpl:bg",
        0.0,
        0.0,
        width,
        height,
        &format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]),
        None,
        0.0,
    )];
    let unit = (height / 480.0).max(1.0);
    let mut add = |x: f64, y: f64, w: f64, h: f64, fill: &str, stroke: Option<&str>, line: f64| {
        let id = format!("tpl:decor-{}", items.len());
        items.push(shape(&id, x, y, w, h, fill, stroke, line));
    };
    match template.decor {
        Decor::None => {}
        Decor::Frame { color } => {
            let inset = (width.min(height) * 0.035).round();
            add(
                inset,
                inset,
                width - inset * 2.0,
                height - inset * 2.0,
                "transparent",
                Some(color),
                2.0 * unit,
            );
        }
        Decor::Corners { color } => {
            let inset = (width.min(height) * 0.04).round();
            let arm = (width.min(height) * 0.09).round();
            let line = (2.0 * unit).round().max(2.0);
            for (cx, cy, sx, sy) in [
                (inset, inset, 1.0, 1.0),
                (width - inset, inset, -1.0, 1.0),
                (inset, height - inset, 1.0, -1.0),
                (width - inset, height - inset, -1.0, -1.0),
            ] {
                let x = if sx > 0.0 { cx } else { cx - arm };
                let y = if sy > 0.0 { cy } else { cy - line };
                add(x, y, arm, line, color, None, 0.0);
                let x = if sx > 0.0 { cx } else { cx - line };
                let y = if sy > 0.0 { cy } else { cy - arm };
                add(x, y, line, arm, color, None, 0.0);
            }
        }
        Decor::TopBand { color } => {
            let band = (height
                * (f64::from(template.header_ratio) + f64::from(template.margin_y) * 0.6))
                .round();
            add(0.0, 0.0, width, band, color, None, 0.0);
        }
        Decor::Filmstrip { band, hole } => {
            let band_h = (height * 0.085).round();
            add(0.0, 0.0, width, band_h, band, None, 0.0);
            add(0.0, height - band_h, width, band_h, band, None, 0.0);
            let hole_w = (band_h * 0.42).round();
            let hole_h = (band_h * 0.5).round();
            let step = hole_w * 2.2;
            let mut x = step * 0.4;
            while x + hole_w < width {
                add(
                    x,
                    ((band_h - hole_h) / 2.0).round(),
                    hole_w,
                    hole_h,
                    hole,
                    None,
                    0.0,
                );
                add(
                    x,
                    (height - band_h + (band_h - hole_h) / 2.0).round(),
                    hole_w,
                    hole_h,
                    hole,
                    None,
                    0.0,
                );
                x += step;
            }
        }
        Decor::Confetti { colors } => {
            // Fixed pseudo-random spots around the edges (same every time).
            const SPOTS: [(f64, f64, f64); 14] = [
                (0.03, 0.04, 1.0),
                (0.09, 0.11, 0.7),
                (0.15, 0.03, 0.8),
                (0.86, 0.05, 1.0),
                (0.92, 0.12, 0.7),
                (0.80, 0.02, 0.6),
                (0.03, 0.30, 0.6),
                (0.96, 0.33, 0.8),
                (0.02, 0.62, 0.8),
                (0.97, 0.66, 0.6),
                (0.05, 0.92, 1.0),
                (0.13, 0.96, 0.6),
                (0.90, 0.93, 0.9),
                (0.82, 0.97, 0.6),
            ];
            for (index, (fx, fy, scale)) in SPOTS.iter().enumerate() {
                let side = (12.0 * unit * scale).round().max(6.0);
                add(
                    (width * fx).round().min(width - side),
                    (height * fy).round().min(height - side),
                    side,
                    side,
                    colors[index % colors.len()],
                    None,
                    0.0,
                );
            }
        }
        Decor::Horizon { color } => {
            let ground = (height * 0.12).round();
            add(0.0, height - ground, width, ground, color, None, 0.0);
        }
        Decor::Stripes { color } => {
            let stripe = (width * 0.012).round().max(4.0);
            for index in 0..3 {
                let x = (width * 0.025).round() + f64::from(index) * stripe * 2.0;
                add(x, 0.0, stripe, height, color, None, 0.0);
            }
        }
    }
    items
}

#[allow(clippy::too_many_arguments)]
fn shape(
    id: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    fill: &str,
    stroke: Option<&str>,
    stroke_width: f64,
) -> MenuItem {
    MenuItem::Shape(ShapeItem {
        id: id.to_string(),
        rect: LogicalRect::new(x, y, w, h),
        fill: fill.to_string(),
        stroke: stroke.map(str::to_string),
        stroke_width: stroke_width as f32,
    })
}

/// True when the menu shows no user picture/video (only the empty starting
/// background), so the template may paint its own colour.
fn bare_background(project: &Project, menu: &Menu) -> bool {
    match &menu.background {
        MenuBackground::Static { asset } => project
            .asset(*asset)
            .is_none_or(|asset| asset.is_placeholder()),
        MenuBackground::VideoLoop { .. } => false,
    }
}

fn template_items(
    root: &Menu,
    template: &MenuTemplate,
    title: &str,
    page: usize,
    count: usize,
) -> Vec<MenuItem> {
    vec![MenuItem::Text(TextItem {
        id: "tpl:title".to_string(),
        text: if count > 1 {
            format!("{title} · {}/{}", page + 1, count)
        } else {
            title.to_string()
        },
        rect: title_rect(root.canvas),
        font_family: template.font_family.to_string(),
        font_size: template.font_size as f32,
        color: template.title_color.to_string(),
        align: TextAlign::Center,
    })]
}

/// Title band at the top of a template page, scaled to the canvas size.
pub(crate) fn title_rect(canvas: CanvasSpec) -> LogicalRect {
    let scale = (f64::from(canvas.height) / 480.0).max(1.0);
    let margin = (18.0 * scale).round();
    LogicalRect::new(
        margin,
        margin,
        (f64::from(canvas.width) - margin * 2.0).max(16.0),
        (36.0 * scale).round(),
    )
}

pub(crate) fn grid_rects(
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
    // Fewer episodes than cells (one-episode discs, the last page): centre
    // the used rows, and each row's cards, instead of leaving them in the
    // top-left corner with the rest of the screen empty.
    let used_rows = slots.div_ceil(columns.max(1)).min(rows.max(1));
    let offset_y = (rows.saturating_sub(used_rows)) as f64 * (cell_h + gap) / 2.0;
    (0..slots)
        .map(|index| {
            let column = index % columns;
            let row = index / columns;
            let in_row = (slots - row * columns).min(columns);
            let offset_x = (columns - in_row) as f64 * (cell_w + gap) / 2.0;
            LogicalRect::new(
                margin_x + offset_x + column as f64 * (cell_w + gap),
                margin_y + header + offset_y + row as f64 * (cell_h + gap),
                cell_w,
                cell_h,
            )
        })
        .collect()
}

pub(crate) fn add_navigation(
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
    let k = (height / 480.0).max(1.0);
    if let Some(target) = previous {
        menu.buttons.push(nav_button(
            "‹ Anterior",
            LogicalRect::new(18.0 * k, height - 54.0 * k, 180.0 * k, 34.0 * k),
            template,
            ButtonAction::GotoMenu { menu: target },
        ));
    }
    if let Some(target) = next {
        menu.buttons.push(nav_button(
            "Siguiente ›",
            LogicalRect::new(width - 198.0 * k, height - 54.0 * k, 180.0 * k, 34.0 * k),
            template,
            ButtonAction::GotoMenu { menu: target },
        ));
    }
    let _ = page;
}

fn add_extras_link(menu: &mut Menu, template: &MenuTemplate, extras: MenuId) {
    let width = menu.canvas.width as f64;
    let height = menu.canvas.height as f64;
    let k = (height / 480.0).max(1.0);
    // Bottom centre when free; it landed on a centred third card before
    // (QA 1.2.2), so otherwise the first free place.
    let centre = LogicalRect::new(
        ((width - 140.0 * k) / 4.0).round() * 2.0,
        ((height - 54.0 * k) / 2.0).round() * 2.0,
        (140.0 * k / 2.0).round() * 2.0,
        (34.0 * k / 2.0).round() * 2.0,
    );
    let spot = free_spot(menu, centre.w, centre.h, Some(centre));
    menu.buttons.push(nav_button(
        "Extras",
        spot,
        template,
        ButtonAction::GotoMenu { menu: extras },
    ));
}

pub(crate) fn nav_button(
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
        // `template.font_size` is already canvas-scaled; nav text is ~70 %.
        font_size: ((f64::from(template.font_size) * 0.72).round() as u16).max(12),
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

pub(crate) fn even_buttons(buttons: Vec<MenuButton>) -> Vec<MenuButton> {
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
