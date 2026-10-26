use libmkiso::iso9660::{IsoReader, Limits};
use std::io::Cursor;

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
fn iso_system_area_is_not_misidentified_as_empty_tar() {
    let mut archive = IsoReader::open(Cursor::new(fixture()), Limits::default()).unwrap();
    assert_eq!(archive.entries()[0].name, "HELLO.TXT");
    let mut payload = Vec::new();
    assert_eq!(archive.extract(0, &mut payload).unwrap(), 5);
    assert_eq!(payload, b"hello");
}

#[test]
fn malformed_root_cycle_is_rejected() {
    let mut bytes = fixture();
    let record = record(b"LOOP", 20, 2048, true);
    bytes[20 * 2048 + 68..20 * 2048 + 68 + record.len()].copy_from_slice(&record);
    assert!(IsoReader::open(Cursor::new(bytes), Limits::default()).is_err());
}

#[test]
fn independent_7z_reads_baseline_fixture_when_available() {
    if std::process::Command::new("7z").arg("i").output().is_err() {
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("baseline.iso");
    std::fs::write(&path, fixture()).unwrap();
    let output = std::process::Command::new("7z")
        .args(["e", "-so"])
        .arg(path)
        .arg("HELLO.TXT")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"hello");
}

// Independent ECMA-119/Joliet/SUSP construction: no libmkiso writer is used.
fn native_fixture(namespace: libmkiso::iso9660::Namespace) -> Vec<u8> {
    use libmkiso::iso9660::Namespace;
    fn named(name: &[u8], extent: u32, directory: bool, nm: Option<&str>) -> Vec<u8> {
        let mut value = record(name, extent, if directory { 2048 } else { 5 }, directory);
        if let Some(nm) = nm {
            value.extend_from_slice(&[b'N', b'M', (5 + nm.len()) as u8, 1, 0]);
            value.extend_from_slice(nm.as_bytes());
            value[0] = value.len() as u8;
        }
        value
    }
    let mut image = fixture();
    image.resize(24 * 2048, 0);
    both32(&mut image[16 * 2048 + 80..16 * 2048 + 88], 24);
    let joliet = namespace == Namespace::Joliet;
    let rr = namespace == Namespace::RockRidge;
    if joliet {
        let descriptor = image[16 * 2048..17 * 2048].to_vec();
        image[17 * 2048..18 * 2048].copy_from_slice(&descriptor);
        image[17 * 2048] = 2;
        image[17 * 2048 + 88..17 * 2048 + 91].copy_from_slice(b"%/E");
        image[18 * 2048] = 255;
        image[18 * 2048 + 1..18 * 2048 + 6].copy_from_slice(b"CD001");
        image[18 * 2048 + 6] = 1;
    }
    let encode = |name: &str| -> Vec<u8> {
        if joliet {
            name.encode_utf16().flat_map(u16::to_be_bytes).collect()
        } else {
            name.as_bytes().to_vec()
        }
    };
    let mut dot = record(&[0], 20, 2048, true);
    if rr {
        dot.extend_from_slice(b"SP\x07\x01\xbe\xef\x00ER\x12\x01\x0a\x00\x00\x01RRIP_1991A");
        dot[0] = dot.len() as u8;
    }
    let directory_name = if joliet { "日本語" } else { "DIR" };
    let mut position = 20 * 2048;
    image[position..position + 2048].fill(0);
    for bytes in [
        dot,
        record(&[1], 20, 2048, true),
        named(
            &encode(directory_name),
            22,
            true,
            rr.then_some("Native Dir"),
        ),
        named(&encode("SAME.TXT;1"), 21, false, rr.then_some("First NM")),
        named(&encode("SAME.TXT;2"), 21, false, rr.then_some("Second NM")),
    ] {
        image[position..position + bytes.len()].copy_from_slice(&bytes);
        position += bytes.len();
    }
    position = 22 * 2048;
    for bytes in [
        record(&[0], 22, 2048, true),
        record(&[1], 20, 2048, true),
        named(&encode("LEAF.TXT;1"), 23, false, rr.then_some("été NM.txt")),
    ] {
        image[position..position + bytes.len()].copy_from_slice(&bytes);
        position += bytes.len();
    }
    image[23 * 2048..23 * 2048 + 5].copy_from_slice(b"child");
    image
}

