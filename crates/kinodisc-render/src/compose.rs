//! Menu artwork for renders that do not come from the desktop editor (CLI,
//! scripted batch). The editor composes each menu with Canvas 2D
//! (`apps/desktop/src/lib/editor/exportMenu.ts`); this module draws the same
//! layers in Rust so a disc rendered from the command line has its titles,
//! labels, thumbnails, pictures and shapes too:
//!
//! - `<menu_id>.png`: background, items and buttons (static menus).
//! - `labels-<menu_id>.png`: transparent overlay for menus over a video loop
//!   or with moving thumbnails (the video stays visible underneath).
//!
//! The geometry mirrors `buttonLayout.ts`; text uses the system fonts that
//! subtitles use (`kinodisc_subtitle::FontProvider`), so no extra dependency.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use image::{imageops, Rgba, RgbaImage};
use kinodisc_domain::{
    ButtonAction, ButtonShapeMask, ButtonStyleKind, LogicalRect, Menu, MenuBackground, MenuButton,
    MenuItem, MenuScaleMode, Project, TextAlign,
};
use kinodisc_subtitle::FontProvider;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Same base colour as the editor canvas (`menuBackgroundColor`).
const BASE: [u8; 4] = [11, 18, 32, 255];
const PADDING: f32 = 8.0;
const CARD_LABEL_HEIGHT: f32 = 28.0;

/// Writes the composition of every menu with buttons or items into `dir`.
/// Returns how many menus were composed. A picture that cannot be read is
/// skipped (like the editor), never fatal.
pub fn compose_project_menus(
    project: &Project,
    project_dir: &Path,
    dir: &Path,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut composer = Composer::new(project, project_dir, dir);
    let mut count = 0;
    for menu in &project.menus {
        if menu.buttons.is_empty() && menu.items.is_empty() {
            continue;
        }
        composer.compose(menu)?;
        count += 1;
    }
    Ok(count)
}

struct Layout {
    thumbnail: Option<Area>,
    label: Area,
    left: bool,
}

#[derive(Clone, Copy)]
struct Area {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// `buttonStyleLayout` from `buttonLayout.ts`.
fn button_layout(style: ButtonStyleKind, width: f32, height: f32, has_thumbnail: bool) -> Layout {
    let area = |x: f32, y: f32, w: f32, h: f32| Area {
        x,
        y,
        w: w.max(8.0),
        h: h.max(8.0),
    };
    match style {
        ButtonStyleKind::ThumbnailCard if !has_thumbnail => Layout {
            thumbnail: None,
            label: area(
                PADDING,
                PADDING,
                width - PADDING * 2.0,
                height - PADDING * 2.0,
            ),
            left: false,
        },
        ButtonStyleKind::ThumbnailCard => {
            let label_height = CARD_LABEL_HEIGHT.min((height * 0.22).max(16.0));
            let inner = (width - PADDING * 2.0).max(8.0);
            let max_thumb_h = (height - PADDING * 3.0 - label_height).max(8.0);
            let thumb_w = inner.min((max_thumb_h * 16.0 / 9.0).round()).max(8.0);
            let thumb_h = (thumb_w * 9.0 / 16.0).round().max(8.0);
            let label_y = (height - PADDING - 8.0).min(PADDING + thumb_h + 4.0);
            Layout {
                thumbnail: Some(Area {
                    x: ((width - thumb_w) / 2.0).round(),
                    y: PADDING,
                    w: thumb_w,
                    h: thumb_h,
                }),
                label: area(PADDING, label_y, inner, height - PADDING - label_y),
                left: false,
            }
        }
        ButtonStyleKind::TextOnly | ButtonStyleKind::Hotspot | ButtonStyleKind::GlassPill => {
            let glass = style == ButtonStyleKind::GlassPill;
            Layout {
                thumbnail: None,
                label: area(
                    if glass { 18.0 } else { 12.0 },
                    PADDING,
                    width - if glass { 36.0 } else { 24.0 },
                    height - PADDING * 2.0,
                ),
                left: style == ButtonStyleKind::Hotspot,
            }
        }
        ButtonStyleKind::PlayBadge => {
            let icon = (width * 0.18).clamp(32.0, 48.0);
            Layout {
                thumbnail: None,
                label: area(
                    icon,
                    PADDING,
                    width - icon - PADDING,
                    height - PADDING * 2.0,
                ),
                left: true,
            }
        }
        _ if !has_thumbnail => Layout {
            thumbnail: None,
            label: area(12.0, PADDING, width - 24.0, height - PADDING * 2.0),
            left: false,
        },
        _ => {
            let available = (height - PADDING * 2.0).max(8.0);
            let max_thumb_w = (width * 0.42).floor().max(16.0);
            let thumb_w = max_thumb_w.min((available * 4.0 / 3.0).round()).max(8.0);
            let label_x = PADDING + thumb_w + PADDING;
            Layout {
                thumbnail: Some(Area {
                    x: PADDING,
                    y: PADDING,
                    w: thumb_w,
                    h: available,
                }),
                label: area(label_x, PADDING, width - label_x - PADDING, available),
                left: true,
            }
        }
    }
}

struct Composer<'a> {
    project: &'a Project,
    project_dir: &'a Path,
    dir: &'a Path,
    fonts: FontProvider,
    font_cache: HashMap<(String, u16), Option<FontArc>>,
    fallbacks: Option<std::rc::Rc<Vec<FontArc>>>,
    frames: HashMap<(kinodisc_domain::AssetId, u64), Option<RgbaImage>>,
}

