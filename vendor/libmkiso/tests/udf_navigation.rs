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

#[test]
fn symbolic_pathname_is_decoded_without_following_target() {
    let mut image = fixture(0x201, false, true);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    let target = b"\x03\x00\x00\x00\x05\x05\x00\x00\x08file";
    file[27] = 12;
    put64(file, 56, target.len() as u64);
    put32(file, 172, target.len() as u32);
    file[176..176 + target.len()].copy_from_slice(target);
    seal(file, 261, 2, 176 + target.len(), 3);
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.entries()[0].kind, EntryKind::SymbolicLink);
    assert_eq!(reader.entries()[0].link_target.as_deref(), Some("../file"));
    assert_eq!(reader.read_entry(0, 100).unwrap(), target);
}
#[test]
fn indirect_icb_is_resolved_and_cycles_are_rejected() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(
        (PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK,
        (PARTITION + 3) * BLOCK,
    );
    seal(
        &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK],
        261,
        3,
        183,
        3,
    );
    let indirect = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    indirect.fill(0);
    put16(indirect, 20, 4);
    indirect[27] = 3;
    put32(indirect, 36, BLOCK as u32);
    put32(indirect, 40, 3);
    seal(indirect, 259, 2, 52, 3);
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
    assert_eq!(reader.entries()[0].icb.block, 3);
    drop(reader);
    let indirect = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put32(indirect, 40, 2);
    seal(indirect, 259, 2, 52, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}
#[test]
fn chained_file_sets_select_highest_number_and_detect_cycles() {
    let mut image = fixture(0x201, false, true);
    image.copy_within(
        PARTITION * BLOCK..(PARTITION + 1) * BLOCK,
        (PARTITION + 3) * BLOCK,
    );
    let next = &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK];
    put32(next, 40, 1);
    seal(next, 256, 3, 512, 3);
    let first = &mut image[PARTITION * BLOCK..(PARTITION + 1) * BLOCK];
    put32(first, 448, BLOCK as u32);
    put32(first, 452, 3);
    seal(first, 256, 0, 512, 3);
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
    let next = &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK];
    put32(next, 448, BLOCK as u32);
    seal(next, 256, 3, 512, 3);
    assert!(UdfReader::open(&image, Limits::default()).is_err());
}
#[test]
fn hard_link_names_share_resolved_identity() {
    let mut image = fixture(0x201, false, true);
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    root.copy_within(176..220, 220);
    root[259..263].copy_from_slice(b"link");
    seal(&mut root[220..264], 257, 1, 44, 3);
    put64(root, 56, 88);
    put32(root, 172, 88);
    seal(root, 261, 1, 264, 3);
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(reader.entries().len(), 2);
    assert_eq!(reader.entries()[0].icb, reader.entries()[1].icb);
    assert_eq!(reader.parent(0), Some(libmkiso::topology::Parent::Root));
    assert_eq!(reader.parent(1), Some(libmkiso::topology::Parent::Root));
    assert_eq!(reader.read_entry(1, 7).unwrap(), b"payload");
}
#[test]
fn strategy_4096_terminal_entry_selects_direct_data() {
    let mut image = fixture(0x201, false, true);
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    put32(root, 196, 4096);
    seal(&mut root[176..220], 257, 1, 44, 3);
    seal(root, 261, 1, 220, 3);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put16(file, 20, 4096);
    seal(file, 261, 2, 183, 3);
    let terminal = &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK];
    terminal[27] = 11;
    seal(terminal, 260, 3, 36, 3);
    assert_eq!(
        UdfReader::open(&image, Limits::default())
            .unwrap()
            .read_entry(0, 7)
            .unwrap(),
        b"payload"
    );
}

