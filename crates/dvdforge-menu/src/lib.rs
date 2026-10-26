//! Menu compiler façade: raster layer generation, subpicture validation and
//! spumux XML construction.

pub use dvdforge_domain::geometry::{
    has_gap, normalize_rect, validate_layout, CanvasSpec, LogicalRect, PixelRect,
};
pub use dvdforge_domain::menu::{
    ButtonAction, ButtonShapeMask, ButtonStyleKind, Menu, MenuBackground, MenuButton,
    MenuPlacement, MenuRole, MenuScaleMode, NavOverride, SubpictureColor, SubpicturePalette,
    SubpictureSpec,
};
pub use dvdforge_domain::navigation::{
    resolve_navigation, EdgeSource, NavDirection, NavEdge, NavNode, NavPolicy, NavigationGraph,
};
use dvdforge_domain::{AspectRatio, ButtonId, FrameRate, MenuId, TvStandard};
use image::{imageops::FilterType, GrayImage, Luma, Rgba, RgbaImage};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use thiserror::Error;
use tokio::process::Command;

pub const CRATE_NAME: &str = "dvdforge-menu";
pub const MAX_BUTTONS_PER_MENU: usize = 36;
pub const RECOMMENDED_BUTTONS_16_9: usize = 18;
pub const MAX_SUBPICTURE_COLORS: usize = 4;

#[derive(Debug, Error)]
pub enum MenuError {
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
    #[error("PNG inválido: {0}")]
    Image(#[from] image::ImageError),
    #[error("la capa {path} tiene {colors} colores; el máximo es 4")]
    TooManyColors { path: String, colors: usize },
    #[error(
        "la capa {path} mide {width}x{height}; se esperaba {expected_width}x{expected_height}"
    )]
    WrongDimensions {
        path: String,
        width: u32,
        height: u32,
        expected_width: u32,
        expected_height: u32,
    },
    #[error("el menú no tiene botones")]
    NoButtons,
    #[error("botón inválido: {0}")]
    InvalidButton(String),
    #[error("spumux falló con código {code}: {stderr}")]
    SpumuxFailed { code: i32, stderr: String },
    #[error("subtítulo inválido: {0}")]
    InvalidSubtitle(String),
}

#[derive(Debug, Clone)]
pub struct CompiledButton {
    pub id: ButtonId,
    pub name: String,
    pub rect: PixelRect,
    pub style_kind: ButtonStyleKind,
    pub shape_mask: ButtonShapeMask,
}

#[derive(Debug, Clone)]
pub struct MenuCompileRequest {
    pub menu_id: MenuId,
    pub canvas: CanvasSpec,
    pub background_png: PathBuf,
    pub highlight_png: PathBuf,
    pub select_png: PathBuf,
    pub buttons: Vec<CompiledButton>,
    pub force: bool,
}

#[derive(Debug, Clone)]
pub struct MenuLayers {
    pub background_png: PathBuf,
    pub highlight_png: PathBuf,
    pub select_png: PathBuf,
}

#[derive(Debug, Clone)]
pub struct MenuLayerRequest {
    pub canvas: CanvasSpec,
    pub background_source: PathBuf,
    pub buttons: Vec<CompiledButton>,
    pub palette: SubpicturePalette,
    pub scale_mode: MenuScaleMode,
    pub dim_percent: u8,
    pub blur_px: u8,
    pub output_dir: PathBuf,
}

/// Generates the background and the two 4-color subpicture masks for a menu.
pub fn generate_menu_layers(request: &MenuLayerRequest) -> Result<MenuLayers, MenuError> {
    if request.buttons.is_empty() {
        return Err(MenuError::NoButtons);
    }
    std::fs::create_dir_all(&request.output_dir)?;

    let background_png = request.output_dir.join("background.png");
    let highlight_png = request.output_dir.join("highlight.png");
    let select_png = request.output_dir.join("select.png");

    let background = fit_background(
        &request.background_source,
        request.canvas.width as u32,
        request.canvas.height as u32,
        request.scale_mode,
        request.dim_percent,
        request.blur_px,
    )?;
    background.save(&background_png)?;

    let width = request.canvas.width as u32;
    let height = request.canvas.height as u32;
    let transparent = Rgba([0, 0, 0, 0]);
    let highlight_color = palette_png_color(request.palette.colors[1], [255, 255, 255, 255]);
    let select_color = palette_png_color(request.palette.colors[2], [255, 220, 0, 255]);

    let mut highlight = RgbaImage::from_pixel(width, height, transparent);
    let mut select = RgbaImage::from_pixel(width, height, transparent);

    for button in &request.buttons {
        if !button.rect.is_valid(request.canvas) {
            return Err(MenuError::InvalidButton(format!(
                "{} tiene un rectángulo inválido o impar",
                button.name
            )));
        }
        draw_button_masks(
            &mut highlight,
            &mut select,
            button.rect,
            button.style_kind,
            button.shape_mask,
            highlight_color,
            select_color,
        );
    }

    highlight.save(&highlight_png)?;
    select.save(&select_png)?;

    validate_subpicture_png(&highlight_png, request.canvas)?;
    validate_subpicture_png(&select_png, request.canvas)?;

    Ok(MenuLayers {
        background_png,
        highlight_png,
        select_png,
    })
}

