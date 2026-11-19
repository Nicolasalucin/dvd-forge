//! Independent structural checks for authored menu streams.

use crate::report::CheckList;
use crate::{be16, paths_with_prefix, read_prefix, warn_with};
use libmkiso::UdfReader;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
struct Button {
    id: u16,
    x: u16,
    y: u16,
    up: u16,
    down: u16,
    left: u16,
    right: u16,
    object: u16,
}

pub(crate) fn check_bdmv_ig(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let streams = paths_with_prefix(reader, "BDMV/STREAM/");
    let mut checked = 0usize;
    let mut buttons = 0usize;
    for path in streams.iter().filter(|path| path.ends_with(".m2ts")) {
        let Ok(prefix) = read_prefix(reader, path, 1024 * 1024) else {
            continue;
        };
        let Some(pid) = ig_pid(&prefix) else {
            continue;
        };
        let segments = pes_segments(&prefix, pid);
        let Some(ics) = segments.iter().find(|(kind, _)| *kind == 0x18) else {
            continue;
        };
        let ods: HashMap<u16, (u16, u16)> = segments
            .iter()
            .filter(|(kind, _)| *kind == 0x15)
            .filter_map(|(_, payload)| {
                if payload.len() < 11 {
                    return None;
                }
                let id = be16(payload, 0)?;
                let width = be16(payload, 7)?;
                let height = be16(payload, 9)?;
                Some((id, (width, height)))
            })
            .collect();
        let Some((width, height, parsed)) = parse_ics(&ics.1) else {
            warn_with(
                checks,
                "BDMV-IG-BUTTONS",
                "ICS presente pero no se pudo parsear la tabla de botones.".to_string(),
                path,
                None,
            );
            continue;
        };
        checked += 1;
        buttons += parsed.len();
        let mut valid = true;
        let mut rects = Vec::new();
        for button in &parsed {
            let (bw, bh) = ods.get(&button.object).copied().unwrap_or((1, 1));
            if button.x.saturating_add(bw) > width || button.y.saturating_add(bh) > height {
                valid = false;
            }
            rects.push((button.x, button.y, bw, bh));
        }
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let a = rects[i];
                let b = rects[j];
                if a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3 {
                    valid = false;
                }
            }
        }
        let ids: Vec<u16> = parsed.iter().map(|button| button.id).collect();
        for button in &parsed {
            for neighbor in [button.up, button.down, button.left, button.right] {
                if neighbor != button.id && !ids.contains(&neighbor) {
                    valid = false;
                }
            }
        }
        if valid {
            checks.pass(
                "BDMV-IG-BUTTONS",
                format!(
                    "{} botones IG con geometría y vecinos válidos.",
                    parsed.len()
                ),
            );
            checks.pass(
                "BDMV-IG-NAV",
                "La tabla de navegación IG referencia botones existentes.".to_string(),
            );
        } else {
            warn_with(
                checks,
                "BDMV-IG-BUTTONS",
                "La tabla de botones IG contiene geometría solapada o referencias inválidas."
                    .to_string(),
                path,
                None,
            );
            warn_with(
                checks,
                "BDMV-IG-NAV",
                "La tabla de navegación IG contiene vecinos inválidos.".to_string(),
                path,
                None,
            );
        }
    }
    if checked > 0 {
        checks.pass(
            "BDMV-MOTION-THUMB",
            format!(
                "{checked} menús IG con clip de vídeo e interactive graphics ({buttons} botones)."
            ),
        );
    }
}

pub(crate) fn check_dvd_spu_buttons(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let vobs = paths_with_prefix(reader, "VIDEO_TS/")
        .into_iter()
        .filter(|path| path.ends_with(".VOB"))
        .collect::<Vec<_>>();
    for vob in vobs {
        let Ok(prefix) = read_prefix(reader, &vob, 1024 * 1024) else {
            continue;
        };
        let mut cursor = 0usize;
        while cursor + 13 < prefix.len() {
            if prefix[cursor..cursor + 4] != [0x00, 0x00, 0x01, 0xBD] {
                cursor += 1;
                continue;
            }
            let packet_length =
                u16::from_be_bytes([prefix[cursor + 4], prefix[cursor + 5]]) as usize;
            let header_length = prefix[cursor + 7] as usize;
            let data = cursor + 8 + header_length;
            if data + 5 > prefix.len() {
                break;
            }
            if (0x20..=0x3F).contains(&prefix[data]) {
                let spu = data + 1;
                let spu_length = u16::from_be_bytes([prefix[spu], prefix[spu + 1]]) as usize;
                let control = u16::from_be_bytes([prefix[spu + 2], prefix[spu + 3]]) as usize;
                if spu_length > control && control >= 6 && spu + control + 6 <= prefix.len() {
                    checks.pass(
                        "DVD-SPU-BUTTONS",
                        "Secuencia de control SPU y tabla de botones alcanzables.".to_string(),
                    );
                    return;
                }
            }
            cursor += 4 + packet_length.max(3);
        }
    }
}

