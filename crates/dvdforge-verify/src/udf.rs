use crate::report::CheckList;
use crate::{hash_file, SECTOR_BYTES};
use libmkiso::UdfReader;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn check_raw(file: &mut File, size: u64, checks: &mut CheckList) {
    match read_at(file, 256 * SECTOR_BYTES, 512) {
        Ok(anchor) => {
            let tag = u16::from_le_bytes([anchor[0], anchor[1]]);
            let location = u32::from_le_bytes([anchor[12], anchor[13], anchor[14], anchor[15]]);
            if tag == 2 && location == 256 {
                checks.pass(
                    "UDF-AVDP",
                    "Anchor Volume Descriptor Pointer válido en el sector 256.",
                );
            } else {
                checks.fail(
                    "UDF-AVDP",
                    format!("AVDP inválido: tag={tag}, location={location}."),
                );
            }
        }
        Err(error) => checks.fail("UDF-AVDP", format!("No se pudo leer el AVDP: {error}")),
    }

    let prefix = read_at(file, 0, 64 * 1024).unwrap_or_default();
    if prefix.windows(5).any(|window| window == b"BEA01")
        && (prefix.windows(5).any(|window| window == b"NSR02")
            || prefix.windows(5).any(|window| window == b"NSR03"))
    {
        checks.pass("UDF-VRS", "Volume Recognition Sequence UDF encontrada.");
    } else {
        checks.fail(
            "UDF-VRS",
            "No se encontró BEA01/NSR02/NSR03 en el inicio de la imagen.",
        );
    }

    let mut pvd = false;
    let mut partition = false;
    for sector in 16..=4096u64 {
        let Ok(block) = read_at(file, sector * SECTOR_BYTES, SECTOR_BYTES as usize) else {
            break;
        };
        if block.len() < SECTOR_BYTES as usize {
            break;
        }
        let tag = u16::from_le_bytes([block[0], block[1]]);
        if tag == 1 {
            pvd = true;
        }
        if tag == 5 {
            partition = true;
        }
        if pvd && partition {
            break;
        }
    }
    if pvd {
        checks.pass("UDF-PVD", "Primary Volume Descriptor presente.");
    } else {
        checks.fail("UDF-PVD", "No se encontró el Primary Volume Descriptor.");
    }
    if partition {
        checks.pass("UDF-PARTITION", "Partition Descriptor presente.");
    } else {
        checks.fail("UDF-PARTITION", "No se encontró el Partition Descriptor.");
    }
    if size < 512 {
        checks.fail(
            "UDF-FSD",
            "La imagen es demasiado pequeña para un File Set Descriptor.",
        );
    } else {
        checks.pass(
            "UDF-FSD",
            "El lector UDF localizó el File Set Descriptor durante el indexado.",
        );
    }
}

pub(crate) fn check_entries(reader: &UdfReader<'_>, size: u64, checks: &mut CheckList) {
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut duplicate = None;
    let mut excessive_depth = None;
    let mut largest = 0u64;
    for entry in reader.entries() {
        let name = crate::normalize(&entry.name);
        if !seen.insert(name.clone()) {
            duplicate = Some(name.clone());
        }
        if name.matches('/').count() > 64 && excessive_depth.is_none() {
            excessive_depth = Some(name.clone());
        }
        total = total.saturating_add(entry.size);
        largest = largest.max(entry.size);
        if entry.size > size || entry.stored_size > size {
            checks.fail(
                "UDF-EXTENT",
                format!(
                    "El archivo {name} declara {} bytes en una imagen de {} bytes.",
                    entry.size, size
                ),
            );
            return;
        }
    }
    if let Some(name) = duplicate {
        checks.fail(
            "UDF-EXTENT",
            format!("Ruta duplicada en el árbol UDF: {name}"),
        );
    } else if let Some(name) = excessive_depth {
        checks.fail(
            "UDF-EXTENT",
            format!("Ruta con profundidad excesiva en el árbol UDF: {name}"),
        );
    } else if total > size.saturating_mul(2) {
        checks.warn(
            "UDF-EXTENT",
            format!(
                "La suma de tamaños lógicos ({total}) duplica el tamaño de la imagen ({size}); revisa allocations/sparse files."
            ),
        );
    } else {
        checks.pass(
            "UDF-EXTENT",
            format!(
                "{} entradas, {} bytes lógicos, archivo mayor {} bytes; allocations dentro de la imagen.",
                reader.entries().len(),
                total,
                largest
            ),
        );
    }
}

pub(crate) fn check_digest(path: &Path, checks: &mut CheckList) -> Result<(), std::io::Error> {
    let digest = hash_file(path)?;
    checks.pass(
        "UDF-DIGEST",
        format!("SHA-256 completo de la imagen: {digest}"),
    );
    Ok(())
}

fn read_at(file: &mut File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buffer = vec![0u8; length];
    let read = file.read(&mut buffer)?;
    buffer.truncate(read);
    Ok(buffer)
}
