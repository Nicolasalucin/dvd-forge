//! Independent byte-crafted ECMA-167 descriptors, without using the crate's writer.
use libmkiso::udf::{EntryKind, Limits, UdfReader};
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

fn add_stream(image: &mut [u8], system: bool) {
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    if !system {
        put64(file, 64, 10);
        put32(file, 152, BLOCK as u32);
        put32(file, 156, 3);
        seal(file, 266, 2, 223, 3);
    } else {
        let fsd = &mut image[PARTITION * BLOCK..(PARTITION + 1) * BLOCK];
        put32(fsd, 464, BLOCK as u32);
        put32(fsd, 468, 3);
        seal(fsd, 256, 0, 512, 3);
    }
    let mut fid = [0u8; 48];
    fid[19] = 9;
    put32(&mut fid, 20, BLOCK as u32);
    put32(&mut fid, 24, 5);
    fid[38..47].copy_from_slice(b"\x08metadata");
    seal(&mut fid, 257, 3, 48, 3);
    let directory = &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK];
    put16(directory, 20, 4);
    directory[27] = 13;
    put16(directory, 34, 3);
    put64(directory, 56, 48);
    put32(directory, 172, 48);
    directory[176..224].copy_from_slice(&fid);
    seal(directory, 261, 3, 224, 3);
    let stream = &mut image[(PARTITION + 5) * BLOCK..(PARTITION + 6) * BLOCK];
    put16(stream, 20, 4);
    stream[27] = 5;
    put16(stream, 34, 3);
    put64(stream, 56, 3);
    put32(stream, 172, 3);
    stream[176..179].copy_from_slice(b"ads");
    seal(stream, 261, 5, 179, 3);
}
#[test]
fn named_and_system_streams_have_structured_owners_and_extractable_data() {
    for system in [false, true] {
        let mut image = fixture(0x201, true, true);
        add_stream(&mut image, system);
        let reader = UdfReader::open(&image, Limits::default()).unwrap();
        let (index, entry) = reader
            .entries()
            .iter()
            .enumerate()
            .find(|(_, e)| e.stream.is_some())
            .unwrap();
        assert_eq!(
            entry.kind,
            if system {
                EntryKind::SystemStream
            } else {
                EntryKind::NamedStream
            }
        );
        assert_eq!(reader.parent(index), None);
        assert_eq!(reader.parent(0), Some(libmkiso::topology::Parent::Root));
        let stream = entry.stream.as_ref().unwrap();
        assert_eq!(stream.owner, if system { None } else { Some(0) });
        assert_eq!(stream.name, "metadata");
        assert_eq!(reader.read_entry(index, 3).unwrap(), b"ads");
    }
}
#[test]
fn stream_bytes_are_charged_to_total_budget() {
    let mut image = fixture(0x201, true, true);
    add_stream(&mut image, false);
    let limits = Limits {
        max_total_bytes: 9,
        ..Limits::default()
    };
    assert!(UdfReader::open(&image, limits).is_err());
}
#[test]
fn recursive_stream_directory_is_rejected() {
    let mut image = fixture(0x201, true, true);
    add_stream(&mut image, false);
    let stream = &mut image[(PARTITION + 5) * BLOCK..(PARTITION + 6) * BLOCK];
    stream.copy_within(176..179, 216);
    put16(stream, 0, 266);
    put64(stream, 64, 3);
    put32(stream, 152, BLOCK as u32);
    put32(stream, 156, 3);
    put32(stream, 212, 3);
    seal(stream, 266, 5, 219, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}
#[test]
fn object_size_must_account_for_actual_associated_stream_bytes() {
    let mut image = fixture(0x201, true, true);
    add_stream(&mut image, false);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put64(file, 64, 11);
    seal(file, 266, 2, 223, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}

#[test]
fn root_stream_ownership_is_distinct_from_ordinary_parent_topology() {
    let mut image = fixture(0x201, true, true);
    add_stream(&mut image, false);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put64(file, 64, 7);
    put32(file, 152, 0);
    seal(file, 266, 2, 223, 3);
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    let fid = root[176..220].to_vec();
    root[64..].fill(0);
    put64(root, 64, 47);
    put32(root, 152, BLOCK as u32);
    put32(root, 156, 3);
    put32(root, 212, 44);
    root[216..260].copy_from_slice(&fid);
    seal(root, 266, 1, 260, 3);
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.stream.is_some())
        .unwrap();
    let stream = reader.entries()[index].stream.as_ref().unwrap();
    assert_eq!(stream.owner, None);
    assert!(!stream.system);
    assert_eq!(reader.parent(index), None);
    assert_eq!(reader.read_entry(index, 3).unwrap(), b"ads");
}
