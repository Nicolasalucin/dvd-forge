//! Independent byte-crafted ECMA-167 descriptors, without using the crate's writer.
use libmkiso::udf::{Error, Limits, UdfReader};
const BLOCK: usize = 2048;
const PARTITION: usize = 320;
fn put16(b: &mut [u8], p: usize, n: u16) {
    b[p..p + 2].copy_from_slice(&n.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, n: u32) {
    b[p..p + 4].copy_from_slice(&n.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, n: u64) {
    b[p..p + 8].copy_from_slice(&n.to_le_bytes());
}
fn seal(b: &mut [u8], kind: u16, location: u32, length: usize, version: u16) {
    put16(b, 0, kind);
    put16(b, 2, version);
    put16(b, 10, (length - 16) as u16);
    put32(b, 12, location);
    let mut crc = 0u16;
    for byte in &b[16..length] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    put16(b, 8, crc);
    b[4] = b[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, v)| sum.wrapping_add(*v));
}
fn fixture(revision: u16, duplicated: bool) -> Vec<u8> {
    let mut image = vec![0u8; 520 * BLOCK];
    let anchor = &mut image[256 * BLOCK..257 * BLOCK];
    put32(anchor, 16, 4 * BLOCK as u32);
    put32(anchor, 20, 257);
    seal(anchor, 2, 256, 512, 3);
    seal(&mut image[257 * BLOCK..258 * BLOCK], 1, 257, 512, 3);
    let pd = &mut image[258 * BLOCK..259 * BLOCK];
    put32(pd, 184, 1);
    put32(pd, 188, PARTITION as u32);
    put32(pd, 192, 200);
    seal(pd, 5, 258, 512, 3);
    let lvd = &mut image[259 * BLOCK..260 * BLOCK];
    put32(lvd, 212, BLOCK as u32);
    put16(lvd, 240, revision);
    put32(lvd, 248, BLOCK as u32);
    put16(lvd, 256, 1);
    put32(lvd, 264, 70);
    put32(lvd, 268, 2);
    lvd[440..442].copy_from_slice(&[1, 6]);
    put16(lvd, 442, 1);
    let map = &mut lvd[446..510];
    map[0] = 2;
    map[1] = 64;
    map[5..28].copy_from_slice(b"*UDF Metadata Partition");
    put16(map, 28, revision);
    put16(map, 36, 1);
    put32(map, 40, 0);
    put32(map, 44, 1);
    put32(map, 48, u32::MAX);
    put32(map, 52, 32);
    put16(map, 56, 1);
    map[58] = u8::from(duplicated);
    seal(lvd, 6, 259, 510, 3);
    seal(&mut image[260 * BLOCK..261 * BLOCK], 8, 260, 16, 3);
    for (icb, kind, a, b) in [
        (0, 250, 32, 96),
        (
            1,
            251,
            if duplicated { 128 } else { 32 },
            if duplicated { 160 } else { 96 },
        ),
    ] {
        let file = &mut image[(PARTITION + icb) * BLOCK..(PARTITION + icb + 1) * BLOCK];
        put16(file, 20, 4);
        file[27] = kind;
        put64(file, 56, 64 * BLOCK as u64);
        put32(file, 172, 16);
        put32(file, 176, 32 * BLOCK as u32);
        put32(file, 180, a as u32);
        put32(file, 184, 32 * BLOCK as u32);
        put32(file, 188, b as u32);
        seal(file, 261, icb as u32, 192, 3);
    }
    let fsd = &mut image[(PARTITION + 32) * BLOCK..(PARTITION + 33) * BLOCK];
    put32(fsd, 400, BLOCK as u32);
    put32(fsd, 404, 32);
    put16(fsd, 408, 1);
    seal(fsd, 256, 0, 512, 3);
    let mut fid = [0u8; 44];
    fid[19] = 5;
    put32(&mut fid, 20, BLOCK as u32);
    put32(&mut fid, 24, 33);
    put16(&mut fid, 28, 1);
    fid[38..43].copy_from_slice(b"\x08file");
    seal(&mut fid, 257, 32, 44, 3);
    let root = &mut image[(PARTITION + 96) * BLOCK..(PARTITION + 97) * BLOCK];
    put16(root, 20, 4);
    root[27] = 4;
    put16(root, 34, 3);
    put64(root, 56, 44);
    put32(root, 172, 44);
    root[176..220].copy_from_slice(&fid);
    seal(root, 261, 32, 220, 3);
    let file = &mut image[(PARTITION + 97) * BLOCK..(PARTITION + 98) * BLOCK];
    put16(file, 20, 4);
    file[27] = 5;
    put16(file, 34, 1);
    put64(file, 56, 7);
    put32(file, 172, 16);
    put32(file, 176, 7);
    put32(file, 180, 2);
    seal(file, 261, 33, 192, 3);
    image[(PARTITION + 2) * BLOCK..(PARTITION + 2) * BLOCK + 7].copy_from_slice(b"payload");
    if duplicated {
        let first = image[(PARTITION + 32) * BLOCK..(PARTITION + 64) * BLOCK].to_vec();
        image[(PARTITION + 128) * BLOCK..(PARTITION + 160) * BLOCK].copy_from_slice(&first);
        let second = image[(PARTITION + 96) * BLOCK..(PARTITION + 128) * BLOCK].to_vec();
        image[(PARTITION + 160) * BLOCK..(PARTITION + 192) * BLOCK].copy_from_slice(&second);
    }
    image
}
#[test]
fn metadata_partition_revisions_and_fragmented_shared_or_duplicate_maps_extract() {
    for revision in [0x250, 0x260] {
        for duplicated in [false, true] {
            let image = fixture(revision, duplicated);
            let reader = UdfReader::open(&image, Limits::default()).unwrap();
            assert_eq!(reader.entries()[0].name, "file");
            assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
        }
    }
}
#[test]
fn damaged_primary_descriptor_uses_duplicate_mirror() {
    let mut image = fixture(0x260, true);
    image[(PARTITION + 96) * BLOCK + 56] ^= 1;
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}
#[test]
fn both_corrupt_metadata_entries_fail() {
    let mut image = fixture(0x250, false);
    image[PARTITION * BLOCK + 56] ^= 1;
    image[(PARTITION + 1) * BLOCK + 56] ^= 1;
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Integrity(_))
    ));
}
#[test]
fn metadata_mapping_and_budgets_are_bounded() {
    let image = fixture(0x260, true);
    assert!(matches!(
        UdfReader::open(
            &image,
            Limits {
                max_metadata_bytes: 2048,
                ..Limits::default()
            }
        ),
        Err(Error::ResourceLimit(_))
    ));
    let mut image = image;
    let lvd = &mut image[259 * BLOCK..260 * BLOCK];
    put32(lvd, 252, 64);
    seal(lvd, 6, 259, 510, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}
#[test]
fn invalid_metadata_allocation_units_are_rejected() {
    let mut image = fixture(0x250, false);
    let lvd = &mut image[259 * BLOCK..260 * BLOCK];
    put32(lvd, 446 + 52, 1);
    seal(lvd, 6, 259, 510, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn shared_mirror_must_map_identical_storage() {
    let mut image = fixture(0x260, false);
    let mirror = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    put32(mirror, 180, 128);
    seal(mirror, 261, 1, 192, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn metadata_extent_alignment_and_exact_size_are_checked() {
    for change in [0, 1, 2] {
        let mut image = fixture(0x250, false);
        if change == 2 {
            let logical = &mut image[259 * BLOCK..260 * BLOCK];
            put16(logical, 446 + 56, 64);
            seal(logical, 6, 259, 510, 3);
        }
        for icb in [0, 1] {
            let file = &mut image[(PARTITION + icb) * BLOCK..(PARTITION + icb + 1) * BLOCK];
            if change == 0 {
                put32(file, 176, 31 * BLOCK as u32);
            } else if change == 1 {
                put32(file, 176, 0x4000_0000 | (32 * BLOCK as u32));
            }
            seal(file, 261, icb as u32, 192, 3);
        }
        assert!(UdfReader::open(&image, Limits::default()).is_err());
    }
}

#[test]
fn damaged_file_set_uses_duplicate_mirror() {
    let mut image = fixture(0x260, true);
    image[(PARTITION + 32) * BLOCK + 404] ^= 1;
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn embedded_file_extraction_uses_selected_mirror_descriptor() {
    let mut image = fixture(0x260, true);
    for block in [97, 161] {
        let file = &mut image[(PARTITION + block) * BLOCK..(PARTITION + block + 1) * BLOCK];
        put16(file, 34, 3);
        put32(file, 172, 7);
        file[176..183].copy_from_slice(b"payload");
        seal(file, 261, 33, 183, 3);
    }
    image[(PARTITION + 97) * BLOCK + 176..(PARTITION + 97) * BLOCK + 183]
        .copy_from_slice(b"damaged");
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn directory_identifier_checksum_recovers_from_duplicate_mirror() {
    let mut image = fixture(0x260, true);
    for block in [96, 160] {
        let fid =
            image[(PARTITION + block) * BLOCK + 176..(PARTITION + block) * BLOCK + 220].to_vec();
        image[(PARTITION + block + 2) * BLOCK..(PARTITION + block + 2) * BLOCK + 44]
            .copy_from_slice(&fid);
        seal(
            &mut image[(PARTITION + block + 2) * BLOCK..(PARTITION + block + 2) * BLOCK + 44],
            257,
            34,
            44,
            3,
        );
        let root = &mut image[(PARTITION + block) * BLOCK..(PARTITION + block + 1) * BLOCK];
        put16(root, 34, 0);
        put32(root, 172, 8);
        put32(root, 176, 44);
        put32(root, 180, 34);
        seal(root, 261, 32, 184, 3);
    }
    image[(PARTITION + 98) * BLOCK + 39] ^= 1;
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.entries()[0].name, "file");
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn metadata_file_allocation_continuations_resolve_fragmented_partition() {
    let mut image = fixture(0x260, false);
    for (icb, aed) in [(0, 3), (1, 4)] {
        let file = &mut image[(PARTITION + icb) * BLOCK..(PARTITION + icb + 1) * BLOCK];
        put32(file, 172, 8);
        put32(file, 176, 0xc000_0800);
        put32(file, 180, aed as u32);
        seal(file, 261, icb as u32, 184, 3);
        let allocation = &mut image[(PARTITION + aed) * BLOCK..(PARTITION + aed + 1) * BLOCK];
        put32(allocation, 20, 16);
        put32(allocation, 24, 32 * BLOCK as u32);
        put32(allocation, 28, 32);
        put32(allocation, 32, 32 * BLOCK as u32);
        put32(allocation, 36, 96);
        seal(allocation, 258, aed as u32, 40, 3);
    }
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn unreferenced_metadata_holes_are_allowed_but_referenced_holes_fail() {
    let mut image = fixture(0x260, false);
    for icb in [0, 1] {
        let file = &mut image[(PARTITION + icb) * BLOCK..(PARTITION + icb + 1) * BLOCK];
        put64(file, 56, 96 * BLOCK as u64);
        put32(file, 172, 24);
        put32(file, 192, 0x8000_0000 | (32 * BLOCK as u32));
        put32(file, 196, 0);
        seal(file, 261, icb as u32, 200, 3);
    }
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
    drop(reader);
    let logical = &mut image[259 * BLOCK..260 * BLOCK];
    put32(logical, 252, 64);
    seal(logical, 6, 259, 510, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn damaged_allocation_extent_descriptor_uses_duplicate_mirror() {
    let mut image = fixture(0x260, true);
    for (file_block, aed_block) in [(97, 98), (161, 162)] {
        let file =
            &mut image[(PARTITION + file_block) * BLOCK..(PARTITION + file_block + 1) * BLOCK];
        put32(file, 176, 0xc000_0800);
        put32(file, 180, 34);
        put16(file, 184, 1);
        seal(file, 261, 33, 192, 3);
        let aed = &mut image[(PARTITION + aed_block) * BLOCK..(PARTITION + aed_block + 1) * BLOCK];
        put32(aed, 20, 16);
        put32(aed, 24, 7);
        put32(aed, 28, 2);
        seal(aed, 258, 34, 40, 3);
    }
    image[(PARTITION + 98) * BLOCK + 24] ^= 1;
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn independent_native_metadata_is_inspected_for_root_and_file() {
    use libmkiso::preservation::{Field, TimestampEncoding, TimestampPrecision};
    for revision in [0x250, 0x260] {
        let mut image = fixture(revision, false);
        let stamp = [0x3c, 0x10, 0xea, 0x07, 10, 7, 12, 34, 56, 78, 90, 12];
        for (physical, logical, length, uid) in [
            (PARTITION + 96, 32, 220, 42),
            (PARTITION + 97, 33, 192, 1001),
        ] {
            let descriptor = &mut image[physical * BLOCK..(physical + 1) * BLOCK];
            put32(descriptor, 36, uid);
            put32(descriptor, 40, 100);
            put32(descriptor, 44, 0x1234);
            descriptor[72..84].copy_from_slice(&stamp);
            seal(descriptor, 261, logical, length, 3);
        }
        let reader = UdfReader::open(&image, Limits::default()).unwrap();
        assert_eq!(reader.root_metadata().ownership, Field::Present((42, 100)));
        let metadata = reader.metadata(0).unwrap();
        assert_eq!(metadata.native_name, Field::Present(b"\x08file".to_vec()));
        assert_eq!(metadata.ownership, Field::Present((1001, 100)));
        assert_eq!(metadata.permissions, Field::Present(0x1234));
        let Field::Present(timestamps) = &metadata.timestamps else {
            panic!("timestamps not inspected")
        };
        assert_eq!(timestamps[0].1.bytes, stamp);
        assert_eq!(timestamps[0].1.encoding, TimestampEncoding::Udf);
        assert_eq!(timestamps[0].1.precision, TimestampPrecision::Microseconds);
        assert_eq!(metadata.opaque, Field::Uninspected);
    }
}