impl<'a> Composer<'a> {
    fn new(project: &'a Project, project_dir: &'a Path, dir: &'a Path) -> Self {
        Self {
            project,
            project_dir,
            dir,
            fonts: FontProvider::new(),
            font_cache: HashMap::new(),
            fallbacks: None,
            frames: HashMap::new(),
        }
    }

    fn compose(&mut self, menu: &Menu) -> std::io::Result<()> {
        let width = u32::from(menu.canvas.width);
        let height = u32::from(menu.canvas.height);
        let video_background = matches!(menu.background, MenuBackground::VideoLoop { .. });
        let motion = video_background
            || menu.motion_config.is_some()
            || menu
                .buttons
                .iter()
                .any(|button| button.motion_preview.is_some() && button.thumbnail.is_some());

        let mut background = RgbaImage::from_pixel(width, height, Rgba(BASE));
        if let MenuBackground::Static { asset } = &menu.background {
            if let Some(image) = self.asset_image(*asset, 0) {
                let image = if menu.blur_px > 0 {
                    imageops::blur(&image, menu.blur_px as f32 / 2.0)
                } else {
                    image
                };
                draw_scaled(&mut background, &image, menu.scale_mode);
            }
        }
        if menu.dim_percent > 0 {
            let alpha = f32::from(menu.dim_percent.min(80)) / 100.0;
            fill_rect(
                &mut background,
                0.0,
                0.0,
                width as f32,
                height as f32,
                [0, 0, 0, 255],
                alpha,
            );
        }

        if motion {
            // Background stays bare; items and labels go on the overlay.
            let mut labels = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
            self.draw_items(&mut labels, menu);
            for button in &menu.buttons {
                let animated = button.motion_preview.is_some()
                    && (button.thumbnail.is_some()
                        || matches!(
                            button.action,
                            ButtonAction::PlayEpisode { .. }
                                | ButtonAction::JumpTitleChapter { .. }
                        ));
                self.draw_button(&mut labels, button, animated);
            }
            labels
                .save(self.dir.join(format!("labels-{}.png", menu.id)))
                .map_err(io)?;
        } else {
            self.draw_items(&mut background, menu);
            for button in &menu.buttons {
                self.draw_button(&mut background, button, false);
            }
        }
        background
            .save(self.dir.join(format!("{}.png", menu.id)))
            .map_err(io)
    }