/// Builds a spumux XML document from a compile request and its resolved
/// navigation graph. Both masks are validated before the XML is emitted.
pub fn build_spumux_xml(
    request: &MenuCompileRequest,
    navigation: &NavigationGraph,
) -> Result<String, MenuError> {
    if request.buttons.is_empty() {
        return Err(MenuError::NoButtons);
    }
    if request.buttons.len() > MAX_BUTTONS_PER_MENU {
        return Err(MenuError::InvalidButton(format!(
            "el menú tiene {} botones; el máximo es {}",
            request.buttons.len(),
            MAX_BUTTONS_PER_MENU
        )));
    }

    let highlight_colors = validate_subpicture_png(&request.highlight_png, request.canvas)?;
    let select_colors = validate_subpicture_png(&request.select_png, request.canvas)?;

    let mut names = HashMap::new();
    let mut seen = HashSet::new();
    for (index, button) in request.buttons.iter().enumerate() {
        if !seen.insert(button.id) {
            return Err(MenuError::InvalidButton(
                "ID de botón duplicado".to_string(),
            ));
        }
        if !button.rect.is_valid(request.canvas) {
            return Err(MenuError::InvalidButton(format!(
                "{} tiene coordenadas impares o fuera del lienzo",
                button.name
            )));
        }
        names.insert(button.id, format!("b{}", index + 1));
    }

    let highlight = xml_escape(&request.highlight_png.display().to_string());
    let select = xml_escape(&request.select_png.display().to_string());
    let force = if request.force { " force=\"yes\"" } else { "" };

    let mut xml = String::new();
    xml.push_str("<subpictures>\n");
    xml.push_str("  <stream>\n");
    xml.push_str(&format!(
        "    <spu start=\"00:00:00.000\"{force} highlight=\"{highlight}\" select=\"{select}\">\n"
    ));
    xml.push_str(&format!(
        "      <!-- colors: highlight={highlight_colors}, select={select_colors} -->\n"
    ));

    for button in &request.buttons {
        let name = names
            .get(&button.id)
            .expect("button name was inserted above");
        let mut directions = Vec::with_capacity(4);
        for direction in NavDirection::ALL {
            let target = navigation.target(button.id, direction).unwrap_or(button.id);
            let target_name = names
                .get(&target)
                .map(String::as_str)
                .unwrap_or(name.as_str());
            directions.push(format!("{}=\"{}\"", direction.label(), target_name));
        }
        xml.push_str(&format!(
            "      <button name=\"{}\" x0=\"{}\" y0=\"{}\" x1=\"{}\" y1=\"{}\" {}/>\n",
            name,
            button.rect.x0,
            button.rect.y0,
            button.rect.x1,
            button.rect.y1,
            directions.join(" ")
        ));
    }

    xml.push_str("    </spu>\n");
    xml.push_str("  </stream>\n");
    xml.push_str("</subpictures>\n");
    Ok(xml)
}

pub fn write_spumux_xml(
    request: &MenuCompileRequest,
    navigation: &NavigationGraph,
    path: &Path,
) -> Result<(), MenuError> {
    let xml = build_spumux_xml(request, navigation)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, xml)?;
    Ok(())
}

/// Async wrapper around the spumux binary.
#[derive(Debug, Clone)]
pub struct Spumux {
    pub binary: PathBuf,
}

impl Default for Spumux {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("spumux"),
        }
    }
}

impl Spumux {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// Runs `spumux -m dvd <xml> < menu_base.mpg > menu_sub.mpg`.
    pub async fn mux(
        &self,
        xml: &Path,
        input_mpeg: &Path,
        output_mpeg: &Path,
    ) -> Result<(), MenuError> {
        self.mux_stream(xml, input_mpeg, output_mpeg, 0).await
    }