fn ig_pid(prefix: &[u8]) -> Option<u16> {
    for offset in (0..prefix.len().saturating_sub(191)).step_by(192) {
        let packet = &prefix[offset + 4..offset + 192];
        if packet.len() < 188 || packet[0] != 0x47 || packet[1] & 0x40 == 0 {
            continue;
        }
        let adaptation = (packet[3] >> 4) & 0x03;
        let mut payload = 4;
        if adaptation == 3 {
            payload += 1 + packet[4] as usize;
        } else if adaptation == 2 {
            continue;
        }
        if payload >= packet.len() {
            continue;
        }
        let section = payload + 1 + packet[payload] as usize;
        if section + 12 > packet.len() || packet[section] != 0x02 {
            continue;
        }
        let end = (section
            + 3
            + ((((packet[section + 1] & 0x0f) as usize) << 8) | packet[section + 2] as usize))
            .min(packet.len());
        let program_info =
            (((packet[section + 10] & 0x0f) as usize) << 8) | packet[section + 11] as usize;
        let mut cursor = section + 12 + program_info;
        while cursor + 5 <= end {
            let stream_type = packet[cursor];
            let pid = (((packet[cursor + 1] & 0x1f) as u16) << 8) | packet[cursor + 2] as u16;
            if stream_type == 0x91 {
                return Some(pid);
            }
            let info_len =
                (((packet[cursor + 3] & 0x0f) as usize) << 8) | packet[cursor + 4] as usize;
            cursor += 5 + info_len;
        }
        return None;
    }
    None
}

fn pes_segments(prefix: &[u8], pid: u16) -> Vec<(u8, Vec<u8>)> {
    let mut current = Vec::new();
    let mut segments = Vec::new();
    let mut push = |data: &[u8]| {
        let mut cursor = 0usize;
        while cursor + 3 <= data.len() {
            let kind = data[cursor];
            let length = u16::from_be_bytes([data[cursor + 1], data[cursor + 2]]) as usize;
            if cursor + 3 + length > data.len() {
                break;
            }
            segments.push((kind, data[cursor + 3..cursor + 3 + length].to_vec()));
            cursor += 3 + length;
        }
    };
    for offset in (0..prefix.len().saturating_sub(191)).step_by(192) {
        let packet = &prefix[offset + 4..offset + 192];
        if packet.len() < 188 || packet[0] != 0x47 {
            continue;
        }
        let packet_pid = (((packet[1] & 0x1f) as u16) << 8) | packet[2] as u16;
        if packet_pid != pid {
            continue;
        }
        let pusi = packet[1] & 0x40 != 0;
        let adaptation = (packet[3] >> 4) & 0x03;
        let mut start = 4;
        if adaptation == 3 {
            start += 1 + packet[4] as usize;
        } else if adaptation == 2 {
            continue;
        }
        if start >= packet.len() {
            continue;
        }
        if pusi {
            if !current.is_empty() {
                push(&current);
            }
            let payload = &packet[start..];
            if payload.len() < 9 || payload[..3] != [0, 0, 1] {
                current.clear();
                continue;
            }
            let data = 9 + payload[8] as usize;
            current = if data <= payload.len() {
                payload[data..].to_vec()
            } else {
                Vec::new()
            };
        } else {
            current.extend_from_slice(&packet[start..]);
        }
    }
    if !current.is_empty() {
        push(&current);
    }
    segments
}