    fn draw_items(&mut self, canvas: &mut RgbaImage, menu: &Menu) {
        for item in &menu.items {
            match item {
                MenuItem::Text(text) => {
                    if text.text.trim().is_empty() || text.rect.w <= 0.0 || text.rect.h <= 0.0 {
                        continue;
                    }
                    let size = text.font_size.max(8.0);
                    let Some(font) = self.font(&text.font_family, 600) else {
                        continue;
                    };
                    let color = parse_hex(&text.color).unwrap_or([255, 255, 255, 255]);
                    let rect = area_of(&text.rect);
                    let lines = wrap(&font, size, &text.text, rect.w.max(8.0));
                    let line_height = size * 1.2;
                    let top =
                        rect.y + rect.h / 2.0 - (lines.len() as f32 - 1.0) * line_height / 2.0;
                    for (index, line) in lines.iter().enumerate() {
                        let (x, align) = match text.align {
                            TextAlign::Left => (rect.x, Align::Left),
                            TextAlign::Right => (rect.x + rect.w, Align::Right),
                            TextAlign::Center => (rect.x + rect.w / 2.0, Align::Center),
                        };
                        draw_text(
                            canvas,
                            &font,
                            size,
                            line,
                            x,
                            top + index as f32 * line_height,
                            align,
                            color,
                            Some(Shadow {
                                color: [0, 0, 0, 140],
                                blur: (size / 10.0).max(2.0),
                                dy: (size / 24.0).max(1.0),
                            }),
                            None,
                        );
                    }
                }
                MenuItem::Shape(shape) => {
                    let rect = area_of(&shape.rect);
                    if let Some(fill) = parse_hex(&shape.fill) {
                        fill_rect(canvas, rect.x, rect.y, rect.w, rect.h, fill, 1.0);
                    }
                    if let (Some(stroke), true) = (
                        shape.stroke.as_deref().and_then(parse_hex),
                        shape.stroke_width > 0.0,
                    ) {
                        stroke_rounded(canvas, rect, 0.0, shape.stroke_width, stroke, 1.0);
                    }
                }
                MenuItem::Image(image_item) => {
                    if let Some(image) = self.asset_image(image_item.asset, 0) {
                        let rect = area_of(&image_item.rect);
                        draw_into(
                            canvas,
                            &image,
                            rect,
                            false,
                            image_item.opacity.clamp(0.0, 1.0),
                        );
                    }
                }
                MenuItem::VideoThumbnail(item) => {
                    let ms = frame_ms(item.in_frame, item.fps);
                    if let Some(image) = self.asset_image(item.asset, ms) {
                        draw_into(canvas, &image, area_of(&item.rect), false, 1.0);
                    }
                }
            }
        }
    }

    fn draw_button(&mut self, canvas: &mut RgbaImage, button: &MenuButton, animated: bool) {
        let x = button.rect.x.round() as f32;
        let y = button.rect.y.round() as f32;
        let width = button.rect.w.round() as f32;
        let height = button.rect.h.round() as f32;
        if width < 2.0 || height < 2.0 {
            return;
        }
        let thumbnail = if animated {
            None
        } else {
            button.thumbnail.as_ref().and_then(|thumb| {
                self.asset_image(thumb.asset, frame_ms(thumb.in_frame, thumb.fps))
            })
        };
        let layout = button_layout(button.style_kind, width, height, thumbnail.is_some());
        let fill = parse_hex(&button.fill_color).unwrap_or([17, 24, 39, 255]);
        let text = parse_hex(&button.text_color).unwrap_or([255, 255, 255, 255]);
        let whole = Area {
            x,
            y,
            w: width,
            h: height,
        };
        let radius = (height / 4.0).min(14.0);
        let offset = |area: Area| Area {
            x: x + area.x,
            y: y + area.y,
            ..area
        };

        if animated {
            let shaped = button.shape_mask != ButtonShapeMask::Rectangle;
            if shaped
                || !matches!(
                    button.style_kind,
                    ButtonStyleKind::TextOnly | ButtonStyleKind::Hotspot
                )
            {
                stroke_rounded(canvas, whole, radius, 2.0, [226, 232, 240, 255], 0.78);
                if button.style_kind == ButtonStyleKind::GlassPill {
                    fill_rounded(canvas, whole, height / 2.0, [15, 23, 42, 255], 0.18);
                }
                if button.style_kind == ButtonStyleKind::PlayBadge {
                    play_badge(canvas, x, y, height, text);
                }
            }
        } else {
            match button.style_kind {
                ButtonStyleKind::Hotspot => return,
                ButtonStyleKind::TextOnly => {}
                ButtonStyleKind::ThumbnailCard => {
                    fill_rounded(canvas, whole, radius, fill, 0.82);
                    stroke_rounded(canvas, whole, radius, 2.0, [148, 163, 184, 255], 0.55);
                    match (&thumbnail, layout.thumbnail) {
                        (Some(image), Some(area)) => {
                            draw_into(canvas, image, offset(area), true, 1.0)
                        }
                        (None, Some(area)) => {
                            let area = offset(area);
                            fill_rect(
                                canvas,
                                area.x,
                                area.y,
                                area.w,
                                area.h,
                                [30, 41, 59, 255],
                                0.9,
                            );
                        }
                        _ => {}
                    }
                }
                ButtonStyleKind::GlassPill => {
                    fill_rounded(canvas, whole, height / 2.0, fill, 0.48);
                    stroke_rounded(canvas, whole, height / 2.0, 1.5, [255, 255, 255, 255], 0.28);
                }
                ButtonStyleKind::PlayBadge => {
                    fill_rounded(canvas, whole, height / 2.0, fill, 0.62);
                    stroke_rounded(canvas, whole, height / 2.0, 1.5, [255, 255, 255, 255], 0.25);
                    play_badge(canvas, x, y, height, text);
                }
                _ => {
                    fill_rounded(canvas, whole, radius, fill, 0.55);
                    stroke_rounded(canvas, whole, radius, 2.0, [226, 232, 240, 255], 0.65);
                    if let (Some(image), Some(area)) = (&thumbnail, layout.thumbnail) {
                        draw_into(canvas, image, offset(area), true, 1.0);
                    }
                }
            }
        }

        // Label (drawButtonLabel).
        let label = offset(layout.label);
        if button.style_kind == ButtonStyleKind::ThumbnailCard && !animated {
            let plate = Area {
                x: label.x - 4.0,
                y: label.y - 2.0,
                w: label.w + 8.0,
                h: label.h + 4.0,
            };
            fill_rounded(canvas, plate, 6.0, [2, 6, 23, 255], 0.72);
        }
        let weight = if button.font_weight == "bold" {
            700
        } else {
            400
        };
        let Some(font) = self.font(&button.font_family, weight) else {
            return;
        };
        // Thumbnail cards: as large as asked, up to what fits the label strip
        // (a fixed 18 px cap ignored the Size control and was tiny on HD).
        let size = if button.style_kind == ButtonStyleKind::ThumbnailCard {
            f32::from(button.font_size).min((label.h * 0.8).round().max(12.0))
        } else {
            f32::from(button.font_size)
        };
        let shadow = parse_hex(&button.shadow_color).map(|mut color| {
            color[3] = 230;
            Shadow {
                color,
                blur: 4.0,
                dy: 2.0,
            }
        });
        let (tx, align) = if layout.left {
            (label.x, Align::Left)
        } else {
            (label.x + label.w / 2.0, Align::Center)
        };
        draw_text(
            canvas,
            &font,
            size,
            &button.name,
            tx,
            label.y + label.h / 2.0,
            align,
            text,
            shadow,
            Some((label.w - 4.0).max(20.0)),
        );
    }

