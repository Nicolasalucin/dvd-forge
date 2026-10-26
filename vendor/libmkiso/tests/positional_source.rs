use libmkiso::source::{BoundedSource, ReadAt, SliceSource, SourceCursor};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::Arc;

#[test]
fn bounded_reads_and_independent_cursors() {
    let source =
        Arc::new(BoundedSource::new(SliceSource::new(b"prefixPAYLOADsuffix"), 6, 7).unwrap());
    let mut first = SourceCursor::new(source.clone());
    let mut second = SourceCursor::new(source.clone());
    first.seek(SeekFrom::Start(2)).unwrap();
    let mut buf = [0; 20];
    assert_eq!(first.read(&mut buf).unwrap(), 5);
    assert_eq!(&buf[..5], b"YLOAD");
    second.read_exact(&mut buf[..7]).unwrap();
    assert_eq!(&buf[..7], b"PAYLOAD");
    assert_eq!(source.read_at(u64::MAX, &mut buf).unwrap(), 0);
    assert_eq!(
        source.read_exact_at(6, &mut buf[..2]).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        source
            .read_exact_at(u64::MAX, &mut buf[..2])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(BoundedSource::new(source.clone(), u64::MAX, 1).is_err());
    assert!(BoundedSource::new(source, 6, 2).is_err());
    assert!(second.seek(SeekFrom::Current(i64::MIN)).is_err());
    assert_eq!(second.stream_position().unwrap(), 7);
}

struct VirtualSource {
    length: u64,
}
impl ReadAt for VirtualSource {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let count = self
            .length
            .saturating_sub(offset)
            .min(buf.len() as u64)
            .min(3) as usize;
        for (i, byte) in buf[..count].iter_mut().enumerate() {
            *byte = ((offset + i as u64) % 251) as u8;
        }
        Ok(count)
    }
}

#[test]
fn huge_virtual_source_repeated_short_reads_use_caller_buffer() {
    let source = BoundedSource::new(VirtualSource { length: 1 << 44 }, 8193, 1 << 40).unwrap();
    let mut buf = [0; 127];
    for offset in [0, 1 << 32, (1 << 40) - 127, 25, 1 << 32] {
        source.read_exact_at(offset, &mut buf).unwrap();
        for (i, byte) in buf.iter().enumerate() {
            assert_eq!(*byte, ((8193 + offset + i as u64) % 251) as u8);
        }
    }
}

struct Failure;
impl ReadAt for Failure {
    fn len(&self) -> u64 {
        100
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "cancelled by source",
        ))
    }
}
#[test]
fn cancellation_cause_is_preserved() {
    let mut buf = [0; 1];
    let error = Failure.read_exact_at(0, &mut buf).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "cancelled by source");
    let mut cursor = SourceCursor::new(Failure);
    assert_eq!(
        cursor.read_exact(&mut buf).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
}

fn both32(bytes: &mut [u8], value: u32) {
    bytes[..4].copy_from_slice(&value.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.to_be_bytes());
}
fn both16(bytes: &mut [u8], value: u16) {
    bytes[..2].copy_from_slice(&value.to_le_bytes());
    bytes[2..4].copy_from_slice(&value.to_be_bytes());
}
fn record(name: &[u8], extent: u32, size: u32, directory: bool) -> Vec<u8> {
    let len = (33 + name.len() + 1) & !1;
    let mut bytes = vec![0; len];
    bytes[0] = len as u8;
    both32(&mut bytes[2..10], extent);
    both32(&mut bytes[10..18], size);
    bytes[18..25].copy_from_slice(&[126, 10, 5, 12, 0, 0, 0]);
    bytes[25] = if directory { 2 } else { 0 };
    both16(&mut bytes[28..32], 1);
    bytes[32] = name.len() as u8;
    bytes[33..33 + name.len()].copy_from_slice(name);
    bytes
}
fn fixture() -> Vec<u8> {
    let mut image = vec![0; 22 * 2048];
    let primary = &mut image[16 * 2048..17 * 2048];
    primary[0] = 1;
    primary[1..6].copy_from_slice(b"CD001");
    primary[6] = 1;
    both32(&mut primary[80..88], 22);
    both16(&mut primary[120..124], 1);
    both16(&mut primary[124..128], 1);
    both16(&mut primary[128..132], 2048);
    let root = record(&[0], 20, 2048, true);
    primary[156..156 + root.len()].copy_from_slice(&root);
    image[17 * 2048] = 255;
    image[17 * 2048 + 1..17 * 2048 + 6].copy_from_slice(b"CD001");
    image[17 * 2048 + 6] = 1;
    let mut offset = 20 * 2048;
    for bytes in [
        root,
        record(&[1], 20, 2048, true),
        record(b"HELLO.TXT;1", 21, 5, false),
    ] {
        image[offset..offset + bytes.len()].copy_from_slice(&bytes);
        offset += bytes.len();
    }
    image[21 * 2048..21 * 2048 + 5].copy_from_slice(b"hello");
    image
}