    /// Añade un stream de subtítulos con un id concreto. Repetir la llamada con
    /// ids crecientes permite multiplexar varias pistas en el mismo MPEG.
    pub async fn mux_stream(
        &self,
        xml: &Path,
        input_mpeg: &Path,
        output_mpeg: &Path,
        stream_id: u8,
    ) -> Result<(), MenuError> {
        if let Some(parent) = output_mpeg.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let stdin = std::fs::File::open(input_mpeg)?;
        let stdout = std::fs::File::create(output_mpeg)?;
        let status = Command::new(&self.binary)
            .arg("-m")
            .arg("dvd")
            .arg("-s")
            .arg(stream_id.to_string())
            .arg(xml)
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .status()
            .await?;

        if status.success() {
            Ok(())
        } else {
            Err(MenuError::SpumuxFailed {
                code: status.code().unwrap_or(-1),
                stderr: "consulta el log de spumux en stderr".to_string(),
            })
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextSubtitleSpec<'a> {
    pub srt: &'a Path,
    pub fps: FrameRate,
    pub standard: TvStandard,
    pub aspect: AspectRatio,
    pub forced: bool,
}

/// Genera el XML que spumux usa para renderizar un `.srt` como subpicture.
pub fn build_textsub_xml(spec: &TextSubtitleSpec<'_>) -> String {
    let (format, height, aspect) = match spec.standard {
        TvStandard::Pal => ("PAL", 576, "4:3"),
        TvStandard::Ntsc => ("NTSC", 480, "4:3"),
    };
    let aspect = if spec.aspect == AspectRatio::SixteenNine {
        "16:9"
    } else {
        aspect
    };
    format!(
        "<subpictures format=\"{format}\">\n  <stream>\n    <textsub filename=\"{}\" characterset=\"UTF-8\" \
fontsize=\"28.0\" font=\"sans-serif\" fill-color=\"#ffffff\" outline-color=\"#000000\" \
outline-thickness=\"2.0\" horizontal-alignment=\"center\" vertical-alignment=\"bottom\" \
left-margin=\"60\" right-margin=\"60\" top-margin=\"20\" bottom-margin=\"30\" \
movie-fps=\"{}/{}\" movie-width=\"720\" movie-height=\"{height}\" aspect=\"{aspect}\" force=\"{}\" />\n  </stream>\n</subpictures>\n",
        xml_escape(&spec.srt.display().to_string()),
        spec.fps.num,
        spec.fps.den,
        if spec.forced { "yes" } else { "no" }
    )
}

pub fn write_textsub_xml(path: &Path, spec: &TextSubtitleSpec<'_>) -> Result<(), MenuError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, build_textsub_xml(spec))?;
    Ok(())
}

/// Desplaza los timestamps de un SRT. Los tiempos negativos se recortan a cero.
pub fn shift_srt(input: &str, delay_ms: i64) -> Result<String, MenuError> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let mut output = String::new();
    for block in normalized.split("\n\n") {
        let block = block.trim_matches('\n');
        if block.trim().is_empty() {
            continue;
        }
        let lines: Vec<&str> = block.lines().collect();
        let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
            output.push_str(block);
            output.push_str("\n\n");
            continue;
        };
        let (start, end) = parse_srt_timing(lines[timing_index])?;
        let shifted_start = (start + delay_ms).max(0);
        let shifted_end = (end + delay_ms).max(shifted_start + 1);
        output.push_str(&lines[..timing_index].join("\n"));
        output.push('\n');
        output.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(shifted_start),
            format_srt_time(shifted_end)
        ));
        output.push_str(&lines[timing_index + 1..].join("\n"));
        output.push_str("\n\n");
    }
    if output.is_empty() {
        return Err(MenuError::InvalidSubtitle(
            "el archivo SRT no contiene bloques válidos".to_string(),
        ));
    }
    Ok(output)
}

fn parse_srt_timing(line: &str) -> Result<(i64, i64), MenuError> {
    let (start, rest) = line
        .split_once("-->")
        .ok_or_else(|| MenuError::InvalidSubtitle(format!("línea de tiempo inválida: {line}")))?;
    let end = rest
        .split_whitespace()
        .next()
        .ok_or_else(|| MenuError::InvalidSubtitle(format!("fin inválido: {line}")))?;
    Ok((parse_srt_time(start.trim())?, parse_srt_time(end)?))
}