    fn font(&mut self, family: &str, weight: u16) -> Option<Glyphs> {
        let key = (family.to_string(), weight);
        let primary = match self.font_cache.get(&key) {
            Some(font) => font.clone(),
            None => {
                let font = self.fonts.resolve_family(family, weight);
                self.font_cache.insert(key, font.clone());
                font
            }
        }?;
        let fonts = &self.fonts;
        let fallbacks = self
            .fallbacks
            .get_or_insert_with(|| {
                let names = [
                    "Segoe UI Emoji",
                    "Segoe UI Symbol",
                    "Noto Emoji",
                    "Noto Color Emoji",
                    "Noto Sans Symbols",
                    "DejaVu Sans",
                ];
                std::rc::Rc::new(
                    names
                        .iter()
                        .filter_map(|name| fonts.resolve_exact(name, 400))
                        .collect(),
                )
            })
            .clone();
        Some(Glyphs { primary, fallbacks })
    }

    /// An image asset, or a frame of a video asset (`at_ms` 0 = the poster
    /// frame the editor shows: 10 % of the duration, at most 10 s).
    fn asset_image(&mut self, id: kinodisc_domain::AssetId, at_ms: u64) -> Option<RgbaImage> {
        let key = (id, at_ms);
        if let Some(image) = self.frames.get(&key) {
            return image.clone();
        }
        let image = self.load_asset(id, at_ms);
        self.frames.insert(key, image.clone());
        image
    }