struct LogicalImage {
    prefix: Vec<u8>,
    failure: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl libmkiso::source::ReadAt for LogicalImage {
    fn len(&self) -> u64 {
        32 << 30
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.failure.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected payload failure",
            ));
        }
        let count = buffer
            .len()
            .min(13)
            .min(self.len().saturating_sub(offset) as usize);
        buffer[..count].fill(0);
        for (i, byte) in buffer[..count].iter_mut().enumerate() {
            if let Some(value) = self.prefix.get(offset as usize + i) {
                *byte = *value;
            }
        }
        Ok(count)
    }
}
#[test]
fn retained_large_positional_source_random_reads_and_shared_controls() {
    use libmkiso::source::BoundedSource;
    use libmkiso::udf::{Error, SourceLimits};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let failure = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut prefix = vec![7; 511];
    prefix.extend(fixture(0x201, false, false));
    let source = BoundedSource::new(
        LogicalImage {
            prefix,
            failure: failure.clone(),
        },
        511,
        16 << 30,
    )
    .unwrap();
    let reader = UdfReader::open_source_with_limits(
        source,
        Limits::default(),
        SourceLimits {
            cancelled: cancelled.clone(),
            max_scratch_bytes: 128,
            max_read_bytes: 4 << 20,
        },
    )
    .unwrap();
    assert!(reader.metadata_bytes() < 64 * 1024);
    assert!(reader.source_read_bytes() < 1 << 20);
    let mut buffer = [0; 20];
    for offset in [0, 3, 6, 2, 3] {
        let count = reader.read_at(0, offset, &mut buffer).unwrap();
        assert_eq!(&buffer[..count], &b"payload"[offset as usize..]);
    }
    assert_eq!(reader.read_at(0, u64::MAX, &mut buffer).unwrap(), 0);
    failure.store(true, Ordering::Relaxed);
    assert!(
        matches!(reader.read_at(0, 0, &mut buffer), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
    failure.store(false, Ordering::Relaxed);
    cancelled.store(true, Ordering::Relaxed);
    assert!(
        matches!(reader.read_at(0, 0, &mut buffer), Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted)
    );
}
#[test]
fn positional_discovery_failure_and_shared_read_budget() {
    use libmkiso::udf::SourceLimits;
    let failure = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    assert!(
        matches!(UdfReader::open_source(LogicalImage { prefix: fixture(0x201, false, false), failure }, Limits::default()), Err(libmkiso::udf::Error::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
    );
    assert!(
        UdfReader::open_source_with_limits(
            libmkiso::source::SliceSource::new(&fixture(0x201, false, false)),
            Limits::default(),
            SourceLimits {
                max_read_bytes: 2047,
                ..SourceLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn holes_are_visible_and_cancellation_applies_without_source_reads() {
    use libmkiso::udf::SourceLimits;
    let mut image = fixture(0x201, false, false);
    let file = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    put32(file, 176, (2 << 30) | BLOCK as u32);
    seal(file, 261, 2, 192, 3);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = UdfReader::open_source_with_limits(
        libmkiso::source::SliceSource::new(&image),
        Limits::default(),
        SourceLimits {
            cancelled: cancelled.clone(),
            ..SourceLimits::default()
        },
    )
    .unwrap();
    let mut extents = Vec::new();
    reader
        .visit_extents(0, |logical, source, size| {
            extents.push((logical, source, size));
            Ok(())
        })
        .unwrap();
    assert_eq!(extents, [(0, None, 7)]);
    assert_eq!(reader.read_entry(0, 7).unwrap(), [0; 7]);
    cancelled.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(reader.read_entry(0, 7).is_err());
    assert!(reader.visit_extents(0, |_, _, _| Ok(())).is_err());
    let mut callbacks = 0;
    assert!(
        reader
            .visit_classified_extents(0, |_, _, _| {
                callbacks += 1;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(callbacks, 0);
}

#[cfg(feature = "native-writer")]
#[test]
fn author_from_retained_reader_without_extraction() {
    use libmkiso::{
        udf_tree::UdfTreeSource,
        udf_writer::{UdfImage, UdfOptions},
    };
    let source = LogicalImage {
        prefix: fixture(0x201, false, false),
        failure: Default::default(),
    };
    #[allow(clippy::arc_with_non_send_sync)]
    let source = UdfTreeSource(std::sync::Arc::new(
        UdfReader::open_source(source, Limits::default()).unwrap(),
    ));
    let image = UdfImage::from_tree_source(&source, 10, 64 * 1024).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("reader-copy.udf");
    image.write(&output, &UdfOptions::default()).unwrap();
    let bytes = std::fs::read(output).unwrap();
    let copied = UdfReader::open(&bytes, Limits::default()).unwrap();
    assert_eq!(copied.read_entry(0, 7).unwrap(), b"payload");
}

#[test]
fn deferred_repeated_reads_share_the_discovery_budget() {
    use libmkiso::{source::SliceSource, udf::SourceLimits};
    let image = fixture(0x201, false, false);
    let baseline = UdfReader::open(&image, Limits::default())
        .unwrap()
        .source_read_bytes();
    let reader = UdfReader::open_source_with_limits(
        SliceSource::new(&image),
        Limits::default(),
        SourceLimits {
            max_read_bytes: baseline + 7,
            ..SourceLimits::default()
        },
    )
    .unwrap();
    assert_eq!(reader.read_entry(0, 7).unwrap(), b"payload");
    assert_eq!(reader.source_read_bytes(), baseline + 7);
    assert!(reader.read_entry(0, 7).is_err());
}

#[test]
fn independent_nested_topology_and_root_icb_are_indexed_during_traversal() {
    use libmkiso::{UdfIcbIdentity, topology::Parent};
    let mut image = fixture(0x201, false, true);
    image.copy_within(
        (PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK,
        (PARTITION + 3) * BLOCK,
    );
    seal(
        &mut image[(PARTITION + 3) * BLOCK..(PARTITION + 4) * BLOCK],
        261,
        3,
        183,
        3,
    );
    let root = &mut image[(PARTITION + 1) * BLOCK..(PARTITION + 2) * BLOCK];
    root[176 + 18] = 2;
    root[215..219].copy_from_slice(b"dirx");
    seal(&mut root[176..220], 257, 1, 44, 3);
    seal(root, 261, 1, 220, 3);
    let mut fid = root[176..220].to_vec();
    fid[18] = 0;
    fid[39..43].copy_from_slice(b"leaf");
    put32(&mut fid, 24, 3);
    seal(&mut fid, 257, 2, 44, 3);
    let directory = &mut image[(PARTITION + 2) * BLOCK..(PARTITION + 3) * BLOCK];
    directory.fill(0);
    put16(directory, 20, 4);
    directory[27] = 4;
    put16(directory, 34, 3);
    put64(directory, 56, 44);
    put32(directory, 172, 44);
    directory[176..220].copy_from_slice(&fid);
    seal(directory, 261, 2, 220, 3);
    let reader = UdfReader::open(&image, Limits::default()).unwrap();
    assert_eq!(
        reader.root_icb(),
        UdfIcbIdentity {
            partition: 0,
            block: 1
        }
    );
    assert_eq!(reader.parent(0), Some(Parent::Root));
    assert_eq!(reader.parent(1), Some(Parent::Entry(0)));
    assert_eq!(reader.parent(99), None);
    assert_eq!(reader.entries()[1].raw_name, b"\x08leaf");
    assert_eq!(reader.read_entry(1, 7).unwrap(), b"payload");
}