#[test]
fn iso_extents_remain_relative_to_nonzero_region() {
    let image = fixture();
    let mut disk = vec![0xa5; 917];
    disk.extend_from_slice(&image);
    disk.extend_from_slice(&[0x5a; 513]);
    let region =
        Arc::new(BoundedSource::new(SliceSource::new(&disk), 917, image.len() as u64).unwrap());
    let mut archive = libmkiso::IsoReader::open(
        SourceCursor::new(region.clone()),
        libmkiso::IsoLimits::default(),
    )
    .unwrap();
    assert_eq!(archive.index().extents[0][0].offset, 21 * 2048);
    let mut content = Vec::new();
    archive.extract(0, &mut content).unwrap();
    assert_eq!(content, b"hello");
    let mut direct = [0; 5];
    region.read_exact_at(21 * 2048, &mut direct).unwrap();
    assert_eq!(&direct, b"hello");
}

#[test]
fn retained_iso_inventory_reads_after_reader_drop() {
    use libmkiso::tree_source::{FileTreeSource, TreeEntryKind, TreeExtent};
    let mut bytes = vec![0; 917];
    bytes.extend_from_slice(&fixture());
    struct Owned(Vec<u8>);
    impl ReadAt for Owned {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            SliceSource::new(&self.0).read_at(offset, buffer)
        }
    }
    let region = BoundedSource::new(Owned(bytes), 917, 22 * 2048).unwrap();
    let reader =
        libmkiso::IsoReader::open(SourceCursor::new(region), libmkiso::IsoLimits::default())
            .unwrap();
    let source = libmkiso::iso_tree_source::IsoTreeSource::new(reader);
    assert!(source.inventory(0, usize::MAX).is_err());
    assert!(source.inventory(100, 1).is_err());
    let inventory = source.inventory(100, 1024 * 1024).unwrap();
    assert!(inventory.root_streams.is_empty());
    assert!(inventory.system_streams.is_empty());
    assert_eq!(inventory.entries[0].path, "HELLO.TXT");
    assert_eq!(inventory.entries[0].native_name, b"HELLO.TXT;1");
    assert!(inventory.entries[0].streams.is_empty());
    drop(source);
    let TreeEntryKind::File(extents) = &inventory.entries[0].kind else {
        panic!("expected file")
    };
    let TreeExtent::Data(content) = &extents[0] else {
        panic!("expected data")
    };
    let mut buf = [0; 5];
    content.read_exact_at(0, &mut buf).unwrap();
    assert_eq!(&buf, b"hello");
    content.read_exact_at(1, &mut buf[..3]).unwrap();
    assert_eq!(&buf[..3], b"ell");
    assert!(content.read_exact_at(4, &mut buf[..2]).is_err());
}

