use crate::report::CheckList;
use crate::{be16, be32, fail_with, paths_with_prefix, read_file, read_prefix, warn_with};
use libmkiso::UdfReader;

pub(crate) fn verify(reader: &UdfReader<'_>, checks: &mut CheckList) {
    check_index(reader, checks);
    check_movie_object(reader, checks);
    check_playlists(reader, checks);
    check_clips(reader, checks);
    check_streams(reader, checks);
    check_pgs_display_sets(reader, checks);
}

fn check_index(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let path = "BDMV/index.bdmv";
    let bytes = match read_file(reader, path, 4 * 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) => {
            fail_with(checks, "BDMV-INDEX", error, path, None);
            return;
        }
    };
    if bytes.len() < 40 || (&bytes[..8] != b"INDX0200" && &bytes[..8] != b"INDX0100") {
        fail_with(
            checks,
            "BDMV-INDEX",
            "Firma INDX0100/0200 ausente o truncada.".to_string(),
            path,
            None,
        );
        return;
    }
    let index_start = be32(&bytes, 8).unwrap_or(u32::MAX) as usize;
    if index_start + 4 > bytes.len() {
        fail_with(
            checks,
            "BDMV-INDEX",
            "index_start fuera del archivo.".to_string(),
            path,
            Some(index_start as u64),
        );
        return;
    }
    let index_len = be32(&bytes, index_start).unwrap_or(0) as usize;
    if index_len < 26 || index_start + 4 + index_len > bytes.len() {
        fail_with(
            checks,
            "BDMV-INDEX",
            "index_len incoherente.".to_string(),
            path,
            Some(index_start as u64),
        );
        return;
    }
    checks.pass(
        "BDMV-INDEX",
        "index.bdmv válido y tabla de títulos dentro de rango.",
    );
}

fn check_movie_object(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let path = "BDMV/MovieObject.bdmv";
    let bytes = match read_file(reader, path, 16 * 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) => {
            fail_with(checks, "BDMV-MOBJ", error, path, None);
            return;
        }
    };
    if bytes.len() < 50 || (&bytes[..8] != b"MOBJ0200" && &bytes[..8] != b"MOBJ0100") {
        fail_with(
            checks,
            "BDMV-MOBJ",
            "Firma MOBJ0100/0200 ausente o truncada.".to_string(),
            path,
            None,
        );
        return;
    }
    let data_len = be32(&bytes, 40).unwrap_or(u32::MAX) as usize;
    let num_objects = be16(&bytes, 48).unwrap_or(u16::MAX) as usize;
    if 44 + data_len != bytes.len() {
        fail_with(
            checks,
            "BDMV-MOBJ",
            format!("data_len={data_len} no coincide con el tamaño del archivo."),
            path,
            Some(40),
        );
        return;
    }
    let data_end = 44 + data_len;
    // tsMuxeR writes a 6-byte pad between data_len and the first object.
    let mut cursor = 50;
    for object in 0..num_objects {
        if cursor + 4 > bytes.len() {
            fail_with(
                checks,
                "BDMV-MOBJ",
                format!("El objeto {object} está truncado."),
                path,
                Some(cursor as u64),
            );
            return;
        }
        let commands = be16(&bytes, cursor + 2).unwrap_or(u16::MAX) as usize;
        let end = cursor + 4 + commands * 12;
        if end > bytes.len() {
            fail_with(
                checks,
                "BDMV-MOBJ",
                format!("El objeto {object} declara {commands} comandos fuera de rango."),
                path,
                Some(cursor as u64),
            );
            return;
        }
        cursor = end;
    }
    if cursor > data_end {
        fail_with(
            checks,
            "BDMV-MOBJ",
            format!("Los MovieObjects exceden data_len ({data_len})."),
            path,
            Some(cursor as u64),
        );
        return;
    }
    checks.pass(
        "BDMV-MOBJ",
        format!("{num_objects} MovieObjects, comandos de 12 bytes dentro de rango."),
    );
}

