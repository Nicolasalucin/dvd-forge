#![cfg(feature = "native-writer")]
use libmkiso::{
    FilenamePolicy, IsoOptions,
    iso9660::{IsoReader, Limits, Namespace, ReadOptions},
    write_iso9660_with_options,
};
use std::{fs, process::Command};

#[test]
fn joliet_unicode_and_primary_aliases_share_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("日本語")).unwrap();
    for name in ["a-b.txt", "a b.txt", "a_b.txt", "FILE_00000001"] {
        fs::write(source.join(name), name.as_bytes()).unwrap();
    }
    fs::write(source.join("日本語/été.txt"), b"unicode payload").unwrap();
    fs::write(source.join("boot.bin"), vec![0x5a; 2048]).unwrap();
    let options = IsoOptions {
        boot: libmkiso::BootOptions {
            bios: Some(libmkiso::BootImage::bios("boot.bin")),
            efi: Some(libmkiso::BootImage::efi("boot.bin")),
        },
        joliet: true,
        filename_policy: FilenamePolicy::Mangle,
        ..IsoOptions::default()
    };
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(&source, &output, &options).unwrap();
    let second = temp.path().join("second.iso");
    write_iso9660_with_options(&source, &second, &options).unwrap();
    assert_eq!(fs::read(&output).unwrap(), fs::read(second).unwrap());
    let mut reader = IsoReader::open_with_options(
        fs::File::open(&output).unwrap(),
        ReadOptions {
            namespace: Namespace::Joliet,
            ..ReadOptions::default()
        },
    )
    .unwrap();
    let entries = reader.entries().to_vec();
    for (index, entry) in entries.iter().enumerate().filter(|(_, e)| !e.directory) {
        let mut payload = Vec::new();
        reader.extract(index, &mut payload).unwrap();
        assert_eq!(payload, fs::read(source.join(&entry.name)).unwrap());
    }
    let image = fs::read(&output).unwrap();
    let catalog_sector =
        u32::from_le_bytes(image[17 * 2048 + 71..17 * 2048 + 75].try_into().unwrap()) as usize;
    let boot_sector = u32::from_le_bytes(
        image[catalog_sector * 2048 + 40..catalog_sector * 2048 + 44]
            .try_into()
            .unwrap(),
    ) as usize;
    let svd = &image[18 * 2048..19 * 2048];
    let root_sector = u32::from_le_bytes(svd[158..162].try_into().unwrap()) as usize;
    let root = &image[root_sector * 2048..(root_sector + 1) * 2048];
    let mut offset = 68;
    while root[offset] != 0 {
        let extent = u32::from_le_bytes(root[offset + 2..offset + 6].try_into().unwrap()) as usize;
        if root[offset + 25] & 2 == 0 {
            assert!(boot_sector <= extent);
        }
        offset += root[offset] as usize;
    }
    let primary = IsoReader::open(fs::File::open(&output).unwrap(), Limits::default()).unwrap();
    assert_eq!(primary.entries().len(), entries.len());
    let unique: std::collections::HashSet<_> = primary.entries().iter().map(|e| &e.name).collect();
    assert_eq!(unique.len(), entries.len());
    if let Some(seven) = ["7z", "7zz"]
        .into_iter()
        .find(|command| Command::new(command).arg("i").output().is_ok())
    {
        let extracted = temp.path().join("extracted");
        let result = Command::new(seven)
            .arg("x")
            .arg(&output)
            .arg(format!("-o{}", extracted.display()))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        for entry in entries.iter().filter(|e| !e.directory) {
            assert_eq!(
                fs::read(extracted.join(&entry.name)).unwrap(),
                fs::read(source.join(&entry.name)).unwrap()
            );
        }
    }
}

#[test]
fn invalid_joliet_names_fail_before_publishing() {
    for name in ["😀.txt".to_string(), "x".repeat(65), "bad;name".to_string()] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join(name), b"data").unwrap();
        let output = temp.path().join("disc.iso");
        let options = IsoOptions {
            joliet: true,
            filename_policy: FilenamePolicy::Mangle,
            ..IsoOptions::default()
        };
        assert!(write_iso9660_with_options(&source, &output, &options).is_err());
        assert!(!output.exists());
    }
}

