#![cfg(feature = "native-writer")]
use libmkiso::udf::{Limits, UdfReader};
const BLOCK: usize = 2048;

fn retag(bytes: &mut [u8], protected: usize) {
    bytes[10..12].copy_from_slice(&(protected as u16).to_le_bytes());
    let mut crc = 0u16;
    for byte in &bytes[16..16 + protected] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    bytes[8..10].copy_from_slice(&crc.to_le_bytes());
    bytes[4] = bytes[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn get32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn fixture() -> (Vec<u8>, usize, u32) {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    std::fs::create_dir_all(source.join("boot")).unwrap();
    std::fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
    std::fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
    std::fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
    std::fs::write(source.join("payload.txt"), b"hello world").unwrap();
    let image = tmp.path().join("image.iso");
    libmkiso::write_iso(&source, &image).unwrap();
    let bytes = std::fs::read(image).unwrap();
    let entry = bytes
        .chunks_exact(BLOCK)
        .enumerate()
        .find(|(_, block)| {
            block[0..2] == 261u16.to_le_bytes()
                && block[27] == 5
                && block[56..64] == 11u64.to_le_bytes()
        })
        .unwrap()
        .0
        * BLOCK;
    let payload = get32(&bytes, entry + 180);
    (bytes, entry, payload)
}
fn read(bytes: &[u8]) -> Vec<u8> {
    let reader = UdfReader::open(bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "payload.txt")
        .unwrap();
    reader.read_entry(index, 8192).unwrap()
}

fn extended(
    bytes: &mut [u8],
    entry: usize,
    payload: u32,
    kind: u32,
    extent: u32,
    recorded: u32,
    information: u32,
) {
    let fe = &mut bytes[entry..entry + BLOCK];
    fe[34..36].copy_from_slice(&2u16.to_le_bytes());
    put32(fe, 172, 20);
    fe[176..196].fill(0);
    put32(fe, 176, kind << 30 | extent);
    put32(fe, 180, recorded);
    put32(fe, 184, information);
    put32(fe, 188, payload);
    retag(fe, 180);
}
#[test]
fn extended_recorded_data_excludes_allocated_padding() {
    let (mut bytes, entry, payload) = fixture();
    extended(&mut bytes, entry, payload, 0, 2048, 11, 11);
    assert_eq!(read(&bytes), b"hello world");
}
#[test]
fn extended_sparse_descriptors_extract_only_information_length() {
    for kind in [1, 2] {
        let (mut bytes, entry, payload) = fixture();
        extended(&mut bytes, entry, payload, kind, 2048, 0, 11);
        assert_eq!(read(&bytes), [0; 11]);
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let index = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "payload.txt")
            .unwrap();
        let mut extents = Vec::new();
        reader
            .visit_classified_extents(index, |logical, class, length| {
                extents.push((logical, class, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            extents,
            [(
                0,
                if kind == 1 {
                    libmkiso::UdfExtentKind::AllocatedUnrecorded
                } else {
                    libmkiso::UdfExtentKind::Unallocated
                },
                11
            )]
        );
    }
}
#[test]
fn extended_lengths_reject_encoding_reserved_bits_and_out_of_bounds() {
    for (extent, recorded, information) in [
        (11, 12, 11),
        (11, 11, 12),
        (11, 10, 11),
        (11, 0x80000000, 11),
    ] {
        let (mut bytes, entry, payload) = fixture();
        extended(&mut bytes, entry, payload, 0, extent, recorded, information);
        assert!(UdfReader::open(&bytes, Limits::default()).is_err());
    }
}
#[test]
fn extended_sparse_recorded_data_and_unknown_partitions_are_rejected() {
    for partition in [false, true] {
        let (mut bytes, entry, payload) = fixture();
        extended(
            &mut bytes,
            entry,
            payload,
            2,
            11,
            if partition { 0 } else { 1 },
            11,
        );
        if partition {
            bytes[entry + 192..entry + 194].copy_from_slice(&1u16.to_le_bytes());
            retag(&mut bytes[entry..entry + BLOCK], 180);
        }
        assert!(UdfReader::open(&bytes, Limits::default()).is_err());
    }
}

#[test]
fn extended_continuation_uses_extended_descriptors_and_checks_cycles() {
    let (mut bytes, entry, payload) = fixture();
    let pd = 259 * BLOCK;
    let start = get32(&bytes, pd + 188) as usize;
    let blocks = get32(&bytes, pd + 192);
    let offset = (start + blocks as usize) * BLOCK;
    bytes.resize(bytes.len().max(offset + BLOCK), 0);
    put32(&mut bytes, pd + 192, blocks + 1);
    retag(&mut bytes[pd..pd + BLOCK], 496);
    extended(&mut bytes, entry, blocks, 3, 2048, 0, 0);
    let aed = &mut bytes[offset..offset + BLOCK];
    aed.fill(0);
    aed[..2].copy_from_slice(&258u16.to_le_bytes());
    aed[2..4].copy_from_slice(&2u16.to_le_bytes());
    put32(aed, 12, blocks);
    put32(aed, 20, 20);
    put32(aed, 24, 2048);
    put32(aed, 28, 11);
    put32(aed, 32, 11);
    put32(aed, 36, payload);
    retag(aed, 28);
    assert_eq!(read(&bytes), b"hello world");
    let aed = &mut bytes[offset..offset + BLOCK];
    put32(aed, 24, 0xc0000800);
    put32(aed, 28, 0);
    put32(aed, 32, 0);
    put32(aed, 36, blocks);
    retag(aed, 28);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn extended_preallocated_tail_has_no_information_bytes() {
    let (mut bytes, entry, payload) = fixture();
    extended(&mut bytes, entry, payload, 0, 2048, 11, 11);
    let fe = &mut bytes[entry..entry + BLOCK];
    put32(fe, 172, 40);
    fe[196..216].fill(0);
    put32(fe, 196, 0x40000800);
    put32(fe, 208, payload);
    retag(fe, 200);
    assert_eq!(read(&bytes), b"hello world");
    put32(&mut bytes[entry..entry + BLOCK], 204, 1);
    retag(&mut bytes[entry..entry + BLOCK], 200);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn extended_information_lengths_must_equal_file_information_length() {
    let (mut bytes, entry, payload) = fixture();
    extended(&mut bytes, entry, payload, 0, 2048, 12, 12);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}