#[test]
fn selected_iso_root_and_entry_timestamps_are_inspected() {
    use libmkiso::preservation::{Field, TimestampEncoding, TimestampPrecision};
    let bytes = fixture();
    let reader = libmkiso::IsoReader::open(
        SourceCursor::new(SliceSource::new(&bytes)),
        libmkiso::IsoLimits::default(),
    )
    .unwrap();
    for metadata in [reader.root_metadata(), reader.metadata(0).unwrap()] {
        let Field::Present(timestamps) = &metadata.timestamps else {
            panic!("timestamp uninspected")
        };
        assert_eq!(timestamps[0].1.bytes, [126, 10, 5, 12, 0, 0, 0]);
        assert_eq!(timestamps[0].1.encoding, TimestampEncoding::IsoShort);
        assert_eq!(timestamps[0].1.precision, TimestampPrecision::Seconds);
        assert!(matches!(metadata.ownership, Field::Uninspected));
    }
    assert!(reader.metadata(1).is_none());
}

#[test]
fn independent_rock_ridge_hardlinks_remain_equivalent() {
    use libmkiso::tree_source::{FileTreeSource, TreeEntryKind};
    fn px(serial: u32, directory: bool) -> Vec<u8> {
        let mut px = vec![0; 44];
        px[..4].copy_from_slice(b"PX\x2c\x01");
        for (at, value) in [
            (4, if directory { 0o40755 } else { 0o100644 }),
            (12, 2),
            (20, 1000),
            (28, 1000),
            (36, serial),
        ] {
            both32(&mut px[at..at + 8], value);
        }
        px
    }
    fn append(mut record: Vec<u8>, system: &[u8]) -> Vec<u8> {
        record.extend_from_slice(system);
        record[0] = record.len() as u8;
        record
    }
    let mut image = fixture();
    let mut root_system = b"SP\x07\x01\xbe\xef\0ER\x12\x01\x0a\0\0\x01RRIP_1991A".to_vec();
    root_system.extend_from_slice(&px(1, true));
    image[20 * 2048..21 * 2048].fill(0);
    let mut offset = 20 * 2048;
    for bytes in [
        append(record(&[0], 20, 2048, true), &root_system),
        record(&[1], 20, 2048, true),
        append(record(b"FIRST;1", 21, 5, false), &px(42, false)),
        append(record(b"SECOND;1", 21, 5, false), &px(42, false)),
    ] {
        image[offset..offset + bytes.len()].copy_from_slice(&bytes);
        offset += bytes.len();
    }
    struct Owned(Vec<u8>);
    impl ReadAt for Owned {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            SliceSource::new(&self.0).read_at(offset, buffer)
        }
    }
    let reader = libmkiso::IsoReader::open_with_options(
        SourceCursor::new(Owned(image)),
        libmkiso::IsoReadOptions {
            namespace: libmkiso::IsoNamespace::RockRidge,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(matches!(
        reader.root_metadata().ownership,
        libmkiso::preservation::Field::Present((1000, 1000))
    ));
    let source = libmkiso::iso_tree_source::IsoTreeSource::new(reader);
    let inventory = source.inventory(100, 1 << 20).unwrap();
    assert_eq!(inventory.entries[0].object, inventory.entries[1].object);
    assert!(
        matches!(&inventory.entries[1].kind, TreeEntryKind::HardLink(target) if target == "FIRST")
    );
}

#[cfg(feature = "native-writer")]
#[test]
fn iso_reader_authors_udf_through_retained_content() {
    struct Owned(Vec<u8>);
    impl ReadAt for Owned {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            SliceSource::new(&self.0).read_at(offset, buffer)
        }
    }
    let reader = libmkiso::IsoReader::open(
        SourceCursor::new(Owned(fixture())),
        libmkiso::IsoLimits::default(),
    )
    .unwrap();
    let source = libmkiso::iso_tree_source::IsoTreeSource::new(reader);
    let image = libmkiso::UdfImage::from_tree_source(&source, 100, 1 << 20).unwrap();
    drop(source);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("translated.udf");
    image
        .write(&path, &libmkiso::UdfOptions::default())
        .unwrap();
    let bytes = std::fs::read(path).unwrap();
    let reader = libmkiso::UdfReader::open(&bytes, libmkiso::UdfLimits::default()).unwrap();
    let entry = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "HELLO.TXT")
        .unwrap();
    assert_eq!(reader.read_entry(entry, 5).unwrap(), b"hello");
}
