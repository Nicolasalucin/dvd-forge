use crate::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};

/// Broadcast standard of the disc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TvStandard {
    #[default]
    Ntsc,
    Pal,
}

impl TvStandard {
    pub const fn canvas_size(self) -> (u16, u16) {
        match self {
            Self::Ntsc => (720, 480),
            Self::Pal => (720, 576),
        }
    }

    pub const fn frame_rate(self) -> FrameRate {
        match self {
            Self::Ntsc => FrameRate::NTSC,
            Self::Pal => FrameRate::PAL,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ntsc => "NTSC",
            Self::Pal => "PAL",
        }
    }
}

/// Display aspect ratio of the DVD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AspectRatio {
    #[default]
    FourThree,
    SixteenNine,
}

impl AspectRatio {
    pub const fn label(self) -> &'static str {
        match self {
            Self::FourThree => "4:3",
            Self::SixteenNine => "16:9",
        }
    }
}

/// Logical canvas used by the visual editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CanvasSpec {
    pub width: u16,
    pub height: u16,
    pub tv_standard: TvStandard,
    pub aspect: AspectRatio,
    pub safe_margin_px: u16,
    pub grid_px: u16,
}

impl CanvasSpec {
    pub fn new(tv_standard: TvStandard, aspect: AspectRatio) -> Self {
        let (width, height) = tv_standard.canvas_size();
        Self {
            width,
            height,
            tv_standard,
            aspect,
            safe_margin_px: ((width as f32) * 0.05).round() as u16,
            grid_px: 8,
        }
    }

    pub fn validate(self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let (expected_width, expected_height) = self.tv_standard.canvas_size();
        if self.width != expected_width || self.height != expected_height {
            diagnostics.push(
                Diagnostic::error(
                    "E_CANVAS_SIZE",
                    format!(
                        "El lienzo {}x{} no corresponde a {} ({}x{}).",
                        self.width,
                        self.height,
                        self.tv_standard.label(),
                        expected_width,
                        expected_height
                    ),
                )
                .with_entity("canvas"),
            );
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            diagnostics.push(
                Diagnostic::error(
                    "E_CANVAS_NOT_EVEN",
                    "Las dimensiones del lienzo deben ser pares.",
                )
                .with_entity("canvas"),
            );
        }
        if self.grid_px == 0 {
            diagnostics.push(
                Diagnostic::warning("W_GRID_ZERO", "La rejilla tiene tamaño cero.")
                    .with_entity("canvas"),
            );
        }
        diagnostics
    }

    pub fn right(self) -> u16 {
        self.width
    }

    pub fn bottom(self) -> u16 {
        self.height
    }
}

impl Default for CanvasSpec {
    fn default() -> Self {
        Self::new(TvStandard::Ntsc, AspectRatio::FourThree)
    }
}

/// Rectangle in editor coordinates. Units are DVD pixels but the values are
/// floating point because Konva allows sub-pixel placement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl LogicalRect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(self) -> f64 {
        self.y + self.h
    }
}

/// Point in pixel space. Used for navigation centers and debugging.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Production rectangle. Coordinates are even and half-open:
/// `[x0, x1) x [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PixelRect {
    pub x0: u16,
    pub y0: u16,
    pub x1: u16,
    pub y1: u16,
}