    fn load_asset(&self, id: kinodisc_domain::AssetId, at_ms: u64) -> Option<RgbaImage> {
        let asset = self.project.asset(id)?;
        if asset.is_placeholder() {
            return None;
        }
        let path = crate::asset_path(self.project_dir, asset)?;
        if !path.is_file() {
            return None;
        }
        let is_video = asset.kind == kinodisc_domain::MediaKind::Video;
        if !is_video
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        {
            if let Ok(image) = image::open(&path) {
                return Some(image.to_rgba8());
            }
        }
        // JPEG/WebP pictures and video frames go through FFmpeg (the image
        // crate is built with PNG only).
        let time_ms = if !is_video {
            0
        } else if at_ms > 0 {
            at_ms
        } else {
            let duration = asset.probe.duration_ms.unwrap_or(0);
            if duration > 0 {
                (duration / 10).min(10_000)
            } else {
                1_000
            }
        };
        let out: PathBuf = self.dir.join(format!("frame-{id}-{time_ms}.png"));
        let mut command =
            kinodisc_toolchain::background_command(kinodisc_toolchain::resolve_binary("ffmpeg"));
        command.args(["-hide_banner", "-loglevel", "error", "-y"]);
        if is_video {
            command.args(["-ss", &format!("{:.3}", time_ms as f64 / 1000.0)]);
        }
        command
            .arg("-protocol_whitelist")
            .arg(crate::MEDIA_PROTOCOL_WHITELIST)
            .arg("-i")
            .arg(&path)
            .args(["-frames:v", "1", "-update", "1"])
            .arg(&out);
        let ok = command.output().is_ok_and(|output| output.status.success());
        let image = ok
            .then(|| image::open(&out).ok())
            .flatten()
            .map(|image| image.to_rgba8());
        let _ = std::fs::remove_file(&out);
        image
    }
}