#[test]
fn joliet_selection_requires_descriptor_and_preference_falls_back() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("hello.txt"), b"hello").unwrap();
    let output = temp.path().join("disc.iso");
    libmkiso::write_iso9660(&source, &output).unwrap();
    assert!(
        IsoReader::open_with_options(
            fs::File::open(&output).unwrap(),
            ReadOptions {
                namespace: Namespace::Joliet,
                ..ReadOptions::default()
            }
        )
        .is_err()
    );
    let reader = IsoReader::open_with_options(
        fs::File::open(&output).unwrap(),
        ReadOptions {
            namespace: Namespace::PreferJoliet,
            ..ReadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(reader.entries()[0].name, "HELLO.TXT");
}

#[test]
fn malformed_joliet_ucs2_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("hello.txt"), b"hello").unwrap();
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            joliet: true,
            ..IsoOptions::default()
        },
    )
    .unwrap();
    let original = fs::read(output).unwrap();
    let svd = &original[17 * 2048..18 * 2048];
    let extent = u32::from_le_bytes(svd[158..162].try_into().unwrap()) as usize * 2048;
    for bytes in [[0xd8, 0x00], [0x00, b'/'], [0x00, 0x00]] {
        let mut image = original.clone();
        image[extent + 68 + 33..extent + 68 + 35].copy_from_slice(&bytes);
        assert!(
            IsoReader::open_with_options(
                std::io::Cursor::new(image),
                ReadOptions {
                    namespace: Namespace::Joliet,
                    ..ReadOptions::default()
                }
            )
            .is_err()
        );
    }
    let mut image = original;
    image[extent + 68 + 32] = 17;
    assert!(
        IsoReader::open_with_options(
            std::io::Cursor::new(image),
            ReadOptions {
                namespace: Namespace::Joliet,
                ..ReadOptions::default()
            }
        )
        .is_err()
    );
}

#[test]
fn primary_display_collisions_are_rejected_or_aliased() {
    use libmkiso::tree_source::*;
    use std::{io, sync::Arc};
    // A deferred inventory can represent case-sensitive names even when the
    // host filesystem cannot create this directory/file pair (e.g. Windows).
    struct Source(TreeInventory);
    impl FileTreeSource for Source {
        fn inventory(&self, entries: usize, bytes: usize) -> io::Result<TreeInventory> {
            self.0.validate_budget(entries, bytes)?;
            Ok(self.0.clone())
        }
    }
    let source = Source(TreeInventory {
        entries: vec![
            TreeEntry {
                path: "FOO".into(),
                native_name: b"FOO".to_vec(),
                object: 1,
                kind: TreeEntryKind::Directory,
                streams: vec![],
                metadata: Default::default(),
            },
            TreeEntry {
                path: "foo".into(),
                native_name: b"foo".to_vec(),
                object: 2,
                kind: TreeEntryKind::File(vec![TreeExtent::Data(DeferredContent::new(Arc::new(
                    BufferContent::new(2, b"file".to_vec()),
                )))]),
                streams: vec![],
                metadata: Default::default(),
            },
        ],
        ..Default::default()
    });
    let temp = tempfile::tempdir().unwrap();
    assert!(
        libmkiso::stage_iso9660_from_tree_source(
            &source,
            temp.path(),
            &IsoOptions::default(),
            || Ok(())
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let output = libmkiso::stage_iso9660_from_tree_source(
        &source,
        temp.path(),
        &IsoOptions {
            filename_policy: FilenamePolicy::Mangle,
            ..Default::default()
        },
        || Ok(()),
    )
    .unwrap();
    let mut reader = IsoReader::open(fs::File::open(output).unwrap(), Limits::default()).unwrap();
    assert_ne!(reader.entries()[0].name, reader.entries()[1].name);
    let file = reader
        .entries()
        .iter()
        .position(|entry| !entry.directory)
        .unwrap();
    assert_eq!(reader.read_entry(file, 4).unwrap(), b"file");
}

#[test]
fn each_namespace_path_table_has_sorted_breadth_first_parents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    for path in ["日本語/inner", "zeta/aaa", "alpha/zzz", "Été/bbb"] {
        fs::create_dir_all(source.join(path)).unwrap();
    }
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            joliet: true,
            filename_policy: FilenamePolicy::Mangle,
            ..IsoOptions::default()
        },
    )
    .unwrap();
    let image = fs::read(output).unwrap();
    for descriptor_sector in [16, 17] {
        let descriptor = &image[descriptor_sector * 2048..(descriptor_sector + 1) * 2048];
        let table_size = u32::from_le_bytes(descriptor[132..136].try_into().unwrap()) as usize;
        let extent = u32::from_le_bytes(descriptor[140..144].try_into().unwrap()) as usize * 2048;
        let table = &image[extent..extent + table_size];
        let mut entries = Vec::new();
        let mut offset = 0;
        while offset < table.len() {
            let size = table[offset] as usize;
            let parent = u16::from_le_bytes(table[offset + 6..offset + 8].try_into().unwrap());
            entries.push((parent, table[offset + 8..offset + 8 + size].to_vec()));
            offset += 8 + size + size % 2;
        }
        assert!(entries[1..].windows(2).all(|p| p[0] <= p[1]));
        for (i, (parent, _)) in entries.iter().enumerate().skip(1) {
            assert!(usize::from(*parent) <= i);
        }
        assert_eq!(entries.len(), 9);
    }
}