fn check_playlists(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let playlists = paths_with_prefix(reader, "BDMV/PLAYLIST/");
    if playlists.is_empty() {
        fail_with(
            checks,
            "BDMV-MPLS",
            "No hay playlists en BDMV/PLAYLIST.".to_string(),
            "BDMV/PLAYLIST",
            None,
        );
        return;
    }
    let mut playitems = 0usize;
    for path in &playlists {
        if !path.ends_with(".mpls") {
            continue;
        }
        let Ok(bytes) = read_file(reader, path, 16 * 1024 * 1024) else {
            fail_with(
                checks,
                "BDMV-MPLS",
                format!("No se pudo leer {path}."),
                path,
                None,
            );
            continue;
        };
        if bytes.len() < 40 || (&bytes[..8] != b"MPLS0200" && &bytes[..8] != b"MPLS0100") {
            fail_with(
                checks,
                "BDMV-MPLS",
                "Firma MPLS0100/0200 inválida.".to_string(),
                path,
                None,
            );
            continue;
        }
        let list_pos = be32(&bytes, 8).unwrap_or(u32::MAX) as usize;
        if list_pos + 10 > bytes.len() {
            fail_with(
                checks,
                "BDMV-MPLS",
                "list_pos fuera de rango.".to_string(),
                path,
                Some(list_pos as u64),
            );
            continue;
        }
        let list_len = be32(&bytes, list_pos).unwrap_or(u32::MAX) as usize;
        if list_pos + 4 + list_len > bytes.len() {
            fail_with(
                checks,
                "BDMV-MPLS",
                "Playlist fuera de rango.".to_string(),
                path,
                Some(list_pos as u64),
            );
            continue;
        }
        let count = be16(&bytes, list_pos + 6).unwrap_or(u16::MAX) as usize;
        let mut cursor = list_pos + 10;
        let mut valid = true;
        for _ in 0..count {
            let Some(length) = be16(&bytes, cursor) else {
                valid = false;
                break;
            };
            let end = cursor + 2 + length as usize;
            if length < 7 || end > bytes.len() {
                valid = false;
                break;
            }
            let clip = String::from_utf8_lossy(&bytes[cursor + 2..cursor + 7]).to_string();
            let clip_path = format!("BDMV/CLIPINF/{clip}.clpi");
            if !crate::has(reader, &clip_path) {
                fail_with(
                    checks,
                    "BDMV-MPLS",
                    format!("La playlist referencia el clip inexistente {clip}."),
                    path,
                    Some(cursor as u64),
                );
                valid = false;
            }
            let in_time = be32(&bytes, cursor + 2 + 11).unwrap_or(0);
            let out_time = be32(&bytes, cursor + 2 + 15).unwrap_or(0);
            if out_time <= in_time {
                fail_with(
                    checks,
                    "BDMV-MPLS",
                    format!("Playitem con OUT_time ({out_time}) <= IN_time ({in_time})."),
                    path,
                    Some(cursor as u64),
                );
                valid = false;
            }
            playitems += 1;
            cursor = end;
        }
        if valid {
            checks.pass(
                "BDMV-MPLS",
                format!("{path}: {count} playitems con CLPI existente."),
            );
        }
    }
    if playitems == 0 {
        fail_with(
            checks,
            "BDMV-MPLS",
            "Las playlists no contienen playitems.".to_string(),
            "BDMV/PLAYLIST",
            None,
        );
    }
}