#[test]
fn native_topology_preserves_versions_and_selected_namespace_names() {
    use libmkiso::{
        iso9660::{Namespace, NativeName, ReadOptions},
        topology::Parent,
    };
    for namespace in [Namespace::Primary, Namespace::Joliet, Namespace::RockRidge] {
        let mut reader = IsoReader::open_with_options(
            Cursor::new(native_fixture(namespace)),
            ReadOptions {
                namespace,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            reader.topology().parents,
            [Parent::Root, Parent::Root, Parent::Root, Parent::Entry(0)]
        );
        assert!(reader.entries()[0].directory);
        match namespace {
            Namespace::Primary => {
                assert_eq!(reader.entries()[1].name, reader.entries()[2].name);
                assert_eq!(
                    reader.topology().names[1],
                    NativeName::Primary(b"SAME.TXT;1".to_vec())
                );
                assert_eq!(
                    reader.topology().names[2],
                    NativeName::Primary(b"SAME.TXT;2".to_vec())
                );
            }
            Namespace::Joliet => {
                assert_eq!(
                    reader.topology().names[0],
                    NativeName::Joliet("日本語".encode_utf16().collect())
                );
                assert_eq!(reader.entries()[1].name, reader.entries()[2].name);
            }
            Namespace::RockRidge => {
                assert_eq!(reader.entries()[3].raw_name, b"LEAF.TXT;1");
                assert_eq!(
                    reader.topology().names[3],
                    NativeName::RockRidge("été NM.txt".as_bytes().to_vec())
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(reader.read_entry(3, 5).unwrap(), b"child");
    }
}

#[test]
fn native_topology_rejects_invalid_parents_names_and_discovery_budgets() {
    use libmkiso::iso9660::{Error, Namespace, ReadOptions};
    let mut image = native_fixture(Namespace::Primary);
    // The native '..' record must reference the traversed parent directory.
    both32(&mut image[22 * 2048 + 34 + 2..22 * 2048 + 34 + 10], 21);
    assert!(matches!(
        IsoReader::open(Cursor::new(image), Limits::default()),
        Err(Error::Malformed(_))
    ));
    for limits in [
        Limits {
            max_nesting_depth: 0,
            ..Default::default()
        },
        Limits {
            max_metadata_bytes: 2048,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            IsoReader::open(Cursor::new(native_fixture(Namespace::Primary)), limits),
            Err(Error::ResourceLimit(_))
        ));
    }
    let mut image = native_fixture(Namespace::Joliet);
    let offset = 20 * 2048 + 68 + 33;
    image[offset..offset + 2].copy_from_slice(&0xd800u16.to_be_bytes());
    assert!(matches!(
        IsoReader::open_with_options(
            Cursor::new(image),
            ReadOptions {
                namespace: Namespace::Joliet,
                ..Default::default()
            }
        ),
        Err(Error::Malformed(_))
    ));
}

#[test]
fn lossy_primary_display_names_remain_distinct_native_occurrences() {
    let mut image = native_fixture(libmkiso::iso9660::Namespace::Primary);
    let mut position = 20 * 2048 + 68;
    position += image[position] as usize; // directory
    let first = position;
    position += image[position] as usize;
    image[first + 33] = 0xfe;
    image[position + 33] = 0xff;
    let reader = IsoReader::open(Cursor::new(image), Limits::default()).unwrap();
    assert_eq!(reader.entries()[1].name, reader.entries()[2].name);
    assert_ne!(reader.topology().names[1], reader.topology().names[2]);
}

#[test]
fn path_authoring_adapter_rejects_native_name_collisions_before_staging() {
    use libmkiso::{source::SourceCursor, tree_source::FileTreeSource};
    struct Source(Vec<u8>);
    impl libmkiso::ReadAt for Source {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<usize> {
            let offset = usize::try_from(offset).unwrap_or(usize::MAX);
            let bytes = self.0.get(offset..).unwrap_or_default();
            let amount = bytes.len().min(buffer.len());
            buffer[..amount].copy_from_slice(&bytes[..amount]);
            Ok(amount)
        }
    }
    let reader = IsoReader::open(
        SourceCursor::new(Source(native_fixture(
            libmkiso::iso9660::Namespace::Primary,
        ))),
        Limits::default(),
    )
    .unwrap();
    let source = libmkiso::iso_tree_source::IsoTreeSource::new(reader);
    assert_eq!(
        source.topology().parents[3],
        libmkiso::topology::Parent::Entry(0)
    );
    assert!(
        source
            .inventory(100, 100_000)
            .unwrap_err()
            .to_string()
            .contains("explicit conversion")
    );
    #[cfg(feature = "native-writer")]
    {
        let directory = tempfile::tempdir().unwrap();
        assert!(
            libmkiso::stage_iso9660_from_tree_source(
                &source,
                directory.path(),
                &libmkiso::IsoOptions::default(),
                || Ok(())
            )
            .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn duplicate_native_identifiers_and_invalid_rock_ridge_bytes_are_rejected() {
    use libmkiso::iso9660::{Error, Namespace, ReadOptions};
    let mut image = native_fixture(Namespace::Primary);
    let mut position = 20 * 2048 + 68;
    position += image[position] as usize; // directory
    position += image[position] as usize; // first file
    let name_length = image[position + 32] as usize;
    image[position + 33 + name_length - 1] = b'1';
    assert!(matches!(
        IsoReader::open(Cursor::new(image), Limits::default()),
        Err(Error::Malformed(_))
    ));
    let mut image = native_fixture(Namespace::RockRidge);
    let position = image
        .windows(8)
        .position(|bytes| bytes == b"First NM")
        .unwrap();
    image[position] = 0xff;
    assert!(matches!(
        IsoReader::open_with_options(
            Cursor::new(image),
            ReadOptions {
                namespace: Namespace::RockRidge,
                ..Default::default()
            }
        ),
        Err(Error::Malformed(_))
    ));
}
