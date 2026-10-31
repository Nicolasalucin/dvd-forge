use crate::report::CheckList;
use crate::{be16, be32, fail_with, paths_with_prefix, read_file, warn_with};
use libmkiso::UdfReader;

const DVD9_BYTES: u64 = 4_700_000_000;

#[derive(Debug, Clone, Copy)]
struct Cell {
    pgc: usize,
    index: usize,
    category: u8,
    start: u64,
    end: u64,
}

pub(crate) fn verify(reader: &UdfReader<'_>, size: u64, checks: &mut CheckList) {
    check_ifo_bup(reader, checks);
    check_vts_ifo(reader, checks);
    let cells = check_pgc(reader, size, checks);
    check_layer_break(reader, size, &cells, checks);
    check_region(reader, checks);
    check_vob(reader, checks);
    check_spu_packets(reader, checks);
    check_audio_packets(reader, checks);
    crate::menu::check_dvd_spu_buttons(reader, checks);
}

fn check_ifo_bup(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let paths = paths_with_prefix(reader, "VIDEO_TS/");
    let ifos: Vec<String> = paths
        .into_iter()
        .filter(|path| path.ends_with(".IFO"))
        .collect();
    if ifos.is_empty() {
        fail_with(
            checks,
            "DVD-IFO-BUP",
            "No hay archivos IFO.".to_string(),
            "VIDEO_TS",
            None,
        );
        return;
    }
    let mut failures = 0;
    for ifo_path in &ifos {
        let bup_path = ifo_path.trim_end_matches(".IFO").to_string() + ".BUP";
        let ifo = read_file(reader, ifo_path, 32 * 1024 * 1024);
        let bup = read_file(reader, &bup_path, 32 * 1024 * 1024);
        match (ifo, bup) {
            (Ok(ifo), Ok(bup)) if ifo == bup => {}
            (Ok(_), Ok(_)) => {
                failures += 1;
                fail_with(
                    checks,
                    "DVD-IFO-BUP",
                    format!("{ifo_path} y {bup_path} no son idénticos."),
                    ifo_path,
                    None,
                );
            }
            (Ok(_), Err(_)) => {
                failures += 1;
                fail_with(
                    checks,
                    "DVD-IFO-BUP",
                    format!("Falta el backup {bup_path}."),
                    ifo_path,
                    None,
                );
            }
            (Err(error), _) => {
                failures += 1;
                fail_with(checks, "DVD-IFO-BUP", error, ifo_path, None);
            }
        }
    }
    if failures == 0 {
        checks.pass(
            "DVD-IFO-BUP",
            format!("{} IFO con su BUP idéntico.", ifos.len()),
        );
    }
}

fn check_vts_ifo(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let paths = paths_with_prefix(reader, "VIDEO_TS/");
    let vts: Vec<String> = paths
        .into_iter()
        .filter(|path| {
            path.starts_with("VIDEO_TS/VTS_")
                && path.ends_with("_0.IFO")
                && path.len() >= "VIDEO_TS/VTS_01_0.IFO".len()
        })
        .collect();
    if vts.is_empty() {
        fail_with(
            checks,
            "DVD-VTS-IFO",
            "No hay VTS_*_0.IFO.".to_string(),
            "VIDEO_TS",
            None,
        );
    } else {
        checks.pass("DVD-VTS-IFO", format!("{} VTS IFO encontrados.", vts.len()));
    }
}