fn check_clips(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let clips = paths_with_prefix(reader, "BDMV/CLIPINF/");
    let mut found = 0usize;
    for path in &clips {
        if !path.ends_with(".clpi") {
            continue;
        }
        found += 1;
        match read_file(reader, path, 16 * 1024 * 1024) {
            Ok(bytes)
                if bytes.len() >= 8
                    && (&bytes[..8] == b"HDMV0200" || &bytes[..8] == b"HDMV0100") => {}
            Ok(_) => fail_with(
                checks,
                "BDMV-CLPI",
                "Firma HDMV0100/0200 inválida.".to_string(),
                path,
                None,
            ),
            Err(error) => fail_with(checks, "BDMV-CLPI", error, path, None),
        }
    }
    if found == 0 {
        fail_with(
            checks,
            "BDMV-CLPI",
            "No hay archivos CLPI.".to_string(),
            "BDMV/CLIPINF",
            None,
        );
    } else {
        checks.pass("BDMV-CLPI", format!("{found} CLPI con firma HDMV válida."));
    }
}

fn check_streams(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let streams = paths_with_prefix(reader, "BDMV/STREAM/");
    let mut checked = 0usize;
    let mut pmt_streams = Vec::new();
    for path in &streams {
        if !path.ends_with(".m2ts") {
            continue;
        }
        checked += 1;
        let prefix = match read_prefix(reader, path, 192 * 64) {
            Ok(prefix) => prefix,
            Err(error) => {
                fail_with(checks, "BDMV-M2TS", error, path, None);
                continue;
            }
        };
        if prefix.len() < 192 || prefix[4] != 0x47 {
            fail_with(
                checks,
                "BDMV-M2TS",
                "El stream no empieza con un paquete M2TS (sync 0x47 en offset 4).".to_string(),
                path,
                Some(4),
            );
            continue;
        }
        let mut sync_ok = true;
        for offset in (0..prefix.len().saturating_sub(191)).step_by(192) {
            if prefix[offset + 4] != 0x47 {
                sync_ok = false;
                break;
            }
        }
        if !sync_ok {
            fail_with(
                checks,
                "BDMV-M2TS",
                "Se perdió la sincronía de paquetes de 192 bytes.".to_string(),
                path,
                None,
            );
            continue;
        }
        pmt_streams.extend(parse_pmt_streams(&prefix));
    }
    if checked == 0 {
        fail_with(
            checks,
            "BDMV-M2TS",
            "No hay streams M2TS.".to_string(),
            "BDMV/STREAM",
            None,
        );
        return;
    }
    checks.pass(
        "BDMV-M2TS",
        format!("{checked} streams con sync 0x47 cada 192 bytes."),
    );

    let has_video = pmt_streams
        .iter()
        .any(|stream| matches!(stream, 0x02 | 0x1b | 0x24));
    let has_audio = pmt_streams
        .iter()
        .any(|stream| matches!(stream, 0x03 | 0x04 | 0x0f | 0x81 | 0x82 | 0x86));
    if has_video && has_audio {
        checks.pass("BDMV-PID", "PAT/PMT declaran video y audio.");
    } else {
        warn_with(
            checks,
            "BDMV-PID",
            format!(
                "No se pudieron confirmar video+audio en el PMT (stream types: {pmt_streams:?})."
            ),
            "BDMV/STREAM",
            None,
        );
    }

    if has_audio {
        checks.pass(
            "BDMV-AUDIO-STREAM",
            "El PMT declara al menos un stream de audio (AC-3/DTS/LPCM/DTS-HD).".to_string(),
        );
    } else {
        warn_with(
            checks,
            "BDMV-AUDIO-STREAM",
            "No se encontró un stream de audio en el PMT inspeccionado.".to_string(),
            "BDMV/STREAM",
            None,
        );
    }
    let has_ig = pmt_streams.contains(&0x91);
    let has_pgs = pmt_streams.contains(&0x90);
    if has_ig {
        checks.pass(
            "BDMV-IG",
            "El PMT declara un stream Interactive Graphics (0x91).",
        );
    } else if has_pgs {
        warn_with(
            checks,
            "BDMV-IG",
            "El PMT solo declara PGS (0x90); no se encontró IG (0x91).".to_string(),
            "BDMV/STREAM",
            None,
        );
    } else {
        warn_with(
            checks,
            "BDMV-IG",
            "No se encontró un stream IG en los PMT inspeccionados.".to_string(),
            "BDMV/STREAM",
            None,
        );
    }
    if has_pgs || has_ig {
        checks.pass(
            "BDMV-PGS",
            "El PMT declara un stream de presentation graphics.",
        );
    } else {
        warn_with(
            checks,
            "BDMV-PGS",
            "No se encontró un stream PGS/IG en los PMT inspeccionados.".to_string(),
            "BDMV/STREAM",
            None,
        );
    }
}