impl PixelRect {
    pub const fn new(x0: u16, y0: u16, x1: u16, y1: u16) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn width(self) -> u16 {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(self) -> u16 {
        self.y1.saturating_sub(self.y0)
    }

    pub fn center(self) -> Point {
        Point::new(
            (self.x0 as f64 + self.x1 as f64) / 2.0,
            (self.y0 as f64 + self.y1 as f64) / 2.0,
        )
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x0 as f64
            && point.x < self.x1 as f64
            && point.y >= self.y0 as f64
            && point.y < self.y1 as f64
    }

    pub fn overlap_x(self, other: Self) -> u16 {
        self.x1.min(other.x1).saturating_sub(self.x0.max(other.x0))
    }

    pub fn overlap_y(self, other: Self) -> u16 {
        self.y1.min(other.y1).saturating_sub(self.y0.max(other.y0))
    }

    pub fn is_even(self) -> bool {
        self.x0.is_multiple_of(2)
            && self.y0.is_multiple_of(2)
            && self.x1.is_multiple_of(2)
            && self.y1.is_multiple_of(2)
    }

    pub fn is_valid(self, canvas: CanvasSpec) -> bool {
        self.x0 < self.x1
            && self.y0 < self.y1
            && self.x1 <= canvas.width
            && self.y1 <= canvas.height
            && self.is_even()
    }
}

/// Returns true when two half-open rectangles are separated by at least
/// `gap` pixels on one axis.
pub fn has_gap(a: PixelRect, b: PixelRect, gap: u16) -> bool {
    let gap = gap as u32;
    (a.x1 as u32 + gap <= b.x0 as u32)
        || (b.x1 as u32 + gap <= a.x0 as u32)
        || (a.y1 as u32 + gap <= b.y0 as u32)
        || (b.y1 as u32 + gap <= a.y0 as u32)
}

pub fn even_floor(value: f64) -> u16 {
    let n = value.floor().max(0.0) as u16;
    n & !1
}

pub fn even_ceil(value: f64) -> u16 {
    let n = value.ceil().max(0.0) as u16;
    (n + 1) & !1
}

/// Converts a logical editor rectangle into the even, half-open rectangle that
/// spumux will receive.
pub fn normalize_rect(
    logical: LogicalRect,
    canvas: CanvasSpec,
    min_size: u16,
) -> Result<PixelRect, Diagnostic> {
    if canvas.width == 0 || canvas.height == 0 {
        return Err(Diagnostic::error(
            "E_CANVAS_EMPTY",
            "El lienzo no tiene dimensiones.",
        ));
    }
    if !logical.x.is_finite()
        || !logical.y.is_finite()
        || !logical.w.is_finite()
        || !logical.h.is_finite()
    {
        return Err(Diagnostic::error(
            "E_RECT_NOT_FINITE",
            "El rectángulo contiene valores no finitos.",
        ));
    }
    if logical.w <= 0.0 || logical.h <= 0.0 {
        return Err(Diagnostic::error(
            "E_RECT_EMPTY",
            "El botón debe tener ancho y alto positivos.",
        ));
    }

    let min_size = min_size.max(2) & !1;
    let width = canvas.width as f64;
    let height = canvas.height as f64;

    let mut x0 = even_floor(logical.x.clamp(0.0, width));
    let mut y0 = even_floor(logical.y.clamp(0.0, height));
    let mut x1 = even_ceil((logical.x + logical.w).clamp(0.0, width));
    let mut y1 = even_ceil((logical.y + logical.h).clamp(0.0, height));

    if x1 <= x0 || y1 <= y0 {
        return Err(Diagnostic::error(
            "E_RECT_DEGENERATE",
            "El botón queda fuera del lienzo o sin superficie.",
        ));
    }

    (x0, x1) = grow_axis_to_min(x0, x1, canvas.width, min_size);
    (y0, y1) = grow_axis_to_min(y0, y1, canvas.height, min_size);

    if x1 - x0 < min_size || y1 - y0 < min_size {
        return Err(Diagnostic::error(
            "E_RECT_TOO_SMALL",
            format!("El botón no cabe en el lienzo con un tamaño mínimo de {min_size} px."),
        ));
    }

    Ok(PixelRect { x0, y0, x1, y1 })
}

fn grow_axis_to_min(mut low: u16, mut high: u16, limit: u16, min_size: u16) -> (u16, u16) {
    if high.saturating_sub(low) >= min_size {
        return (low, high);
    }

    let mut need = min_size - (high - low);
    let right_room = limit.saturating_sub(high);
    let take_right = need.min(right_room);
    high += take_right;
    need -= take_right;
    if need > 0 {
        low = low.saturating_sub(need);
    }
    (low, high)
}

/// Validates a set of production rectangles for a menu.
pub fn validate_layout(
    rects: &[PixelRect],
    gap_px: u16,
    canvas: CanvasSpec,
    max_buttons: usize,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if rects.len() > max_buttons {
        diagnostics.push(
            Diagnostic::error(
                "E_TOO_MANY_BUTTONS",
                format!(
                    "El menú tiene {} botones; el máximo es {}.",
                    rects.len(),
                    max_buttons
                ),
            )
            .with_entity("menu"),
        );
    }

    for (index, rect) in rects.iter().copied().enumerate() {
        let entity = format!("button:{index}");
        if rect.x0 >= rect.x1 || rect.y0 >= rect.y1 {
            diagnostics.push(
                Diagnostic::error("E_RECT_DEGENERATE", "El botón no tiene superficie.")
                    .with_entity(entity.clone()),
            );
            continue;
        }
        if !rect.is_even() {
            diagnostics.push(
                Diagnostic::error(
                    "E_RECT_NOT_EVEN",
                    "Las coordenadas del botón deben ser pares.",
                )
                .with_entity(entity.clone()),
            );
        }
        if !rect.is_valid(canvas) {
            diagnostics.push(
                Diagnostic::error(
                    "E_RECT_OUT_OF_BOUNDS",
                    "El botón sale del lienzo o usa coordenadas inválidas.",
                )
                .with_entity(entity.clone()),
            );
        }

        let safe = canvas.safe_margin_px;
        if rect.x0 < safe
            || rect.y0 < safe
            || rect.x1 > canvas.width.saturating_sub(safe)
            || rect.y1 > canvas.height.saturating_sub(safe)
        {
            diagnostics.push(
                Diagnostic::warning(
                    "W_BUTTON_OUTSIDE_SAFE_AREA",
                    "El botón invade el área segura de acción.",
                )
                .with_entity(entity),
            );
        }
    }

    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            if !has_gap(rects[i], rects[j], gap_px) {
                diagnostics.push(
                    Diagnostic::error(
                        "E_BUTTON_OVERLAP",
                        format!(
                            "Los botones {i} y {j} se solapan o no tienen una separación de {gap_px} px."
                        ),
                    )
                    .with_entity(format!("button:{i}")),
                );
            }
        }
    }

    diagnostics
}