fn parse_srt_time(value: &str) -> Result<i64, MenuError> {
    let value = value.replace(',', ".");
    let mut parts = value.split(':');
    let hours = parts.next().and_then(|part| part.parse::<i64>().ok());
    let minutes = parts.next().and_then(|part| part.parse::<i64>().ok());
    let seconds = parts.next().and_then(|part| part.parse::<f64>().ok());
    let (Some(hours), Some(minutes), Some(seconds)) = (hours, minutes, seconds) else {
        return Err(MenuError::InvalidSubtitle(format!(
            "timestamp inválido: {value}"
        )));
    };
    Ok(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as i64)
}

fn format_srt_time(milliseconds: i64) -> String {
    let millis = milliseconds.max(0);
    let hours = millis / 3_600_000;
    let minutes = (millis % 3_600_000) / 60_000;
    let seconds = (millis % 60_000) / 1000;
    let remainder = millis % 1000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{remainder:03}")
}

fn validate_subpicture_png(path: &Path, canvas: CanvasSpec) -> Result<usize, MenuError> {
    let image = image::open(path)?.to_rgba8();
    let (width, height) = image.dimensions();
    if width != canvas.width as u32 || height != canvas.height as u32 {
        return Err(MenuError::WrongDimensions {
            path: path.display().to_string(),
            width,
            height,
            expected_width: canvas.width as u32,
            expected_height: canvas.height as u32,
        });
    }
    let mut colors = HashSet::new();
    for pixel in image.pixels() {
        colors.insert(pixel.0);
    }
    if colors.len() > MAX_SUBPICTURE_COLORS {
        return Err(MenuError::TooManyColors {
            path: path.display().to_string(),
            colors: colors.len(),
        });
    }
    Ok(colors.len())
}

fn fit_background(
    path: &Path,
    width: u32,
    height: u32,
    scale_mode: MenuScaleMode,
    dim_percent: u8,
    blur_px: u8,
) -> Result<RgbaImage, MenuError> {
    let source = image::open(path)?.to_rgba8();
    let (source_width, source_height) = source.dimensions();
    if source_width == 0 || source_height == 0 {
        return Err(MenuError::InvalidButton(
            "el fondo tiene dimensiones cero".to_string(),
        ));
    }
    let mut canvas = RgbaImage::from_pixel(width, height, Rgba([16, 24, 40, 255]));
    match scale_mode {
        MenuScaleMode::Cover => {
            let scale =
                (width as f64 / source_width as f64).max(height as f64 / source_height as f64);
            let scaled_width = (source_width as f64 * scale).round().max(1.0) as u32;
            let scaled_height = (source_height as f64 * scale).round().max(1.0) as u32;
            let scaled =
                image::imageops::resize(&source, scaled_width, scaled_height, FilterType::Lanczos3);
            let offset_x = (scaled_width.saturating_sub(width)) / 2;
            let offset_y = (scaled_height.saturating_sub(height)) / 2;
            let cropped = image::imageops::crop_imm(
                &scaled,
                offset_x,
                offset_y,
                width.min(scaled_width),
                height.min(scaled_height),
            )
            .to_image();
            image::imageops::overlay(&mut canvas, &cropped, 0, 0);
        }
        MenuScaleMode::Contain => {
            let scale =
                (width as f64 / source_width as f64).min(height as f64 / source_height as f64);
            let scaled_width = (source_width as f64 * scale).round().max(1.0) as u32;
            let scaled_height = (source_height as f64 * scale).round().max(1.0) as u32;
            let scaled =
                image::imageops::resize(&source, scaled_width, scaled_height, FilterType::Lanczos3);
            let offset_x = (width as i64 - scaled_width as i64) / 2;
            let offset_y = (height as i64 - scaled_height as i64) / 2;
            image::imageops::overlay(&mut canvas, &scaled, offset_x, offset_y);
        }
        MenuScaleMode::Stretch => {
            let scaled = image::imageops::resize(&source, width, height, FilterType::Lanczos3);
            canvas = scaled;
        }
    }
    if blur_px > 0 {
        canvas = image::imageops::blur(&canvas, blur_px as f32);
    }
    if dim_percent > 0 {
        let factor = (100 - dim_percent.min(80)) as f32 / 100.0;
        for pixel in canvas.pixels_mut() {
            pixel.0[0] = (pixel.0[0] as f32 * factor) as u8;
            pixel.0[1] = (pixel.0[1] as f32 * factor) as u8;
            pixel.0[2] = (pixel.0[2] as f32 * factor) as u8;
        }
    }
    Ok(canvas)
}

