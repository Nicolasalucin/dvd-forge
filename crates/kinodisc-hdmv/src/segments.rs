//! Binary HDMV IG segments.
//!
//! Segment payload layouts follow libbluray's `ig_decode.c` / `pg_decode.c`.

use crate::opcodes::HdmvCommand;
use crate::palette::Palette;
use thiserror::Error;

pub const SEGMENT_PDS: u8 = 0x14;
pub const SEGMENT_ODS: u8 = 0x15;
pub const SEGMENT_WDS: u8 = 0x17;
pub const SEGMENT_ICS: u8 = 0x18;
pub const SEGMENT_END: u8 = 0x80;

pub const NO_BUTTON: u16 = 0xFFFF;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SegmentError {
    #[error(
        "el payload del segmento {kind:#04x} ocupa {length} bytes y no cabe en el campo de 16 bits"
    )]
    PayloadTooLarge { kind: u8, length: usize },
    #[error("el campo {field} no admite {value} elementos")]
    FieldOverflow { field: &'static str, value: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Segment {
    pub fn new(kind: u8, payload: Vec<u8>) -> Self {
        Self { kind, payload }
    }

    pub fn encode(&self) -> Result<Vec<u8>, SegmentError> {
        let length =
            u16::try_from(self.payload.len()).map_err(|_| SegmentError::PayloadTooLarge {
                kind: self.kind,
                length: self.payload.len(),
            })?;
        let mut out = Vec::with_capacity(3 + usize::from(length));
        out.push(self.kind);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositionObject {
    pub object_id: u16,
    pub window_id: u8,
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonDefinition {
    pub id: u16,
    pub x: u16,
    pub y: u16,
    pub up: Option<u16>,
    pub down: Option<u16>,
    pub left: Option<u16>,
    pub right: Option<u16>,
    pub normal_object: u16,
    pub selected_object: u16,
    pub activated_object: u16,
    pub commands: Vec<HdmvCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageDefinition {
    pub id: u8,
    pub version: u8,
    pub default_selected_button: u16,
    pub default_activated_button: u16,
    pub palette_id: u8,
    pub buttons: Vec<ButtonDefinition>,
    pub in_objects: Vec<CompositionObject>,
    pub window: Window,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractiveComposition {
    pub video_width: u16,
    pub video_height: u16,
    pub frame_rate_code: u8,
    pub composition_number: u16,
    pub composition_state: u8,
    pub user_timeout_duration: u32,
    pub pages: Vec<PageDefinition>,
}

pub fn palette_definition_segment(palette: &Palette, palette_id: u8, version: u8) -> Segment {
    let mut payload = Vec::with_capacity(2 + palette.entries.len() * 5);
    payload.push(palette_id);
    payload.push(version);
    for entry in &palette.entries {
        payload.push(entry.index);
        payload.push(entry.y);
        payload.push(entry.cr);
        payload.push(entry.cb);
        payload.push(entry.alpha);
    }
    Segment::new(SEGMENT_PDS, payload)
}

pub fn window_definition_segment(window: Window) -> Segment {
    let mut payload = Vec::with_capacity(10);
    payload.push(1);
    payload.push(window.id);
    payload.extend_from_slice(&window.x.to_be_bytes());
    payload.extend_from_slice(&window.y.to_be_bytes());
    payload.extend_from_slice(&window.width.to_be_bytes());
    payload.extend_from_slice(&window.height.to_be_bytes());
    Segment::new(SEGMENT_WDS, payload)
}

pub fn object_definition_segment(
    object_id: u16,
    width: u16,
    height: u16,
    version: u8,
    rle: &[u8],
) -> Segment {
    let data_len = u32::try_from(rle.len())
        .unwrap_or(u32::MAX)
        .saturating_add(4);
    let mut payload = Vec::with_capacity(11 + rle.len());
    payload.extend_from_slice(&object_id.to_be_bytes());
    payload.push(version);
    payload.push(0xC0); // first + last segment of the object
    payload.extend_from_slice(&data_len.to_be_bytes()[1..]);
    payload.extend_from_slice(&width.to_be_bytes());
    payload.extend_from_slice(&height.to_be_bytes());
    payload.extend_from_slice(rle);
    Segment::new(SEGMENT_ODS, payload)
}

pub fn interactive_composition_segment(
    composition: &InteractiveComposition,
) -> Result<Segment, SegmentError> {
    let mut interactive = Vec::new();
    // stream_model = 1 (non-streaming); ui_model = 0.
    interactive.push(0x80);
    push_u24(&mut interactive, composition.user_timeout_duration);
    let page_count =
        u8::try_from(composition.pages.len()).map_err(|_| SegmentError::FieldOverflow {
            field: "pages",
            value: composition.pages.len(),
        })?;
    interactive.push(page_count);
    for page in &composition.pages {
        push_page(&mut interactive, page)?;
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&composition.video_width.to_be_bytes());
    payload.extend_from_slice(&composition.video_height.to_be_bytes());
    payload.push((composition.frame_rate_code & 0x0F) << 4);
    payload.extend_from_slice(&composition.composition_number.to_be_bytes());
    payload.push((composition.composition_state & 0x03) << 6);
    payload.push(0xC0); // sequence descriptor: first + last
    let interactive_len =
        u32::try_from(interactive.len()).map_err(|_| SegmentError::FieldOverflow {
            field: "interactive_length",
            value: interactive.len(),
        })?;
    push_u24(&mut payload, interactive_len);
    payload.extend_from_slice(&interactive);
    Ok(Segment::new(SEGMENT_ICS, payload))
}

pub fn end_of_display_set_segment() -> Segment {
    Segment::new(SEGMENT_END, Vec::new())
}

fn push_page(out: &mut Vec<u8>, page: &PageDefinition) -> Result<(), SegmentError> {
    out.push(page.id);
    out.push(page.version);
    out.extend_from_slice(&[0u8; 8]); // user-operation mask: everything allowed
    push_effect_sequence(out, page.window, &page.in_objects, 1, page.palette_id)?;
    push_effect_sequence(out, page.window, &[], 0, page.palette_id)?;
    out.push(0); // animation frame rate code
    out.extend_from_slice(&page.default_selected_button.to_be_bytes());
    out.extend_from_slice(&page.default_activated_button.to_be_bytes());
    out.push(page.palette_id);
    // Each independent button gets its own button-overlap group (BOG).
    // libbluray tracks the enabled button per BOG; packing every button into
    // one BOG allows only a single cursor move before the navigation refs are
    // resolved from the wrong button. Independent BOGs also match how HDMV
    // authoring tools lay out non-overlapping buttons.
    let button_count =
        u8::try_from(page.buttons.len()).map_err(|_| SegmentError::FieldOverflow {
            field: "buttons",
            value: page.buttons.len(),
        })?;
    out.push(button_count);
    for button in &page.buttons {
        out.extend_from_slice(&button.id.to_be_bytes());
        out.push(1);
        push_button(out, button)?;
    }
    Ok(())
}

fn push_effect_sequence(
    out: &mut Vec<u8>,
    window: Window,
    objects: &[CompositionObject],
    effect_count: u8,
    palette_id: u8,
) -> Result<(), SegmentError> {
    if objects.is_empty() {
        out.push(0);
        out.push(0);
        return Ok(());
    }
    out.push(1);
    out.push(window.id);
    out.extend_from_slice(&window.x.to_be_bytes());
    out.extend_from_slice(&window.y.to_be_bytes());
    out.extend_from_slice(&window.width.to_be_bytes());
    out.extend_from_slice(&window.height.to_be_bytes());
    out.push(effect_count);
    if effect_count == 0 {
        return Ok(());
    }
    push_u24(out, 0); // infinite/still duration
    out.push(palette_id);
    let object_count = u8::try_from(objects.len()).map_err(|_| SegmentError::FieldOverflow {
        field: "objects",
        value: objects.len(),
    })?;
    out.push(object_count);
    for object in objects {
        out.extend_from_slice(&object.object_id.to_be_bytes());
        out.push(object.window_id);
        out.push(0); // crop flag + reserved
        out.extend_from_slice(&object.x.to_be_bytes());
        out.extend_from_slice(&object.y.to_be_bytes());
    }
    Ok(())
}

fn push_button(out: &mut Vec<u8>, button: &ButtonDefinition) -> Result<(), SegmentError> {
    out.extend_from_slice(&button.id.to_be_bytes());
    out.extend_from_slice(&button.id.to_be_bytes()); // numeric select value
    out.push(0); // auto-action flag + reserved
    out.extend_from_slice(&button.x.to_be_bytes());
    out.extend_from_slice(&button.y.to_be_bytes());
    // A missing neighbour points back at the button itself: that is how HDMV
    // encodes "this key does not move the selection".
    for target in [button.up, button.down, button.left, button.right] {
        out.extend_from_slice(&target.unwrap_or(button.id).to_be_bytes());
    }
    out.extend_from_slice(&button.normal_object.to_be_bytes());
    out.extend_from_slice(&button.normal_object.to_be_bytes());
    out.push(0); // normal repeat flag + reserved
    out.push(0xFF); // selected sound id
    out.extend_from_slice(&button.selected_object.to_be_bytes());
    out.extend_from_slice(&button.selected_object.to_be_bytes());
    out.push(0); // selected repeat flag + reserved
    out.push(0xFF); // activated sound id
    out.extend_from_slice(&button.activated_object.to_be_bytes());
    out.extend_from_slice(&button.activated_object.to_be_bytes());
    let command_count =
        u16::try_from(button.commands.len()).map_err(|_| SegmentError::FieldOverflow {
            field: "commands",
            value: button.commands.len(),
        })?;
    out.extend_from_slice(&command_count.to_be_bytes());
    for command in &button.commands {
        out.extend_from_slice(&command.to_bytes());
    }
    Ok(())
}

fn push_u24(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes()[1..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::jump_title;
    use crate::palette::Palette;

    fn palette() -> Palette {
        Palette::from_pixels([[255u8, 255, 255, 255].as_slice()]).expect("palette")
    }

    #[test]
    fn segment_headers_are_big_endian_and_end_is_empty() {
        let segment = Segment::new(SEGMENT_PDS, vec![1, 2, 3]);
        assert_eq!(
            segment.encode().expect("encode"),
            vec![SEGMENT_PDS, 0, 3, 1, 2, 3]
        );
        assert_eq!(
            end_of_display_set_segment().encode().expect("encode"),
            vec![SEGMENT_END, 0, 0]
        );
    }

    #[test]
    fn oversized_segment_payload_is_rejected() {
        let segment = Segment::new(SEGMENT_PDS, vec![0; 0x1_0000]);
        assert_eq!(
            segment.encode().expect_err("payload"),
            SegmentError::PayloadTooLarge {
                kind: SEGMENT_PDS,
                length: 0x1_0000,
            }
        );
    }

    #[test]
    fn pds_and_wds_have_the_canonical_layout() {
        let palette = palette();
        let pds = palette_definition_segment(&palette, 0, 1);
        assert_eq!(pds.kind, SEGMENT_PDS);
        assert_eq!(pds.payload[0], 0);
        assert_eq!(pds.payload[1], 1);
        assert_eq!(pds.payload[2], palette.entries[0].index);
        let wds = window_definition_segment(Window {
            id: 0,
            x: 10,
            y: 20,
            width: 1280,
            height: 720,
        });
        assert_eq!(wds.kind, SEGMENT_WDS);
        assert_eq!(&wds.payload[..], &[1, 0, 0, 10, 0, 20, 5, 0, 2, 208]);
    }

    #[test]
    fn ics_contains_page_button_and_navigation_data() {
        let composition = InteractiveComposition {
            video_width: 1280,
            video_height: 720,
            frame_rate_code: 1,
            composition_number: 2,
            composition_state: 0,
            user_timeout_duration: 0,
            pages: vec![PageDefinition {
                id: 0,
                version: 0,
                default_selected_button: 1,
                default_activated_button: NO_BUTTON,
                palette_id: 0,
                buttons: vec![ButtonDefinition {
                    id: 1,
                    x: 100,
                    y: 200,
                    up: None,
                    down: None,
                    left: None,
                    right: None,
                    normal_object: 1,
                    selected_object: 2,
                    activated_object: 3,
                    commands: vec![jump_title(1)],
                }],
                in_objects: vec![CompositionObject {
                    object_id: 1,
                    window_id: 0,
                    x: 100,
                    y: 200,
                }],
                window: Window {
                    id: 0,
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720,
                },
            }],
        };
        let segment = interactive_composition_segment(&composition).expect("ics");
        assert_eq!(segment.kind, SEGMENT_ICS);
        assert_eq!(&segment.payload[..4], &[5, 0, 2, 208]);
        assert_eq!(segment.payload[4] >> 4, 1);
        assert!(segment
            .payload
            .windows(12)
            .any(|window| window == jump_title(1).to_bytes()));
        // The page advertises one BOG, whose default valid button is 1 and
        // whose button list contains exactly that button.
        let tail = [
            0x00, 0x01, // default selected
            0xFF, 0xFF, // default activated = none
            0x00, // palette id
            0x01, // BOG count
            0x00, 0x01, // default valid button
            0x01, // buttons in this BOG
            0x00, 0x01, // button id
        ];
        assert!(segment
            .payload
            .windows(tail.len())
            .any(|window| window == tail));
    }

    #[test]
    fn ics_supports_multiple_pages_and_independent_bogs() {
        let button = ButtonDefinition {
            id: 1,
            x: 10,
            y: 10,
            up: None,
            down: None,
            left: None,
            right: Some(2),
            normal_object: 1,
            selected_object: 2,
            activated_object: 3,
            commands: vec![jump_title(1)],
        };
        let page = PageDefinition {
            id: 0,
            version: 0,
            default_selected_button: 1,
            default_activated_button: NO_BUTTON,
            palette_id: 0,
            buttons: vec![button.clone()],
            in_objects: Vec::new(),
            window: Window {
                id: 0,
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
        };
        let mut second = page.clone();
        second.id = 1;
        second.buttons = vec![ButtonDefinition { id: 2, ..button }];
        let composition = InteractiveComposition {
            video_width: 1280,
            video_height: 720,
            frame_rate_code: 1,
            composition_number: 0,
            composition_state: 2,
            user_timeout_duration: 0,
            pages: vec![page, second],
        };
        let payload = interactive_composition_segment(&composition)
            .expect("ics")
            .payload;
        // page count follows stream_model (1) + timeout (3).
        assert_eq!(payload[16], 2);
    }

    #[test]
    fn ods_length_includes_width_height_and_rle() {
        let segment = object_definition_segment(7, 2, 3, 0, &[0x11, 0x22]);
        assert_eq!(segment.kind, SEGMENT_ODS);
        assert_eq!(&segment.payload[0..2], &[0, 7]);
        assert_eq!(&segment.payload[4..7], &[0, 0, 6]);
        assert_eq!(&segment.payload[7..11], &[0, 2, 0, 3]);
    }

    fn composition_with_page(page: PageDefinition) -> InteractiveComposition {
        InteractiveComposition {
            video_width: 1280,
            video_height: 720,
            frame_rate_code: 1,
            composition_number: 0,
            composition_state: 2,
            user_timeout_duration: 0,
            pages: vec![page],
        }
    }

    fn page() -> PageDefinition {
        PageDefinition {
            id: 0,
            version: 0,
            default_selected_button: 1,
            default_activated_button: NO_BUTTON,
            palette_id: 0,
            buttons: Vec::new(),
            in_objects: Vec::new(),
            window: Window {
                id: 0,
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
        }
    }

    fn button() -> ButtonDefinition {
        ButtonDefinition {
            id: 1,
            x: 0,
            y: 0,
            up: None,
            down: None,
            left: None,
            right: None,
            normal_object: 1,
            selected_object: 1,
            activated_object: 1,
            commands: Vec::new(),
        }
    }

    #[test]
    fn rejects_page_count_overflow() {
        let mut composition = composition_with_page(page());
        composition.pages = vec![page(); 256];
        assert_eq!(
            interactive_composition_segment(&composition).expect_err("pages"),
            SegmentError::FieldOverflow {
                field: "pages",
                value: 256,
            }
        );
    }

    #[test]
    fn rejects_button_and_object_count_overflow() {
        let mut page_with_buttons = page();
        page_with_buttons.buttons = vec![button(); 256];
        let error = interactive_composition_segment(&composition_with_page(page_with_buttons))
            .expect_err("buttons");
        assert_eq!(
            error,
            SegmentError::FieldOverflow {
                field: "buttons",
                value: 256,
            }
        );

        let mut page_with_objects = page();
        page_with_objects.in_objects = vec![
            CompositionObject {
                object_id: 1,
                window_id: 0,
                x: 0,
                y: 0,
            };
            256
        ];
        let error = interactive_composition_segment(&composition_with_page(page_with_objects))
            .expect_err("objects");
        assert_eq!(
            error,
            SegmentError::FieldOverflow {
                field: "objects",
                value: 256,
            }
        );
    }

    #[test]
    fn rejects_command_count_overflow() {
        let mut overflowing = button();
        overflowing.commands = vec![jump_title(1); 0x1_0000];
        let mut page_with_button = page();
        page_with_button.buttons = vec![overflowing];
        let error = interactive_composition_segment(&composition_with_page(page_with_button))
            .expect_err("commands");
        assert_eq!(
            error,
            SegmentError::FieldOverflow {
                field: "commands",
                value: 0x1_0000,
            }
        );
    }
}