/// Rational frame rate, kept exact for NTSC 30000/1001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameRate {
    pub num: u32,
    pub den: u32,
}

impl FrameRate {
    pub const NTSC: Self = Self {
        num: 30_000,
        den: 1_001,
    };
    pub const PAL: Self = Self { num: 25, den: 1 };
    pub const FPS_23_976: Self = Self {
        num: 24_000,
        den: 1_001,
    };
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    pub const FPS_29_97: Self = Self {
        num: 30_000,
        den: 1_001,
    };
    pub const FPS_50: Self = Self { num: 50, den: 1 };
    pub const FPS_59_94: Self = Self {
        num: 60_000,
        den: 1_001,
    };

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    pub fn is_valid(self) -> bool {
        self.num > 0 && self.den > 0
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        Self::NTSC
    }
}

/// Half-open frame range: `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FrameRange {
    pub start: u64,
    pub end: u64,
}

impl FrameRange {
    pub fn is_valid(self) -> bool {
        self.start < self.end
    }

    pub fn duration(self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ntsc() -> CanvasSpec {
        CanvasSpec::new(TvStandard::Ntsc, AspectRatio::FourThree)
    }

    #[test]
    fn normalizes_to_even_half_open_rect() {
        let rect = normalize_rect(LogicalRect::new(100.3, 149.7, 50.7, 40.1), ntsc(), 8)
            .expect("valid rect");
        assert_eq!(rect, PixelRect::new(100, 148, 152, 190));
        assert!(rect.is_even());
        assert_eq!(rect.width(), 52);
        assert_eq!(rect.height(), 42);
    }

    #[test]
    fn clamps_to_canvas_and_keeps_minimum_size() {
        let rect = normalize_rect(LogicalRect::new(715.0, 475.0, 20.0, 20.0), ntsc(), 8)
            .expect("valid rect");
        assert_eq!(rect.x1, 720);
        assert_eq!(rect.y1, 480);
        assert_eq!(rect.width(), 8);
        assert_eq!(rect.height(), 8);
    }

    #[test]
    fn rejects_empty_rect() {
        let error = normalize_rect(LogicalRect::new(10.0, 10.0, 0.0, 10.0), ntsc(), 8)
            .expect_err("must reject");
        assert_eq!(error.code, "E_RECT_EMPTY");
    }

    #[test]
    fn gap_uses_half_open_coordinates() {
        let a = PixelRect::new(60, 132, 252, 204);
        let b = PixelRect::new(256, 132, 448, 204);
        assert!(has_gap(a, b, 4));
        assert!(!has_gap(a, b, 5));
    }

    #[test]
    fn touching_buttons_need_a_positive_gap() {
        let a = PixelRect::new(0, 0, 100, 100);
        let b = PixelRect::new(100, 0, 200, 100);
        assert!(has_gap(a, b, 0));
        assert!(!has_gap(a, b, 2));
    }

    #[test]
    fn layout_detects_overlap() {
        let a = PixelRect::new(60, 132, 252, 204);
        let b = PixelRect::new(200, 132, 400, 204);
        let diagnostics = validate_layout(&[a, b], 2, ntsc(), 36);
        assert!(diagnostics.iter().any(|d| d.code == "E_BUTTON_OVERLAP"));
    }
}