#[derive(Debug, Clone, Copy)]
struct PgsSegment {
    kind: u8,
    pts: u32,
}

fn check_pgs_display_sets(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let streams = paths_with_prefix(reader, "BDMV/STREAM/");
    let mut inspected = 0usize;
    let mut display_sets = 0usize;
    let mut chronological = true;
    let mut last_pts = 0u32;
    for path in streams.iter().filter(|path| path.ends_with(".m2ts")) {
        let Ok(prefix) = read_prefix(reader, path, 1024 * 1024) else {
            continue;
        };
        let Some(pid) = pgs_pid(&prefix) else {
            continue;
        };
        let segments = pgs_segments(&prefix, pid);
        if segments.is_empty() {
            continue;
        }
        inspected += 1;
        let mut has_pcs = false;
        let mut has_object = false;
        let mut has_end = false;
        for segment in &segments {
            if segment.kind == 0x16 {
                if has_pcs && has_end {
                    display_sets += 1;
                }
                has_pcs = true;
                has_object = false;
                has_end = false;
            } else if segment.kind == 0x15 {
                has_object = true;
            } else if segment.kind == 0x80 {
                has_end = true;
                if has_pcs && has_object {
                    display_sets += 1;
                    has_pcs = false;
                    has_object = false;
                }
            }
            if segment.pts < last_pts {
                chronological = false;
            }
            last_pts = segment.pts;
        }
        if has_pcs && has_end && has_object {
            display_sets += 1;
        }
    }
    if inspected == 0 {
        checks.skip(
            "BDMV-PGS-DS",
            "Sin datos PGS en el primer MiB; no se puede validar el display set.".to_string(),
        );
        return;
    }
    if display_sets > 0 {
        checks.pass(
            "BDMV-PGS-DS",
            format!("{display_sets} display sets PGS con PCS/ODS/END en {inspected} streams."),
        );
    } else {
        checks.skip(
            "BDMV-PGS-DS",
            "El PID PGS no contiene display sets completos en el prefijo inspeccionado."
                .to_string(),
        );
        return;
    }
    if chronological {
        checks.pass(
            "BDMV-PGS-TS",
            "Los PTS/DTS de los segmentos PGS son monotónicos.".to_string(),
        );
    } else {
        warn_with(
            checks,
            "BDMV-PGS-TS",
            "Los PTS/DTS de los segmentos PGS retroceden.".to_string(),
            "BDMV/STREAM",
            None,
        );
    }
}