fn draw_button_masks(
    highlight: &mut RgbaImage,
    select: &mut RgbaImage,
    rect: PixelRect,
    style: ButtonStyleKind,
    shape: ButtonShapeMask,
    highlight_color: Rgba<u8>,
    select_color: Rgba<u8>,
) {
    if shape != ButtonShapeMask::Rectangle {
        fill_shape(select, rect, shape, select_color);
        draw_shape_border(highlight, rect, shape, highlight_color, 4);
        draw_shape_border(select, rect, shape, highlight_color, 4);
        return;
    }
    match style {
        ButtonStyleKind::TextOnly | ButtonStyleKind::Hotspot => {
            draw_underline(highlight, rect, highlight_color);
            draw_underline(select, rect, select_color);
            if style == ButtonStyleKind::Hotspot {
                draw_left_indicator(highlight, rect, highlight_color);
                draw_left_indicator(select, rect, select_color);
            }
        }
        ButtonStyleKind::ThumbnailCard
        | ButtonStyleKind::GlassPill
        | ButtonStyleKind::ClassicFrame
        | ButtonStyleKind::PlayBadge => {
            draw_border(highlight, rect, highlight_color, 4);
            fill_rect(select, rect, select_color);
            draw_border(select, rect, highlight_color, 4);
        }
    }
}

