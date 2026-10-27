//! High-level menu-to-IG compiler.

use crate::opcodes::HdmvCommand;
use crate::palette::Palette;
use crate::pes::pes_packets_from_segments;
use crate::rle::encode_rle;
use crate::segments::{
    end_of_display_set_segment, interactive_composition_segment, palette_definition_segment,
    window_definition_segment, ButtonDefinition, CompositionObject, InteractiveComposition,
    PageDefinition, Segment, Window, NO_BUTTON, SEGMENT_ODS,
};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaBitmap {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

impl RgbaBitmap {
    pub fn new(width: u16, height: u16, pixels: Vec<u8>) -> Result<Self, HdmvError> {
        let expected = width as usize * height as usize * 4;
        if width == 0 || height == 0 || pixels.len() != expected {
            return Err(HdmvError::InvalidBitmap {
                width,
                height,
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn solid(width: u16, height: u16, rgba: [u8; 4]) -> Result<Self, HdmvError> {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            pixels.extend_from_slice(&rgba);
        }
        Self::new(width, height, pixels)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdmvButtonState {
    pub normal: RgbaBitmap,
    pub selected: RgbaBitmap,
    pub activated: RgbaBitmap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdmvButton {
    pub id: u16,
    pub x: u16,
    pub y: u16,
    pub up: Option<u16>,
    pub down: Option<u16>,
    pub left: Option<u16>,
    pub right: Option<u16>,
    pub commands: Vec<HdmvCommand>,
    pub state: HdmvButtonState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdmvMenu {
    pub width: u16,
    pub height: u16,
    pub frame_rate_code: u8,
    /// Optional full-screen menu artwork. When present it is the first
    /// composition object and is painted before the button states.
    pub background: Option<RgbaBitmap>,
    pub buttons: Vec<HdmvButton>,
    pub default_selected: Option<u16>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HdmvError {
    #[error("el menú HDMV no tiene botones")]
    EmptyMenu,
    #[error("dimensiones de menú inválidas: {0}x{1}")]
    InvalidSize(u16, u16),
    #[error("bitmap {width}x{height} esperaba {expected} bytes RGBA y recibió {actual}")]
    InvalidBitmap {
        width: u16,
        height: u16,
        expected: usize,
        actual: usize,
    },
    #[error("el estado del botón {button} mide {width}x{height} y se esperaba {expected_w}x{expected_h}")]
    StateSizeMismatch {
        button: u16,
        width: u16,
        height: u16,
        expected_w: u16,
        expected_h: u16,
    },
    #[error("el objeto gráfico es demasiado grande")]
    ObjectTooLarge,
    #[error("segmento HDMV inválido: {0}")]
    Segment(String),
    #[error("PES HDMV inválido: {0}")]
    Pes(String),
}

/// Compiles a menu to the complete byte stream of IG segments.
pub fn compile_ig_segments(menu: &HdmvMenu) -> Result<Vec<Segment>, HdmvError> {
    if menu.width == 0 || menu.height == 0 {
        return Err(HdmvError::InvalidSize(menu.width, menu.height));
    }
    if menu.buttons.is_empty() {
        return Err(HdmvError::EmptyMenu);
    }

    let mut all_pixels: Vec<&[u8]> = Vec::new();
    if let Some(background) = &menu.background {
        all_pixels.push(&background.pixels);
    }
    for button in &menu.buttons {
        for state in [
            &button.state.normal,
            &button.state.selected,
            &button.state.activated,
        ] {
            if state.width != button_width(button) || state.height != button_height(button) {
                return Err(HdmvError::StateSizeMismatch {
                    button: button.id,
                    width: state.width,
                    height: state.height,
                    expected_w: button_width(button),
                    expected_h: button_height(button),
                });
            }
            all_pixels.push(&state.pixels);
        }
    }
    let palette = Palette::from_pixels(all_pixels).map_err(|_| HdmvError::ObjectTooLarge)?;

    // The interactive composition segment opens the display set. Blu-ray
    // decoders only accept palette/window/object definitions after the ICS
    // has marked the display set as being decoded.
    let mut display_segments = Vec::new();
    display_segments.push(palette_definition_segment(&palette, 0, 0));
    let window = Window {
        id: 0,
        x: 0,
        y: 0,
        width: menu.width,
        height: menu.height,
    };
    display_segments.push(window_definition_segment(window));

    let mut next_object = 1u16;
    let mut in_objects = Vec::new();
    if let Some(background) = &menu.background {
        let object_id = next_object;
        next_object = next_object.saturating_add(1);
        let indexed = palette.index_pixels(&background.pixels);
        display_segments.extend(split_object_segments(
            object_id,
            background.width,
            background.height,
            &encode_rle(&indexed),
        )?);
        in_objects.push(CompositionObject {
            object_id,
            window_id: 0,
            x: 0,
            y: 0,
        });
    }

    let mut button_definitions = Vec::with_capacity(menu.buttons.len());
    for button in &menu.buttons {
        let width = button_width(button);
        let height = button_height(button);
        let normal = next_object;
        next_object = next_object.saturating_add(1);
        let selected = next_object;
        next_object = next_object.saturating_add(1);
        let activated = next_object;
        next_object = next_object.saturating_add(1);
        for (object_id, state) in [
            (normal, &button.state.normal),
            (selected, &button.state.selected),
            (activated, &button.state.activated),
        ] {
            let indexed = palette.index_pixels(&state.pixels);
            display_segments.extend(split_object_segments(
                object_id,
                width,
                height,
                &encode_rle(&indexed),
            )?);
        }
        button_definitions.push(ButtonDefinition {
            id: button.id,
            x: button.x,
            y: button.y,
            up: button.up,
            down: button.down,
            left: button.left,
            right: button.right,
            normal_object: normal,
            selected_object: selected,
            activated_object: activated,
            commands: button.commands.clone(),
        });
    }

    let default_selected = menu
        .default_selected
        .filter(|id| menu.buttons.iter().any(|button| button.id == *id))
        .unwrap_or(menu.buttons[0].id);
    let composition = InteractiveComposition {
        video_width: menu.width,
        video_height: menu.height,
        frame_rate_code: menu.frame_rate_code,
        composition_number: 0,
        // Epoch start: the menu display set is self-contained and safe for
        // random access by hardware decoders.
        composition_state: 2,
        user_timeout_duration: 0,
        pages: vec![PageDefinition {
            id: 0,
            version: 0,
            default_selected_button: default_selected,
            default_activated_button: NO_BUTTON,
            palette_id: 0,
            buttons: button_definitions,
            in_objects,
            window,
        }],
    };
    let mut segments = Vec::with_capacity(display_segments.len() + 2);
    segments.push(
        interactive_composition_segment(&composition)
            .map_err(|error| HdmvError::Segment(error.to_string()))?,
    );
    segments.extend(display_segments);
    segments.push(end_of_display_set_segment());
    Ok(segments)
}

/// Compiles a menu and wraps the segments in PES/IG packets.
///
/// Blu-ray graphics decoders treat every PES packet as one segment (ODS
/// fragments are joined by object id across packets). Packing several
/// segments into the same PES makes libbluray discard everything after the
/// first one, so each segment gets its own PES packet. ODS fragmentation is
/// handled at segment level and never splits a segment across PES packets.
pub fn compile_ig_pes(menu: &HdmvMenu, pts_90k: u64) -> Result<Vec<u8>, HdmvError> {
    let packets = compile_ig_pes_packets(menu, pts_90k)?;
    let mut output = Vec::new();
    for packet in packets {
        output.extend_from_slice(&packet);
    }
    Ok(output)
}

pub fn compile_ig_pes_packets(menu: &HdmvMenu, pts_90k: u64) -> Result<Vec<Vec<u8>>, HdmvError> {
    let segments = compile_ig_segments(menu)?;
    pes_packets_from_segments(&segments, pts_90k).map_err(|error| HdmvError::Pes(error.to_string()))
}

/// Builds a standalone 192-byte M2TS with PAT/PMT (stream type 0x91) and the
/// Interactive Graphics PID 0x1400.
pub fn compile_ig_m2ts(menu: &HdmvMenu, pts_90k: u64) -> Result<Vec<u8>, HdmvError> {
    let packets = compile_ig_pes_packets(menu, pts_90k)?;
    Ok(crate::m2ts::m2ts_from_pes_packets(&packets))
}

fn button_width(button: &HdmvButton) -> u16 {
    button.state.normal.width
}

fn button_height(button: &HdmvButton) -> u16 {
    button.state.normal.height
}

/// Splits an ODS into continuation segments. libbluray joins continuation
/// payloads from byte 7 (after type/length/id/version/sequence), so only the
/// first fragment carries `data_len`, width and height.
fn split_object_segments(
    object_id: u16,
    width: u16,
    height: u16,
    rle: &[u8],
) -> Result<Vec<Segment>, HdmvError> {
    let total_data = 4usize.saturating_add(rle.len());
    // The ODS object data length is a 24-bit field shared by every fragment.
    // Reject oversized objects instead of silently truncating the length.
    if total_data > 0xFF_FFFF {
        return Err(HdmvError::ObjectTooLarge);
    }
    // Keep each completed segment below the PES payload budget so a segment is
    // never split across PES packets.
    let first_capacity = 60_000usize.saturating_sub(11);
    let next_capacity = 60_000usize.saturating_sub(4);
    if rle.len() <= first_capacity {
        let mut payload = Vec::with_capacity(11 + rle.len());
        payload.extend_from_slice(&object_id.to_be_bytes());
        payload.push(0);
        payload.push(0xC0);
        payload.extend_from_slice(&(total_data as u32).to_be_bytes()[1..]);
        payload.extend_from_slice(&width.to_be_bytes());
        payload.extend_from_slice(&height.to_be_bytes());
        payload.extend_from_slice(rle);
        return Ok(vec![Segment::new(SEGMENT_ODS, payload)]);
    }

    let mut out = Vec::new();
    let (first, rest) = rle.split_at(first_capacity);
    let mut payload = Vec::with_capacity(11 + first.len());
    payload.extend_from_slice(&object_id.to_be_bytes());
    payload.push(0);
    payload.push(0x80); // first, not last
    payload.extend_from_slice(&(total_data as u32).to_be_bytes()[1..]);
    payload.extend_from_slice(&width.to_be_bytes());
    payload.extend_from_slice(&height.to_be_bytes());
    payload.extend_from_slice(first);
    out.push(Segment::new(SEGMENT_ODS, payload));

    let mut offset = 0usize;
    while offset < rest.len() {
        let end = (offset + next_capacity).min(rest.len());
        let last = end == rest.len();
        let mut payload = Vec::with_capacity(4 + (end - offset));
        payload.extend_from_slice(&object_id.to_be_bytes());
        payload.push(0);
        payload.push(if last { 0x40 } else { 0x00 });
        payload.extend_from_slice(&rest[offset..end]);
        out.push(Segment::new(SEGMENT_ODS, payload));
        offset = end;
    }
    Ok(out)
}

/// Maps common frame rates to the 4-bit IG video descriptor code.
pub fn frame_rate_code(fps: f64) -> u8 {
    match (fps * 1000.0).round() as u32 {
        23_976 => 1,
        24_000 => 2,
        25_000 => 3,
        29_970 => 4,
        30_000 => 5,
        50_000 => 6,
        59_940 => 7,
        60_000 => 8,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::jump_title;
    use crate::segments::{SEGMENT_END, SEGMENT_ICS, SEGMENT_ODS, SEGMENT_PDS, SEGMENT_WDS};

    fn bitmap(width: u16, height: u16, rgba: [u8; 4]) -> RgbaBitmap {
        RgbaBitmap::solid(width, height, rgba).expect("bitmap")
    }

    fn menu() -> HdmvMenu {
        HdmvMenu {
            width: 640,
            height: 360,
            frame_rate_code: 4,
            background: Some(bitmap(640, 360, [8, 12, 24, 255])),
            buttons: vec![
                HdmvButton {
                    id: 1,
                    x: 40,
                    y: 40,
                    up: None,
                    down: None,
                    left: None,
                    right: Some(2),
                    commands: vec![jump_title(1)],
                    state: HdmvButtonState {
                        normal: bitmap(220, 80, [20, 40, 70, 255]),
                        selected: bitmap(220, 80, [255, 255, 255, 255]),
                        activated: bitmap(220, 80, [34, 211, 238, 255]),
                    },
                },
                HdmvButton {
                    id: 2,
                    x: 40,
                    y: 150,
                    up: None,
                    down: None,
                    left: Some(1),
                    right: None,
                    commands: vec![jump_title(2)],
                    state: HdmvButtonState {
                        normal: bitmap(220, 80, [20, 40, 70, 255]),
                        selected: bitmap(220, 80, [255, 255, 255, 255]),
                        activated: bitmap(220, 80, [34, 211, 238, 255]),
                    },
                },
            ],
            default_selected: Some(1),
        }
    }

    #[test]
    fn compiles_the_canonical_segment_order() {
        let segments = compile_ig_segments(&menu()).expect("segments");
        let kinds: Vec<u8> = segments.iter().map(|segment| segment.kind).collect();
        assert_eq!(kinds.first().copied(), Some(SEGMENT_ICS));
        assert_eq!(kinds.get(1).copied(), Some(SEGMENT_PDS));
        assert_eq!(kinds.get(2).copied(), Some(SEGMENT_WDS));
        assert!(kinds.contains(&SEGMENT_ODS));
        assert_eq!(kinds[kinds.len() - 1], SEGMENT_END);
    }

    #[test]
    fn emits_one_pes_packet_per_segment() {
        let segments = compile_ig_segments(&menu()).expect("segments");
        let packets = compile_ig_pes_packets(&menu(), 90_000).expect("pes packets");
        assert_eq!(packets.len(), segments.len());
        assert!(packets.iter().all(|packet| packet[..4] == [0, 0, 1, 0xBD]));
    }

    #[test]
    fn compiles_to_pes_with_private_stream_one() {
        let pes = compile_ig_pes(&menu(), 0).expect("pes");
        assert_eq!(&pes[..4], &[0, 0, 1, 0xBD]);
        assert!(u16::from_be_bytes([pes[4], pes[5]]) > 0);
    }

    #[test]
    fn rejects_bitmap_size_mismatches() {
        let mut menu = menu();
        menu.buttons[0].state.selected = bitmap(10, 10, [0, 0, 0, 255]);
        assert!(matches!(
            compile_ig_segments(&menu),
            Err(HdmvError::StateSizeMismatch { .. })
        ));
    }

    #[test]
    fn supports_objects_larger_than_one_segment() {
        let width = 640u16;
        let height = 360u16;
        let pixels = (0..width as usize * height as usize)
            .flat_map(|index| [(index % 251) as u8, 0, 0, 255])
            .collect::<Vec<_>>();
        let mut large = menu();
        large.background = Some(RgbaBitmap::new(width, height, pixels).expect("bitmap"));
        let segments = compile_ig_segments(&large).expect("segments");
        let ods = segments
            .iter()
            .filter(|segment| segment.kind == SEGMENT_ODS)
            .count();
        assert!(ods > 1);
    }

    #[test]
    fn rejects_objects_that_overflow_the_ods_length_field() {
        let rle = vec![0u8; 0xFF_FFFC];
        assert!(matches!(
            split_object_segments(1, 4, 4, &rle),
            Err(HdmvError::ObjectTooLarge)
        ));
    }
}