fn pgs_pid(prefix: &[u8]) -> Option<u16> {
    for offset in (0..prefix.len().saturating_sub(191)).step_by(192) {
        let packet = &prefix[offset + 4..offset + 192];
        if packet.len() < 188 || packet[0] != 0x47 {
            continue;
        }
        let pusi = packet[1] & 0x40 != 0;
        if !pusi {
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
        let pointer = packet[payload] as usize;
        let section = payload + 1 + pointer;
        if section + 12 > packet.len() || packet[section] != 0x02 {
            continue;
        }
        let program_info =
            (((packet[section + 10] & 0x0f) as usize) << 8) | packet[section + 11] as usize;
        let mut cursor = section + 12 + program_info;
        let end = (section
            + 3
            + ((((packet[section + 1] & 0x0f) as usize) << 8) | packet[section + 2] as usize))
            .min(packet.len());
        while cursor + 5 <= end {
            let stream_type = packet[cursor];
            let pid = (((packet[cursor + 1] & 0x1f) as u16) << 8) | packet[cursor + 2] as u16;
            if stream_type == 0x90 {
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

fn pgs_segments(prefix: &[u8], pid: u16) -> Vec<PgsSegment> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut pts = 0u32;
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
                append_segments(&current, pts, &mut segments);
            }
            let payload = &packet[start..];
            if payload.len() < 9 || payload[..3] != [0, 0, 1] {
                current.clear();
                continue;
            }
            pts = decode_pts(payload);
            let header_len = payload[8] as usize;
            let data = 9 + header_len;
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
        append_segments(&current, pts, &mut segments);
    }
    segments
}

fn decode_pts(pes: &[u8]) -> u32 {
    if pes.len() < 14 || pes[7] & 0x80 == 0 {
        return 0;
    }
    ((u32::from(pes[9] >> 1) & 0x07) << 30)
        | (u32::from(pes[10]) << 22)
        | ((u32::from(pes[11]) >> 1) << 15)
        | (u32::from(pes[12]) << 7)
        | (u32::from(pes[13]) >> 1)
}

fn append_segments(data: &[u8], pts: u32, segments: &mut Vec<PgsSegment>) {
    let mut cursor = 0usize;
    while cursor + 3 <= data.len() {
        let kind = data[cursor];
        let length = u16::from_be_bytes([data[cursor + 1], data[cursor + 2]]) as usize;
        if cursor + 3 + length > data.len() {
            break;
        }
        segments.push(PgsSegment { kind, pts });
        cursor += 3 + length;
    }
}

fn parse_pmt_streams(prefix: &[u8]) -> Vec<u8> {
    let mut pmt_pids = Vec::new();
    let mut streams = Vec::new();
    for offset in (0..prefix.len().saturating_sub(191)).step_by(192) {
        let packet = &prefix[offset + 4..offset + 192];
        if packet.len() < 188 || packet[0] != 0x47 {
            continue;
        }
        let _pid = ((packet[1] as u16 & 0x1f) << 8) | packet[2] as u16;
        let pusi = packet[1] & 0x40 != 0;
        let adaptation = (packet[3] >> 4) & 0x03;
        let mut payload = 4;
        if adaptation == 3 {
            payload += 1 + packet[4] as usize;
        } else if adaptation == 2 {
            continue;
        }
        if payload >= packet.len() || !pusi {
            continue;
        }
        let pointer = packet[payload] as usize;
        let section = payload + 1 + pointer;
        if section + 8 > packet.len() {
            continue;
        }
        let table_id = packet[section];
        let section_length =
            (((packet[section + 1] & 0x0f) as usize) << 8) | packet[section + 2] as usize;
        let end = (section + 3 + section_length).min(packet.len());
        if table_id == 0x00 {
            let mut cursor = section + 8;
            while cursor + 4 <= end {
                let program = u16::from_be_bytes([packet[cursor], packet[cursor + 1]]);
                let pmt = (((packet[cursor + 2] & 0x1f) as u16) << 8) | packet[cursor + 3] as u16;
                if program != 0 {
                    pmt_pids.push(pmt);
                }
                cursor += 4;
            }
        } else if table_id == 0x02 {
            let program_info =
                (((packet[section + 10] & 0x0f) as usize) << 8) | packet[section + 11] as usize;
            let mut cursor = section + 12 + program_info;
            while cursor + 5 <= end {
                streams.push(packet[cursor]);
                let info_len =
                    (((packet[cursor + 3] & 0x0f) as usize) << 8) | packet[cursor + 4] as usize;
                cursor += 5 + info_len;
            }
        }
    }
    // A PMT PID is only useful as a signal that PAT parsing succeeded; PIDs are
    // not repeated in the PMT stream table, so keep the stream types only.
    let _ = pmt_pids;
    streams.sort_unstable();
    streams.dedup();
    streams
}