fn fill_shape(image: &mut RgbaImage, rect: PixelRect, shape: ButtonShapeMask, color: Rgba<u8>) {
    let (width_px, height_px) = image.dimensions();
    for y in rect.y0 as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x1 as u32 {
            if x < width_px && y < height_px && inside_shape(shape, rect, x as f64, y as f64) {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn draw_shape_border(
    image: &mut RgbaImage,
    rect: PixelRect,
    shape: ButtonShapeMask,
    color: Rgba<u8>,
    thickness: u16,
) {
    let inner = PixelRect::new(
        rect.x0.saturating_add(thickness / 2),
        rect.y0.saturating_add(thickness / 2),
        rect.x1.saturating_sub(thickness / 2),
        rect.y1.saturating_sub(thickness / 2),
    );
    if inner.width() == 0 || inner.height() == 0 {
        return;
    }
    let (width_px, height_px) = image.dimensions();
    for y in rect.y0 as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x1 as u32 {
            if x >= width_px || y >= height_px {
                continue;
            }
            let outside = inside_shape(shape, rect, x as f64, y as f64);
            let inside = inside_shape(shape, inner, x as f64, y as f64);
            if outside && !inside {
                image.put_pixel(x, y, color);
            }
        }
    }
}

/// Escribe una máscara de 8 bits (255 dentro, 0 fuera) que FFmpeg usa con
/// `alphamerge` para recortar el vídeo de un botón con la forma elegida.
pub fn write_button_shape_mask(
    shape: ButtonShapeMask,
    width: u32,
    height: u32,
    path: &Path,
) -> Result<(), MenuError> {
    if width == 0 || height == 0 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err(MenuError::InvalidButton(
            "dimensiones de máscara inválidas".to_string(),
        ));
    }
    let rect = PixelRect::new(0, 0, width as u16, height as u16);
    let mut image = GrayImage::from_pixel(width, height, Luma([0]));
    for y in 0..height {
        for x in 0..width {
            if inside_shape(shape, rect, x as f64, y as f64) {
                image.put_pixel(x, y, Luma([255]));
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(path)?;
    Ok(())
}

fn inside_shape(shape: ButtonShapeMask, rect: PixelRect, x: f64, y: f64) -> bool {
    let left = rect.x0 as f64;
    let top = rect.y0 as f64;
    let right = rect.x1 as f64;
    let bottom = rect.y1 as f64;
    let width = right - left;
    let height = bottom - top;
    if width <= 0.0 || height <= 0.0 {
        return false;
    }
    match shape {
        ButtonShapeMask::Rectangle => true,
        ButtonShapeMask::RoundedRect => {
            let radius = (width.min(height) / 6.0).max(1.0);
            let cx = x.clamp(left + radius, right - radius);
            let cy = y.clamp(top + radius, bottom - radius);
            let dx = x - cx;
            let dy = y - cy;
            dx * dx + dy * dy <= radius * radius
        }
        ButtonShapeMask::Circle => {
            let cx = left + width / 2.0;
            let cy = top + height / 2.0;
            let rx = width / 2.0;
            let ry = height / 2.0;
            let nx = (x - cx) / rx;
            let ny = (y - cy) / ry;
            nx * nx + ny * ny <= 1.0
        }
        ButtonShapeMask::Star => {
            let cx = left + width / 2.0;
            let cy = top + height / 2.0;
            let outer = width.min(height) / 2.0;
            let inner = outer * 0.45;
            let mut points = Vec::with_capacity(10);
            for index in 0..10 {
                let angle =
                    -std::f64::consts::FRAC_PI_2 + index as f64 * std::f64::consts::PI / 5.0;
                let radius = if index % 2 == 0 { outer } else { inner };
                points.push((cx + angle.cos() * radius, cy + angle.sin() * radius));
            }
            point_in_polygon(x, y, &points)
        }
    }
}

fn point_in_polygon(x: f64, y: f64, points: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut previous = points.len() - 1;
    for current in 0..points.len() {
        let (xi, yi) = points[current];
        let (xj, yj) = points[previous];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        previous = current;
    }
    inside
}

fn draw_underline(image: &mut RgbaImage, rect: PixelRect, color: Rgba<u8>) {
    let thickness = 4.min(rect.height().max(1)) as u32;
    let (width_px, height_px) = image.dimensions();
    for y in rect.y1.saturating_sub(thickness as u16) as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x1 as u32 {
            if x < width_px && y < height_px {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn draw_left_indicator(image: &mut RgbaImage, rect: PixelRect, color: Rgba<u8>) {
    let thickness = 3.min(rect.width().max(1)) as u32;
    let (width_px, height_px) = image.dimensions();
    for y in rect.y0 as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x0.saturating_add(thickness as u16) as u32 {
            if x < width_px && y < height_px {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn palette_png_color(color: SubpictureColor, fallback: [u8; 4]) -> Rgba<u8> {
    if color.alpha == 0 {
        return Rgba(fallback);
    }
    Rgba([color.rgb[0], color.rgb[1], color.rgb[2], 255])
}

fn draw_border(image: &mut RgbaImage, rect: PixelRect, color: Rgba<u8>, width: u16) {
    let width = width.min(rect.width().max(1) / 2).max(1) as u32;
    let (width_px, height_px) = image.dimensions();
    for y in rect.y0 as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x1 as u32 {
            if x >= width_px || y >= height_px {
                continue;
            }
            let inner_x = x >= rect.x0 as u32 + width && x < rect.x1 as u32 - width;
            let inner_y = y >= rect.y0 as u32 + width && y < rect.y1 as u32 - width;
            if !inner_x || !inner_y {
                image.put_pixel(x, y, color);
            }
        }
    }
}

fn fill_rect(image: &mut RgbaImage, rect: PixelRect, color: Rgba<u8>) {
    let (width, height) = image.dimensions();
    for y in rect.y0 as u32..rect.y1 as u32 {
        for x in rect.x0 as u32..rect.x1 as u32 {
            if x < width && y < height {
                image.put_pixel(x, y, color);
            }
        }
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
    use dvdforge_domain::{
        AspectRatio, ButtonId, FrameRate, LogicalRect, MenuButton, MenuId, NavPolicy, TvStandard,
    };

    fn canvas() -> CanvasSpec {
        CanvasSpec::new(TvStandard::Ntsc, AspectRatio::FourThree)
    }

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dvdforge-menu-test-{}-{name}", std::process::id()))
    }

    fn buttons() -> Vec<CompiledButton> {
        vec![
            CompiledButton {
                id: ButtonId::from_u128(1),
                name: "play-all".into(),
                rect: PixelRect::new(60, 132, 240, 204),
                style_kind: ButtonStyleKind::ClassicFrame,
                shape_mask: ButtonShapeMask::Rectangle,
            },
            CompiledButton {
                id: ButtonId::from_u128(2),
                name: "episode".into(),
                rect: PixelRect::new(300, 132, 480, 204),
                style_kind: ButtonStyleKind::ThumbnailCard,
                shape_mask: ButtonShapeMask::Rectangle,
            },
        ]
    }

    fn navigation() -> NavigationGraph {
        let menu_buttons = vec![
            MenuButton {
                id: ButtonId::from_u128(1),
                name: "play-all".into(),
                rect: LogicalRect::new(60.0, 132.0, 180.0, 72.0),
                z: 0,
                action: ButtonAction::Noop,
                thumbnail: None,
                label: None,
                style_kind: ButtonStyleKind::ClassicFrame,
                font_family: "sans-serif".into(),
                font_size: 22,
                font_weight: "normal".into(),
                text_color: "#FFFFFF".into(),
                shadow_color: "#000000".into(),
                fill_color: "#0F172A".into(),
                shape_mask: dvdforge_domain::ButtonShapeMask::Rectangle,
                motion_preview: None,
                nav_override: NavOverride::default(),
                enabled: true,
            },
            MenuButton {
                id: ButtonId::from_u128(2),
                name: "episode".into(),
                rect: LogicalRect::new(300.0, 132.0, 180.0, 72.0),
                z: 0,
                action: ButtonAction::Noop,
                thumbnail: None,
                label: None,
                style_kind: ButtonStyleKind::ClassicFrame,
                font_family: "sans-serif".into(),
                font_size: 22,
                font_weight: "normal".into(),
                text_color: "#FFFFFF".into(),
                shadow_color: "#000000".into(),
                fill_color: "#0F172A".into(),
                shape_mask: dvdforge_domain::ButtonShapeMask::Rectangle,
                motion_preview: None,
                nav_override: NavOverride::default(),
                enabled: true,
            },
        ];
        resolve_navigation(
            MenuId::from_u128(9),
            &canvas(),
            &menu_buttons,
            &NavPolicy::default(),
            Some(ButtonId::from_u128(1)),
        )
    }

    #[test]
    fn generates_four_color_layers() {
        let dir = test_dir("layers");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let background = dir.join("source.png");
        RgbaImage::from_pixel(320, 240, Rgba([20, 40, 60, 255]))
            .save(&background)
            .expect("background");
        let layers = generate_menu_layers(&MenuLayerRequest {
            canvas: canvas(),
            background_source: background,
            buttons: buttons(),
            palette: SubpicturePalette::default(),
            scale_mode: MenuScaleMode::Cover,
            dim_percent: 0,
            blur_px: 0,
            output_dir: dir.join("layers"),
        })
        .expect("layers");
        assert!(layers.background_png.is_file());
        assert!(layers.highlight_png.is_file());
        assert!(layers.select_png.is_file());
        let highlight = image::open(&layers.highlight_png)
            .expect("highlight")
            .to_rgba8();
        let colors: HashSet<[u8; 4]> = highlight.pixels().map(|pixel| pixel.0).collect();
        assert!(colors.len() <= MAX_SUBPICTURE_COLORS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builds_spumux_xml_with_even_rectangles_and_directions() {
        let dir = test_dir("xml");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let background = dir.join("source.png");
        RgbaImage::from_pixel(720, 480, Rgba([20, 40, 60, 255]))
            .save(&background)
            .expect("background");
        let layers = generate_menu_layers(&MenuLayerRequest {
            canvas: canvas(),
            background_source: background,
            buttons: buttons(),
            palette: SubpicturePalette::default(),
            scale_mode: MenuScaleMode::Cover,
            dim_percent: 0,
            blur_px: 0,
            output_dir: dir.join("layers"),
        })
        .expect("layers");
        let request = MenuCompileRequest {
            menu_id: MenuId::from_u128(9),
            canvas: canvas(),
            background_png: layers.background_png,
            highlight_png: layers.highlight_png,
            select_png: layers.select_png,
            buttons: buttons(),
            force: true,
        };
        let xml = build_spumux_xml(&request, &navigation()).expect("xml");
        assert!(xml.contains("x0=\"60\""));
        assert!(xml.contains("x1=\"240\""));
        assert!(xml.contains("right=\"b2\""));
        assert!(xml.contains("left=\"b1\""));
        assert!(xml.contains("force=\"yes\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn styles_keep_even_geometry_and_four_colors() {
        let dir = test_dir("styles");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let background = dir.join("source.png");
        RgbaImage::from_pixel(720, 480, Rgba([20, 40, 60, 255]))
            .save(&background)
            .expect("background");
        let styles = [
            ButtonStyleKind::TextOnly,
            ButtonStyleKind::Hotspot,
            ButtonStyleKind::ThumbnailCard,
            ButtonStyleKind::GlassPill,
            ButtonStyleKind::ClassicFrame,
            ButtonStyleKind::PlayBadge,
        ];
        for (index, style) in styles.into_iter().enumerate() {
            let button = CompiledButton {
                id: ButtonId::from_u128(index as u128 + 1),
                name: format!("style-{index}"),
                rect: PixelRect::new(60 + index as u16 * 4, 132, 240, 204),
                style_kind: style,
                shape_mask: ButtonShapeMask::Rectangle,
            };
            let layers = generate_menu_layers(&MenuLayerRequest {
                canvas: canvas(),
                background_source: background.clone(),
                buttons: vec![button],
                palette: SubpicturePalette::default(),
                scale_mode: MenuScaleMode::Cover,
                dim_percent: 20,
                blur_px: 0,
                output_dir: dir.join(format!("layers-{index}")),
            })
            .expect("layers");
            for path in [&layers.highlight_png, &layers.select_png] {
                let image = image::open(path).expect("mask").to_rgba8();
                let colors: HashSet<[u8; 4]> = image.pixels().map(|pixel| pixel.0).collect();
                assert!(colors.len() <= MAX_SUBPICTURE_COLORS, "{style:?} {path:?}");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_masks_with_more_than_four_colors() {
        let dir = test_dir("colors");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let background = dir.join("source.png");
        RgbaImage::from_pixel(720, 480, Rgba([20, 40, 60, 255]))
            .save(&background)
            .expect("background");
        let layers = generate_menu_layers(&MenuLayerRequest {
            canvas: canvas(),
            background_source: background,
            buttons: buttons(),
            palette: SubpicturePalette::default(),
            scale_mode: MenuScaleMode::Cover,
            dim_percent: 0,
            blur_px: 0,
            output_dir: dir.join("layers"),
        })
        .expect("layers");
        let mut bad = image::open(&layers.highlight_png)
            .expect("highlight")
            .to_rgba8();
        for (index, color) in [
            Rgba([1, 2, 3, 255]),
            Rgba([4, 5, 6, 255]),
            Rgba([7, 8, 9, 255]),
            Rgba([10, 11, 12, 255]),
            Rgba([13, 14, 15, 255]),
        ]
        .into_iter()
        .enumerate()
        {
            bad.put_pixel(index as u32, 0, color);
        }
        let bad_path = dir.join("bad.png");
        bad.save(&bad_path).expect("bad png");
        let request = MenuCompileRequest {
            menu_id: MenuId::from_u128(9),
            canvas: canvas(),
            background_png: layers.background_png,
            highlight_png: bad_path,
            select_png: layers.select_png,
            buttons: buttons(),
            force: false,
        };
        let error = build_spumux_xml(&request, &navigation()).expect_err("must reject");
        assert!(matches!(error, MenuError::TooManyColors { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frame_rate_type_is_available_to_callers() {
        assert_eq!(FrameRate::PAL.as_f64(), 25.0);
    }

    #[test]
    fn srt_delay_shifts_timestamps_and_keeps_text() {
        let input =
            "1\n00:00:01,000 --> 00:00:03,500\nHola\n\n2\n00:00:05,000 --> 00:00:06,000\nMundo\n";
        let shifted = shift_srt(input, 500).expect("shift");
        assert!(shifted.contains("00:00:01,500 --> 00:00:04,000"));
        assert!(shifted.contains("00:00:05,500 --> 00:00:06,500"));
        assert!(shifted.contains("Hola"));
        assert!(shifted.contains("Mundo"));

        let negative = shift_srt(input, -2_000).expect("negative shift");
        assert!(negative.contains("00:00:00,000 --> 00:00:01,500"));
        assert!(negative.contains("00:00:03,000 --> 00:00:04,000"));
    }

    #[test]
    fn shape_masks_keep_alpha_inside_and_outside_the_geometry() {
        let dir = test_dir("shape-masks");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let circle = dir.join("circle.png");
        write_button_shape_mask(ButtonShapeMask::Circle, 100, 100, &circle).expect("circle mask");
        let image = image::open(&circle).expect("open circle").to_luma8();
        assert_eq!(image.get_pixel(50, 50)[0], 255);
        assert_eq!(image.get_pixel(1, 1)[0], 0);

        let star = dir.join("star.png");
        write_button_shape_mask(ButtonShapeMask::Star, 100, 100, &star).expect("star mask");
        let image = image::open(&star).expect("open star").to_luma8();
        assert_eq!(image.get_pixel(50, 50)[0], 255);
        assert_eq!(image.get_pixel(0, 0)[0], 0);

        let rectangle = dir.join("rectangle.png");
        write_button_shape_mask(ButtonShapeMask::Rectangle, 10, 10, &rectangle).expect("rect mask");
        let image = image::open(&rectangle).expect("open rect").to_luma8();
        assert_eq!(image.get_pixel(0, 0)[0], 255);
        assert_eq!(image.get_pixel(9, 9)[0], 255);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn textsub_xml_uses_project_fps_and_forced_flag() {
        let xml = build_textsub_xml(&TextSubtitleSpec {
            srt: Path::new("/tmp/sub & title.srt"),
            fps: FrameRate::FPS_23_976,
            standard: TvStandard::Ntsc,
            aspect: AspectRatio::SixteenNine,
            forced: true,
        });
        assert!(xml.contains("format=\"NTSC\""));
        assert!(xml.contains("movie-fps=\"24000/1001\""));
        assert!(xml.contains("movie-height=\"480\""));
        assert!(xml.contains("aspect=\"16:9\""));
        assert!(xml.contains("force=\"yes\""));
        assert!(xml.contains("sub &amp; title.srt"));
    }
}