fn io(error: image::ImageError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

fn frame_ms(frame: u64, fps: kinodisc_domain::FrameRate) -> u64 {
    if frame == 0 || fps.num == 0 {
        0
    } else {
        (frame * u64::from(fps.den) * 1000) / u64::from(fps.num)
    }
}

fn area_of(rect: &LogicalRect) -> Area {
    Area {
        x: rect.x as f32,
        y: rect.y as f32,
        w: rect.w as f32,
        h: rect.h as f32,
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub(crate) fn parse_hex(value: &str) -> Option<[u8; 4]> {
    let hex = value.trim().strip_prefix('#')?;
    let byte = |index: usize| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok();
    match hex.len() {
        3 => {
            let digit = |index: usize| {
                u8::from_str_radix(hex.get(index..index + 1)?, 16)
                    .ok()
                    .map(|v| v * 17)
            };
            Some([digit(0)?, digit(1)?, digit(2)?, 255])
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// Source-over blend of `color` with `coverage` (0..1) onto a pixel.
fn blend(pixel: &mut Rgba<u8>, color: [u8; 4], coverage: f32) {
    let alpha = (f32::from(color[3]) / 255.0) * coverage.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let dst_alpha = f32::from(pixel[3]) / 255.0;
    let out_alpha = alpha + dst_alpha * (1.0 - alpha);
    if out_alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let src = f32::from(color[channel]);
        let dst = f32::from(pixel[channel]);
        let value = (src * alpha + dst * dst_alpha * (1.0 - alpha)) / out_alpha;
        pixel[channel] = value.round().clamp(0.0, 255.0) as u8;
    }
    pixel[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

fn fill_rect(canvas: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, color: [u8; 4], alpha: f32) {
    fill_rounded(canvas, Area { x, y, w, h }, 0.0, color, alpha);
}

/// Signed distance to a rounded rectangle (negative inside).
fn rounded_distance(px: f32, py: f32, area: Area, radius: f32) -> f32 {
    let radius = radius.min(area.w / 2.0).min(area.h / 2.0).max(0.0);
    let cx = area.x + area.w / 2.0;
    let cy = area.y + area.h / 2.0;
    let qx = (px - cx).abs() - (area.w / 2.0 - radius);
    let qy = (py - cy).abs() - (area.h / 2.0 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

fn bounds(canvas: &RgbaImage, area: Area, pad: f32) -> (u32, u32, u32, u32) {
    let x0 = (area.x - pad).floor().max(0.0) as u32;
    let y0 = (area.y - pad).floor().max(0.0) as u32;
    let x1 = ((area.x + area.w + pad).ceil().max(0.0) as u32).min(canvas.width());
    let y1 = ((area.y + area.h + pad).ceil().max(0.0) as u32).min(canvas.height());
    (x0, y0, x1, y1)
}

fn fill_rounded(canvas: &mut RgbaImage, area: Area, radius: f32, color: [u8; 4], alpha: f32) {
    let (x0, y0, x1, y1) = bounds(canvas, area, 1.0);
    for py in y0..y1 {
        for px in x0..x1 {
            let distance = rounded_distance(px as f32 + 0.5, py as f32 + 0.5, area, radius);
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(canvas.get_pixel_mut(px, py), color, coverage * alpha);
            }
        }
    }
}

fn stroke_rounded(
    canvas: &mut RgbaImage,
    area: Area,
    radius: f32,
    line: f32,
    color: [u8; 4],
    alpha: f32,
) {
    let half = line / 2.0;
    let (x0, y0, x1, y1) = bounds(canvas, area, half + 1.0);
    for py in y0..y1 {
        for px in x0..x1 {
            let distance = rounded_distance(px as f32 + 0.5, py as f32 + 0.5, area, radius).abs();
            let coverage = (half + 0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend(canvas.get_pixel_mut(px, py), color, coverage * alpha);
            }
        }
    }
}

/// Round badge with a play triangle (playBadge style).
fn play_badge(canvas: &mut RgbaImage, x: f32, y: f32, height: f32, color: [u8; 4]) {
    let radius = (height * 0.3).min(18.0);
    let cx = x + 10.0 + radius;
    let cy = y + height / 2.0;
    let circle = Area {
        x: cx - radius,
        y: cy - radius,
        w: radius * 2.0,
        h: radius * 2.0,
    };
    fill_rounded(canvas, circle, radius, color, 1.0);
    let a = (cx - radius * 0.28, cy - radius * 0.45);
    let b = (cx + radius * 0.5, cy);
    let c = (cx - radius * 0.28, cy + radius * 0.45);
    let (x0, y0, x1, y1) = bounds(canvas, circle, 1.0);
    for py in y0..y1 {
        for px in x0..x1 {
            // 4x4 supersampling is plenty for a small triangle.
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = (
                        px as f32 + (sx as f32 + 0.5) / 4.0,
                        py as f32 + (sy as f32 + 0.5) / 4.0,
                    );
                    let edge = |u: (f32, f32), v: (f32, f32)| {
                        (v.0 - u.0) * (p.1 - u.1) - (v.1 - u.1) * (p.0 - u.0)
                    };
                    let (e0, e1, e2) = (edge(a, b), edge(b, c), edge(c, a));
                    if (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0)
                        || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)
                    {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                blend(
                    canvas.get_pixel_mut(px, py),
                    [11, 18, 32, 255],
                    hits as f32 / 16.0,
                );
            }
        }
    }
}

/// Background picture with the menu's scale mode (`drawImageWithScale`).
fn draw_scaled(canvas: &mut RgbaImage, image: &RgbaImage, mode: MenuScaleMode) {
    let (width, height) = (canvas.width() as f32, canvas.height() as f32);
    let area = Area {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
    };
    match mode {
        MenuScaleMode::Stretch => draw_resized(canvas, image, area, 1.0),
        MenuScaleMode::Contain => {
            let scale = (width / image.width() as f32).min(height / image.height() as f32);
            let (w, h) = (image.width() as f32 * scale, image.height() as f32 * scale);
            draw_resized(
                canvas,
                image,
                Area {
                    x: (width - w) / 2.0,
                    y: (height - h) / 2.0,
                    w,
                    h,
                },
                1.0,
            );
        }
        _ => draw_into(canvas, image, area, true, 1.0),
    }
}

/// Draws `image` into `area`: `cover` crops to fill (thumbnails), otherwise
/// it is stretched to the area (items).
fn draw_into(canvas: &mut RgbaImage, image: &RgbaImage, area: Area, cover: bool, alpha: f32) {
    if !cover {
        draw_resized(canvas, image, area, alpha);
        return;
    }
    let scale = (area.w / image.width() as f32).max(area.h / image.height() as f32);
    let src_w = (area.w / scale).round().clamp(1.0, image.width() as f32) as u32;
    let src_h = (area.h / scale).round().clamp(1.0, image.height() as f32) as u32;
    let src_x = (image.width() - src_w) / 2;
    let src_y = (image.height() - src_h) / 2;
    let cropped = imageops::crop_imm(image, src_x, src_y, src_w, src_h).to_image();
    draw_resized(canvas, &cropped, area, alpha);
}

fn draw_resized(canvas: &mut RgbaImage, image: &RgbaImage, area: Area, alpha: f32) {
    let w = area.w.round().max(1.0) as u32;
    let h = area.h.round().max(1.0) as u32;
    let resized = imageops::resize(image, w, h, imageops::FilterType::Triangle);
    let (ox, oy) = (area.x.round() as i64, area.y.round() as i64);
    for (sx, sy, pixel) in resized.enumerate_pixels() {
        let (px, py) = (ox + i64::from(sx), oy + i64::from(sy));
        if px < 0 || py < 0 || px >= i64::from(canvas.width()) || py >= i64::from(canvas.height()) {
            continue;
        }
        blend(canvas.get_pixel_mut(px as u32, py as u32), pixel.0, alpha);
    }
}

/// A font plus per-character fallbacks (emoji, symbols), like the browser.
#[derive(Clone)]
struct Glyphs {
    primary: FontArc,
    fallbacks: std::rc::Rc<Vec<FontArc>>,
}

impl Glyphs {
    fn pick(&self, ch: char) -> &FontArc {
        if ch.is_whitespace() || self.primary.glyph_id(ch).0 != 0 {
            return &self.primary;
        }
        self.fallbacks
            .iter()
            .find(|font| font.glyph_id(ch).0 != 0)
            .unwrap_or(&self.primary)
    }
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy)]
struct Shadow {
    color: [u8; 4],
    blur: f32,
    dy: f32,
}

fn line_width(font: &Glyphs, size: f32, text: &str) -> f32 {
    text.chars()
        .map(|ch| {
            let face = font.pick(ch);
            face.as_scaled(PxScale::from(size))
                .h_advance(face.glyph_id(ch))
        })
        .sum()
}

fn wrap(font: &Glyphs, size: f32, text: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut words = paragraph.split_whitespace();
        let Some(first) = words.next() else {
            lines.push(String::new());
            continue;
        };
        let mut line = first.to_string();
        for word in words {
            let next = format!("{line} {word}");
            if line_width(font, size, &next) <= width {
                line = next;
            } else {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            }
        }
        lines.push(line);
    }
    lines
}

/// Fits one line in `max_width` like the editor export (`fitLabel` in
/// exportMenu.ts): a slightly smaller size (down to 75 %), then an ellipsis.
/// Squeezing the glyphs made long names unreadable.
fn fit_line(font: &Glyphs, size: f32, text: &str, max_width: Option<f32>) -> (f32, String) {
    let Some(max) = max_width.filter(|max| *max > 0.0) else {
        return (size, text.to_string());
    };
    if line_width(font, size, text) <= max {
        return (size, text.to_string());
    }
    let mut scale = 0.95;
    while scale >= 0.749 {
        if line_width(font, size * scale, text) <= max {
            return (size * scale, text.to_string());
        }
        scale -= 0.05;
    }
    let small = size * 0.75;
    let mut chars: Vec<char> = text.chars().collect();
    loop {
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if chars.len() <= 1 || line_width(font, small, &candidate) <= max {
            return (small, candidate);
        }
        chars.pop();
    }
}

/// One line of text centred vertically on `center_y`. With `max_width` the
/// text is fitted by `fit_line`.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    canvas: &mut RgbaImage,
    font: &Glyphs,
    size: f32,
    text: &str,
    x: f32,
    center_y: f32,
    align: Align,
    color: [u8; 4],
    shadow: Option<Shadow>,
    max_width: Option<f32>,
) {
    let (size, fitted) = fit_line(font, size, text, max_width);
    let text = fitted.as_str();
    let scale = PxScale { x: size, y: size };
    let scaled = font.primary.as_scaled(scale);
    let width = line_width(font, size, text);
    let start = match align {
        Align::Left => x,
        Align::Center => x - width / 2.0,
        Align::Right => x - width,
    };
    let baseline = center_y + (scaled.ascent() + scaled.descent()) / 2.0;
    // Coverage mask of the line, then shadow (blurred, offset) and fill.
    let pad = shadow.map_or(2.0, |shadow| shadow.blur * 2.0 + shadow.dy + 2.0);
    let mask_x = (start - pad).floor();
    let mask_y = (baseline - scaled.ascent() - pad).floor();
    let mask_w = (width + pad * 2.0).ceil().max(1.0) as u32;
    let mask_h = (scaled.ascent() - scaled.descent() + pad * 2.0)
        .ceil()
        .max(1.0) as u32;
    let mut mask = image::GrayImage::new(mask_w, mask_h);
    let mut pen = start;
    for ch in text.chars() {
        let face = font.pick(ch);
        let id = face.glyph_id(ch);
        let glyph =
            id.with_scale_and_position(scale, ab_glyph::point(pen - mask_x, baseline - mask_y));
        if let Some(outline) = face.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + i64::from(gx);
                let py = bounds.min.y as i64 + i64::from(gy);
                if px >= 0 && py >= 0 && px < i64::from(mask_w) && py < i64::from(mask_h) {
                    let pixel = mask.get_pixel_mut(px as u32, py as u32);
                    pixel[0] = pixel[0].max((coverage * 255.0).round().clamp(0.0, 255.0) as u8);
                }
            });
        }
        pen += face.as_scaled(scale).h_advance(id);
    }
    if let Some(shadow) = shadow {
        let blurred = if shadow.blur > 0.0 {
            imageops::blur(&mask, shadow.blur / 2.0)
        } else {
            mask.clone()
        };
        paint_mask(canvas, &blurred, mask_x, mask_y + shadow.dy, shadow.color);
    }
    paint_mask(canvas, &mask, mask_x, mask_y, color);
}

fn paint_mask(canvas: &mut RgbaImage, mask: &image::GrayImage, x: f32, y: f32, color: [u8; 4]) {
    let (ox, oy) = (x.round() as i64, y.round() as i64);
    for (mx, my, value) in mask.enumerate_pixels() {
        if value[0] == 0 {
            continue;
        }
        let (px, py) = (ox + i64::from(mx), oy + i64::from(my));
        if px < 0 || py < 0 || px >= i64::from(canvas.width()) || py >= i64::from(canvas.height()) {
            continue;
        }
        blend(
            canvas.get_pixel_mut(px as u32, py as u32),
            color,
            f32::from(value[0]) / 255.0,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_labels_shrink_then_get_an_ellipsis() {
        // Needs any system font; machines without fonts skip the check.
        let fonts = FontProvider::new();
        let Some(primary) = fonts
            .resolve_family("Arial", 400)
            .or_else(|| fonts.resolve_family("DejaVu Sans", 400))
        else {
            return;
        };
        let glyphs = Glyphs {
            primary,
            fallbacks: std::rc::Rc::new(Vec::new()),
        };
        let (size, text) = fit_line(&glyphs, 20.0, "Short", Some(500.0));
        assert_eq!((size, text.as_str()), (20.0, "Short"));
        // Slightly too long: a smaller size, same text.
        let natural = line_width(&glyphs, 20.0, "Cumpleaños de Sofía");
        let (size, text) = fit_line(&glyphs, 20.0, "Cumpleaños de Sofía", Some(natural * 0.9));
        assert!((15.0..20.0).contains(&size));
        assert_eq!(text, "Cumpleaños de Sofía");
        // Far too long: the smallest size and an ellipsis, never squeezed
        // (QA 1.1.3: long names were unreadable on the disc).
        let long = "Cumpleaños de Sofía — fiesta en el jardín con toda la familia 2026";
        let (size, text) = fit_line(&glyphs, 20.0, long, Some(120.0));
        assert!((size - 15.0).abs() < 0.01);
        assert!(text.ends_with('…'));
        assert!(line_width(&glyphs, size, &text) <= 120.0);
    }

    #[test]
    fn hex_colours_parse() {
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_hex("#0b1220"), Some([11, 18, 32, 255]));
        assert_eq!(parse_hex("#00000080"), Some([0, 0, 0, 128]));
        assert_eq!(parse_hex("red"), None);
    }

    #[test]
    fn card_layout_matches_the_editor() {
        // buttonStyleLayout("thumbnailCard", 302, 200, true) in the editor:
        // label 28 px, thumbnail max 148 px high -> 263x148, centred at 20.
        let layout = button_layout(ButtonStyleKind::ThumbnailCard, 302.0, 200.0, true);
        let thumb = layout.thumbnail.expect("thumbnail");
        assert_eq!(
            (thumb.x, thumb.y, thumb.w, thumb.h),
            (20.0, 8.0, 263.0, 148.0)
        );
        assert_eq!(layout.label.y, 8.0 + 148.0 + 4.0);
    }

    #[test]
    fn rounded_fill_covers_inside_and_leaves_outside() {
        let mut canvas = RgbaImage::from_pixel(40, 40, Rgba([0, 0, 0, 255]));
        fill_rounded(
            &mut canvas,
            Area {
                x: 5.0,
                y: 5.0,
                w: 30.0,
                h: 30.0,
            },
            8.0,
            [255, 0, 0, 255],
            1.0,
        );
        assert_eq!(canvas.get_pixel(20, 20).0, [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(1, 1).0, [0, 0, 0, 255]);
        // The rounded corner leaves the very corner pixel untouched.
        assert_eq!(canvas.get_pixel(5, 5).0, [0, 0, 0, 255]);
    }
}
