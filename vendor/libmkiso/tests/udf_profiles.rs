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
fn fixture(revision: u16, extended: bool, embedded: bool) -> Vec<u8> {
    let mut image = vec![0u8; 326 * BLOCK];
    let version = if revision >= 0x200 { 3 } else { 2 };
    let anchor = &mut image[256 * BLOCK..257 * BLOCK];
    put32(anchor, 16, 4 * BLOCK as u32);
    put32(anchor, 20, 257);
    seal(anchor, 2, 256, 512, version);
    let pvd = &mut image[257 * BLOCK..258 * BLOCK];
    seal(pvd, 1, 257, 512, version);
    let pd = &mut image[258 * BLOCK..259 * BLOCK];
    put32(pd, 188, PARTITION as u32);
    put32(pd, 192, 6);
    seal(pd, 5, 258, 512, version);
    let lvd = &mut image[259 * BLOCK..260 * BLOCK];
    put32(lvd, 212, BLOCK as u32);
    put16(lvd, 240, revision);
    put32(lvd, 248, BLOCK as u32);
    put32(lvd, 264, 6);
    put32(lvd, 268, 1);
    lvd[440..442].copy_from_slice(&[1, 6]);
    put16(lvd, 442, 1);
    seal(lvd, 6, 259, 446, version);
    let end = &mut image[260 * BLOCK..261 * BLOCK];
    seal(end, 8, 260, 16, version);
    let fsd = &mut image[PARTITION * BLOCK..(PARTITION + 1) * BLOCK];
    put32(fsd, 400, BLOCK as u32);
    put32(fsd, 404, 1);
    seal(fsd, 256, 0, 512, version);
    let mut fid = [0u8; 44];
    fid[19] = 5;
    put32(&mut fid, 20, BLOCK as u32);
    put32(&mut fid, 24, 2);
    fid[38..43].copy_from_slice(b"\x08file");
    seal(&mut fid, 257, 1, 44, version);
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    put16(root, 20, 4);
    root[27] = 4;
    put16(root, 34, 3);
    put64(root, 56, 44);
    put32(root, 172, 44);
    root[176..220].copy_from_slice(&fid);
    seal(root, 261, 1, 220, version);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    let header = if extended { 216 } else { 176 };
    put16(file, 20, 4);
    file[27] = 5;
    put16(file, 34, if embedded { 3 } else { 1 });
    put64(file, 56, 7);
    if extended {
        put64(file, 64, 7);
    }
    if embedded {
        put32(file, header - 4, 7);
        file[header..header + 7].copy_from_slice(b"payload");
    } else {
        put32(file, header - 4, 16);
        put32(file, header, BLOCK as u32);
        put32(file, header + 4, 4);
    }
    seal(
        file,
        if extended { 266 } else { 261 },
        2,
        header + if embedded { 7 } else { 16 },
        version,
    );
    image[(PARTITION + 4) * BLOCK..(PARTITION + 4) * BLOCK + 7].copy_from_slice(b"payload");
    image
}
#[test]
fn physical_partition_revisions_read_long_and_embedded_file_entries() {
    for revision in [0x102, 0x150, 0x200, 0x201] {
        for extended in [false, true] {
            for embedded in [false, true] {
                let image = fixture(revision, extended, embedded);
                let reader = UdfReader::open(&image, Limits::default()).unwrap();
                assert_eq!(reader.entries()[0].name, "file");
                let before = reader.source_read_bytes();
                let mut extents = Vec::new();
                reader
                    .visit_classified_extents(0, |logical, kind, length| {
                        extents.push((logical, kind, length));
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(reader.source_read_bytes(), before);
                let source_offset = if embedded {
                    (PARTITION + 2) * BLOCK + if extended { 216 } else { 176 }
                } else {
                    (PARTITION + 4) * BLOCK
                };
                assert_eq!(
                    extents,
                    [(
                        0,
                        libmkiso::UdfExtentKind::Recorded {
                            source_offset: source_offset as u64
                        },
                        7
                    )]
                );
                assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
            }
        }
    }
}
#[test]
fn crc_must_cover_extended_allocation_fields() {
    let mut image = fixture(0x201, true, true);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    seal(file, 266, 2, 216, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}
#[test]
fn invalid_partition_and_continuation_are_rejected() {
    for (offset, value) in [(216 + 8, 1), (216, 0xc000_0800)] {
        let mut image = fixture(0x201, true, false);
        let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
        put32(file, offset, value);
        seal(file, 266, 2, 232, 3);
        assert!(UdfReader::open(&image, Limits::default()).is_err());
    }
    let mut image = fixture(0x201, true, false);
    let lvd = &mut image[259 * BLOCK..260 * BLOCK];
    lvd[440] = 2;
    seal(lvd, 6, 259, 446, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn embedded_directory_cycles_and_declared_sizes_are_bounded() {
    let image = fixture(0x201, true, true);
    let limits = Limits {
        max_entry_bytes: 6,
        ..Limits::default()
    };
    assert!(matches!(
        UdfReader::open(&image, limits),
        Err(Error::ResourceLimit(_))
    ));
    let mut image = image;
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    put32(root, 176 + 24, 1);
    seal(&mut root[176..220], 257, 1, 44, 3);
    seal(root, 261, 1, 220, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn padding_must_belong_to_final_allocation() {
    let mut image = fixture(0x201, true, false);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put32(file, 212, 32);
    put32(file, 216, 8); // The file has only seven bytes: padding before another extent.
    put32(file, 232, 1);
    put32(file, 236, 4);
    seal(file, 266, 2, 248, 3);
    assert!(matches!(
        UdfReader::open(&image, Limits::default()),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn file_set_and_root_icb_extents_must_be_recorded_and_sufficient() {
    for (block, offset, kind, location, protected) in
        [(259, 248, 6, 259, 446), (320, 400, 256, 0, 512)]
    {
        for value in [0, 128, 0xc000_0800] {
            let mut image = fixture(0x201, true, true);
            let descriptor = &mut image[block * BLOCK..(block + 1) * BLOCK];
            put32(descriptor, offset, value);
            seal(descriptor, kind, location, protected, 3);
            assert!(UdfReader::open(&image, Limits::default()).is_err());
        }
    }
}

#[test]
fn named_stream_directory_must_be_valid() {
    let mut image = fixture(0x201, true, true);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put32(file, 152, 2048);
    put64(file, 64, 123);
    seal(file, 266, 2, 223, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}

#[test]
fn inconsistent_extended_object_sizes_are_rejected() {
    for (object, stream) in [(6, 2048), (8, 0)] {
        let mut image = fixture(0x201, true, true);
        let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
        put64(file, 64, object);
        put32(file, 152, stream);
        seal(file, 266, 2, 223, 3);
        assert!(UdfReader::open(&image, Limits::default()).is_err());
    }
}

#[test]
fn independent_fe_and_efe_metadata_keep_native_fields_across_revisions() {
    use libmkiso::preservation::{Field, TimestampEncoding};
    for revision in [0x102, 0x150, 0x200, 0x201, 0x250, 0x260] {
        for extended in [false, true] {
            let mut bytes = fixture(revision, extended, true);
            let descriptor = &mut bytes[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
            put32(descriptor, 36, 123);
            put32(descriptor, 40, 456);
            put32(descriptor, 44, 0x4321);
            let offsets: &[usize] = if extended {
                &[80, 92, 104, 116]
            } else {
                &[72, 84, 96]
            };
            let stamp = [0x3c, 0x10, 0xea, 0x07, 10, 7, 12, 34, 56, 78, 90, 12];
            for (index, &offset) in offsets.iter().enumerate() {
                let mut value = stamp;
                value[11] += index as u8;
                descriptor[offset..offset + 12].copy_from_slice(&value);
            }
            seal(
                descriptor,
                if extended { 266 } else { 261 },
                2,
                if extended { 223 } else { 183 },
                if revision >= 0x200 { 3 } else { 2 },
            );
            let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
            let metadata = reader.metadata(0).unwrap();
            assert_eq!(metadata.ownership, Field::Present((123, 456)));
            assert_eq!(metadata.permissions, Field::Present(0x4321));
            let Field::Present(timestamps) = &metadata.timestamps else {
                panic!("timestamps not inspected")
            };
            assert_eq!(timestamps.len(), offsets.len());
            for (kind, timestamp) in timestamps {
                let index = match *kind {
                    4 => 0,
                    2 => 1,
                    1 if extended => 2,
                    8 => {
                        if extended {
                            3
                        } else {
                            2
                        }
                    }
                    _ => panic!("unexpected timestamp field"),
                };
                let mut expected = stamp;
                expected[11] += index;
                assert_eq!(timestamp.bytes, expected);
                assert_eq!(timestamp.encoding, TimestampEncoding::Udf);
            }
            assert_eq!(timestamps.iter().any(|(kind, _)| *kind == 1), extended);
        }
    }
}