fn check_pgc(reader: &UdfReader<'_>, size: u64, checks: &mut CheckList) -> Vec<Cell> {
    let paths = paths_with_prefix(reader, "VIDEO_TS/");
    let mut all_cells = Vec::new();
    let iso_sectors = size / 2048;
    for path in paths
        .into_iter()
        .filter(|path| path.starts_with("VIDEO_TS/VTS_") && path.ends_with("_0.IFO"))
    {
        let bytes = match read_file(reader, &path, 32 * 1024 * 1024) {
            Ok(bytes) => bytes,
            Err(error) => {
                fail_with(checks, "DVD-PGC", error, &path, None);
                continue;
            }
        };
        match parse_cells(&bytes, iso_sectors) {
            Ok(cells) if cells.is_empty() => {
                fail_with(
                    checks,
                    "DVD-PGC",
                    "El VTS IFO no contiene celdas.".to_string(),
                    &path,
                    None,
                );
            }
            Ok(cells) => {
                let mut sorted = cells.clone();
                sorted.sort_by_key(|cell| cell.start);
                let mut previous_end = 0;
                let mut overlap = None;
                for cell in &sorted {
                    if cell.start < previous_end {
                        overlap = Some(*cell);
                    }
                    previous_end = previous_end.max(cell.end);
                }
                if let Some(cell) = overlap {
                    fail_with(
                        checks,
                        "DVD-PGC",
                        format!(
                            "Celdas solapadas en PGC {} (índice {}, {}–{}).",
                            cell.pgc, cell.index, cell.start, cell.end
                        ),
                        &path,
                        None,
                    );
                } else {
                    checks.pass(
                        "DVD-PGC",
                        format!("{path}: {} celdas dentro de la imagen.", cells.len()),
                    );
                }
                all_cells.extend(cells);
            }
            Err(error) => fail_with(checks, "DVD-PGC", error, &path, None),
        }
    }
    all_cells
}

fn check_layer_break(_reader: &UdfReader<'_>, size: u64, cells: &[Cell], checks: &mut CheckList) {
    if size <= DVD9_BYTES {
        checks.skip(
            "DVD-LAYER-BREAK",
            "La imagen cabe en una capa; no se requiere layer break.",
        );
        return;
    }
    let Some(cell) = cells.iter().find(|cell| cell.category & 0x08 != 0) else {
        fail_with(
            checks,
            "DVD-LAYER-BREAK",
            "Imagen DVD-9 sin ninguna celda marcada como SPLIP/seamless.".to_string(),
            "VIDEO_TS",
            None,
        );
        return;
    };
    if cell.start % 16 != 0 && cell.end % 16 != 0 {
        warn_with(
            checks,
            "DVD-LAYER-BREAK",
            format!(
                "La celda SPLIP {}–{} no está alineada a 16 sectores.",
                cell.start, cell.end
            ),
            "VIDEO_TS",
            None,
        );
    } else {
        checks.pass(
            "DVD-LAYER-BREAK",
            format!(
                "Celda SPLIP en PGC {} (índice {}, {}–{}) para la imagen DVD-9.",
                cell.pgc, cell.index, cell.start, cell.end
            ),
        );
    }
}

fn check_region(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let path = "VIDEO_TS/VIDEO_TS.IFO";
    match read_file(reader, path, 8 * 1024 * 1024) {
        Ok(bytes) if bytes.len() > 0x23 && &bytes[..12] == b"DVDVIDEO-VMG" => {
            let mask = bytes[0x23];
            if mask <= 0x3F {
                checks.pass(
                    "DVD-REGION",
                    format!("Máscara de región prohibida válida: 0x{mask:02X}."),
                );
            } else {
                fail_with(
                    checks,
                    "DVD-REGION",
                    format!("Máscara de región fuera de rango: 0x{mask:02X}."),
                    path,
                    Some(0x23),
                );
            }
        }
        Ok(_) => fail_with(
            checks,
            "DVD-REGION",
            "VIDEO_TS.IFO no tiene cabecera VMG válida.".to_string(),
            path,
            None,
        ),
        Err(error) => fail_with(checks, "DVD-REGION", error, path, None),
    }
}

fn check_vob(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let paths = paths_with_prefix(reader, "VIDEO_TS/");
    let vobs: Vec<String> = paths
        .into_iter()
        .filter(|path| path.ends_with(".VOB"))
        .collect();
    if vobs.is_empty() {
        fail_with(
            checks,
            "DVD-VOB",
            "No hay archivos VOB en VIDEO_TS.".to_string(),
            "VIDEO_TS",
            None,
        );
    } else {
        checks.pass("DVD-VOB", format!("{} VOB encontrados.", vobs.len()));
    }
}

