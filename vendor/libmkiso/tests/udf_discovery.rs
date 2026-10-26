//! Independent byte-crafted ECMA-167 descriptors, without using the crate's writer.
use libmkiso::udf::{Limits, UdfReader};
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
fn backup_anchor_recovers_a_corrupted_primary_anchor() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(256 * BLOCK..257 * BLOCK, 325 * BLOCK);
    seal(&mut image[325 * BLOCK..326 * BLOCK], 2, 325, 512, 3);
    image[256 * BLOCK + 4] ^= 1;
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
}
#[test]
fn reserve_sequence_recovers_corrupted_main_sequence() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(257 * BLOCK..261 * BLOCK, 270 * BLOCK);
    for (block, kind, length) in [(270, 1, 512), (271, 5, 512), (272, 6, 446), (273, 8, 16)] {
        seal(
            &mut image[block * BLOCK..(block + 1) * BLOCK],
            kind,
            block as u32,
            length,
            3,
        );
    }
    let anchor = &mut image[256 * BLOCK..257 * BLOCK];
    put32(anchor, 24, 4 * BLOCK as u32);
    put32(anchor, 28, 270);
    seal(anchor, 2, 256, 512, 3);
    image[258 * BLOCK + 4] ^= 1;
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
}
#[test]
fn descriptor_sequence_pointer_chains_are_bounded() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(257 * BLOCK..261 * BLOCK, 270 * BLOCK);
    for (block, kind, length) in [(270, 1, 512), (271, 5, 512), (272, 6, 446), (273, 8, 16)] {
        seal(
            &mut image[block * BLOCK..(block + 1) * BLOCK],
            kind,
            block as u32,
            length,
            3,
        );
    }
    let pointer = &mut image[257 * BLOCK..258 * BLOCK];
    pointer.fill(0);
    put32(pointer, 20, 4 * BLOCK as u32);
    put32(pointer, 24, 270);
    seal(pointer, 3, 257, 28, 3);
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
    let pointer = &mut image[257 * BLOCK..258 * BLOCK];
    put32(pointer, 24, 257);
    seal(pointer, 3, 257, 28, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}
#[test]
fn prevailing_partition_descriptor_supersedes_earlier_version() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(258 * BLOCK..259 * BLOCK, 260 * BLOCK);
    let newer = &mut image[260 * BLOCK..261 * BLOCK];
    put32(newer, 16, 2);
    seal(newer, 5, 260, 512, 3);
    let earlier = &mut image[258 * BLOCK..259 * BLOCK];
    put32(earlier, 188, 319);
    seal(earlier, 5, 258, 512, 3);
    seal(&mut image[261 * BLOCK..262 * BLOCK], 8, 261, 16, 3);
    let anchor = &mut image[256 * BLOCK..257 * BLOCK];
    put32(anchor, 16, 5 * BLOCK as u32);
    seal(anchor, 2, 256, 512, 3);
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
}