/// Interactive Composition Segment (HDMV IG) after the segment header:
/// video descriptor (5), composition descriptor (3), sequence descriptor (1),
/// data length (3), then the interactive composition: model flags (1), the
/// two 5-byte timeouts only for the multiplexed stream model, user timeout
/// (3), pages. Each page lists button overlap groups (BOGs), each with its
/// buttons. QA 1.1.3: the old reader started one byte late and skipped the
/// BOG layer, so it never saw the buttons ("0 IG buttons").
fn parse_ics(payload: &[u8]) -> Option<(u16, u16, Vec<Button>)> {
    let width = be16(payload, 0)?;
    let height = be16(payload, 2)?;
    let flags = *payload.get(12)?;
    let mut pos = 13usize;
    if flags & 0x80 == 0 {
        pos += 10; // composition + selection timeout PTS (multiplexed model)
    }
    pos += 3; // user timeout duration
    let pages = *payload.get(pos)? as usize;
    pos += 1;
    let mut buttons = Vec::new();
    for _ in 0..pages {
        pos += 10; // page id, version, user operation mask
        pos = skip_effect(payload, pos)?;
        pos = skip_effect(payload, pos)?;
        pos += 1 + 2 + 2 + 1; // frame rate, default selected/activated, palette
        let groups = *payload.get(pos)? as usize;
        pos += 1;
        for _ in 0..groups {
            pos += 2; // default valid button
            let count = *payload.get(pos)? as usize;
            pos += 1;
            for _ in 0..count {
                if pos + 35 > payload.len() {
                    return None;
                }
                let id = be16(payload, pos)?;
                let x = be16(payload, pos + 5)?;
                let y = be16(payload, pos + 7)?;
                let up = be16(payload, pos + 9)?;
                let down = be16(payload, pos + 11)?;
                let left = be16(payload, pos + 13)?;
                let right = be16(payload, pos + 15)?;
                let object = be16(payload, pos + 17)?;
                let commands = be16(payload, pos + 33)? as usize;
                pos += 35 + commands * 12;
                if pos > payload.len() {
                    return None;
                }
                buttons.push(Button {
                    id,
                    x,
                    y,
                    up,
                    down,
                    left,
                    right,
                    object,
                });
            }
        }
    }
    Some((width, height, buttons))
}

/// Effect sequence: windows (id + x/y/w/h, 9 bytes each), then effects, each
/// with duration (3), palette (1) and composition objects (8 bytes, 16 when
/// cropped).
fn skip_effect(data: &[u8], mut pos: usize) -> Option<usize> {
    let windows = *data.get(pos)? as usize;
    pos += 1 + windows * 9;
    let effects = *data.get(pos)? as usize;
    pos += 1;
    for _ in 0..effects {
        pos += 4; // duration + palette id
        let objects = *data.get(pos)? as usize;
        pos += 1;
        for _ in 0..objects {
            let cropped = *data.get(pos + 3)? & 0x80 != 0;
            pos += if cropped { 16 } else { 8 };
        }
    }
    (pos <= data.len()).then_some(pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kinodisc_hdmv::segments::{
        interactive_composition_segment, ButtonDefinition, CompositionObject,
        InteractiveComposition, PageDefinition, Window,
    };

    fn button(id: u16, x: u16, right: Option<u16>) -> ButtonDefinition {
        ButtonDefinition {
            id,
            x,
            y: 100,
            up: None,
            down: None,
            left: None,
            right,
            normal_object: id * 3,
            selected_object: id * 3 + 1,
            activated_object: id * 3 + 2,
            commands: Vec::new(),
        }
    }

    #[test]
    fn reads_every_button_of_a_written_ics() {
        let window = Window {
            id: 0,
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let page = PageDefinition {
            id: 0,
            version: 0,
            default_selected_button: 1,
            default_activated_button: 0xFFFF,
            palette_id: 0,
            buttons: vec![button(1, 100, Some(2)), button(2, 600, None)],
            // An in-effect with an object, like the real menus.
            in_objects: vec![CompositionObject {
                object_id: 9,
                window_id: 0,
                x: 0,
                y: 0,
            }],
            window,
        };
        let composition = InteractiveComposition {
            video_width: 1920,
            video_height: 1080,
            frame_rate_code: 1,
            composition_number: 0,
            composition_state: 2,
            user_timeout_duration: 0,
            pages: vec![page],
        };
        let segment = interactive_composition_segment(&composition).expect("ics");
        let (width, height, buttons) = parse_ics(&segment.payload).expect("parsed");
        assert_eq!((width, height), (1920, 1080));
        // QA 1.1.3: "0 IG buttons" on discs that had buttons.
        assert_eq!(buttons.len(), 2);
        assert_eq!((buttons[0].id, buttons[0].x, buttons[0].right), (1, 100, 2));
        assert_eq!((buttons[1].id, buttons[1].x, buttons[1].right), (2, 600, 2));
        assert_eq!(buttons[1].object, 6);
    }
}