fn check_spu_packets(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let vobs: Vec<String> = paths_with_prefix(reader, "VIDEO_TS/")
        .into_iter()
        .filter(|path| path.ends_with(".VOB"))
        .collect();
    for vob in vobs {
        let Ok(prefix) = crate::read_prefix(reader, &vob, 1024 * 1024) else {
            continue;
        };
        let mut cursor = 0usize;
        while cursor + 9 < prefix.len() {
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
            let substream = prefix[data];
            if (0x20..=0x3F).contains(&substream) {
                let spu = data + 1;
                let spu_length = u16::from_be_bytes([prefix[spu], prefix[spu + 1]]) as usize;
                let control = u16::from_be_bytes([prefix[spu + 2], prefix[spu + 3]]) as usize;
                if spu_length > 4 && control >= 4 && control < spu_length {
                    checks.pass(
                        "DVD-SPU-SEQ",
                        "Paquete SPU con longitud y control_offset coherentes.".to_string(),
                    );
                    checks.pass(
                        "DVD-SPU-PALETTE",
                        "Secuencia de control SPU alcanzable dentro del paquete.".to_string(),
                    );
                    return;
                }
            }
            cursor += 4 + packet_length.max(3);
        }
    }
}

fn check_audio_packets(reader: &UdfReader<'_>, checks: &mut CheckList) {
    let vobs: Vec<String> = paths_with_prefix(reader, "VIDEO_TS/")
        .into_iter()
        .filter(|path| path.ends_with(".VOB"))
        .collect();
    for vob in vobs {
        let Ok(prefix) = crate::read_prefix(reader, &vob, 1024 * 1024) else {
            continue;
        };
        let mut cursor = 0usize;
        while cursor + 9 < prefix.len() {
            if prefix[cursor..cursor + 4] != [0x00, 0x00, 0x01, 0xBD] {
                cursor += 1;
                continue;
            }
            let packet_length =
                u16::from_be_bytes([prefix[cursor + 4], prefix[cursor + 5]]) as usize;
            let header_length = prefix[cursor + 7] as usize;
            let data = cursor + 8 + header_length;
            if data >= prefix.len() {
                break;
            }
            let substream = prefix[data];
            if (0x80..=0x8F).contains(&substream) || (0xA0..=0xA7).contains(&substream) {
                checks.pass(
                    "DVD-AUDIO-STREAM",
                    "El VOB contiene un substream de audio DVD (AC-3/DTS/LPCM).".to_string(),
                );
                return;
            }
            cursor += 4 + packet_length.max(3);
        }
    }
}

fn parse_cells(bytes: &[u8], iso_sectors: u64) -> Result<Vec<Cell>, String> {
    if bytes.len() < 0x100 || &bytes[..12] != b"DVDVIDEO-VTS" {
        return Err("Cabecera VTS inválida.".to_string());
    }
    let vob_start = u64::from(be32(bytes, 0xC4).ok_or("VTS_VOB start fuera de rango")?);
    let pgci = be32(bytes, 0xCC).ok_or("VTS_PGCI fuera de rango")? as usize * 2048;
    if pgci + 8 > bytes.len() {
        return Err("VTS_PGCI fuera del IFO.".to_string());
    }
    let pgc_count = be16(bytes, pgci).ok_or("número de PGC")? as usize;
    let mut cells = Vec::new();
    for pgc_index in 0..pgc_count {
        let entry = pgci + 8 + pgc_index * 8;
        if entry + 8 > bytes.len() {
            break;
        }
        let pgc = pgci + be32(bytes, entry + 4).ok_or("offset de PGC")? as usize;
        if pgc + 0xEC > bytes.len() {
            continue;
        }
        let cell_count = bytes[pgc + 3] as usize;
        let table = pgc + be16(bytes, pgc + 0xE8).ok_or("offset de celdas")? as usize;
        for cell_index in 0..cell_count {
            let cell = table + cell_index * 0x18;
            if cell + 0x18 > bytes.len() {
                return Err("tabla de celdas truncada.".to_string());
            }
            let category = bytes[cell];
            let start = u64::from(be32(bytes, cell + 8).ok_or("cell start")?);
            let end = u64::from(be32(bytes, cell + 0x14).ok_or("cell end")?);
            if end < start {
                return Err(format!("celda {cell_index} termina antes de empezar."));
            }
            let absolute_end = vob_start.saturating_add(end);
            if absolute_end > iso_sectors {
                return Err(format!(
                    "celda {cell_index} apunta al sector {absolute_end}, fuera de la imagen ({iso_sectors})."
                ));
            }
            cells.push(Cell {
                pgc: pgc_index,
                index: cell_index,
                category,
                start: vob_start.saturating_add(start),
                end: absolute_end,
            });
        }
    }
    Ok(cells)
}
