#![cfg(feature = "native-writer")]
use libmkiso::{
    AdvancedBootOptions, BootEmulation, BootEntry, FilenamePolicy, HybridLayout, HybridOptions,
    IsoLevel, IsoOptions, MbrBootPatch, UnixMetadataOptions, VolumeMetadata,
    iso9660::{IsoReader, Limits, Namespace, ReadOptions},
    write_iso9660_with_options,
};
use std::{fs, io::Cursor, process::Command};

fn read(bytes: &[u8], namespace: Namespace) -> IsoReader<Cursor<&[u8]>> {
    IsoReader::open_with_options(
        Cursor::new(bytes),
        ReadOptions {
            namespace,
            ..Default::default()
        },
    )
    .unwrap()
}
fn number(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
fn external(name: &str, env: &str) -> Option<String> {
    let command = std::env::var(env).unwrap_or_else(|_| name.into());
    if Command::new(&command).arg("--version").output().is_ok() {
        Some(command)
    } else {
        eprintln!("SKIP independent check: {name} unavailable; set {env}");
        None
    }
}

#[test]
fn level_three_sections_extract_in_both_namespaces_and_independent_reader() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let payload: Vec<_> = (0..10001).map(|value| (value % 251) as u8).collect();
    fs::write(source.join("data.bin"), &payload).unwrap();
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            level: IsoLevel::Level3,
            extent_bytes: 2048,
            joliet: true,
            rock_ridge: true,
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = fs::read(&output).unwrap();
    for namespace in [Namespace::Primary, Namespace::Joliet, Namespace::RockRidge] {
        let mut reader = read(&bytes, namespace);
        assert_eq!(reader.entries().len(), 1);
        assert_eq!(reader.entries()[0].size, payload.len() as u64);
        let mut extracted = Vec::new();
        reader.extract(0, &mut extracted).unwrap();
        assert_eq!(extracted, payload);
    }
    if let Some(command) = external("bsdtar", "BSDTAR") {
        let listing = Command::new(&command)
            .arg("-tf")
            .arg(&output)
            .output()
            .unwrap();
        let result = Command::new(command)
            .args(["-xOf"])
            .arg(&output)
            .arg("DATA.BIN")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}; listing {}",
            String::from_utf8_lossy(&result.stderr),
            String::from_utf8_lossy(&listing.stdout)
        );
        assert_eq!(result.stdout, payload);
    }
    if let Some(command) = external("xorriso", "XORRISO") {
        let extracted = temp.path().join("independent.bin");
        let result = Command::new(command)
            .args(["-osirrox", "on", "-indev"])
            .arg(&output)
            .arg("-extract")
            .arg("/data.bin")
            .arg(&extracted)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(fs::read(extracted).unwrap(), payload);
    }
}

#[test]
fn noncontiguous_level_three_sections_and_missing_final_record_are_checked() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("data.bin"), vec![7; 4096]).unwrap();
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            level: IsoLevel::Level3,
            extent_bytes: 2048,
            ..Default::default()
        },
    )
    .unwrap();
    let mut bytes = fs::read(output).unwrap();
    let root = number(&bytes[16 * 2048 + 158..16 * 2048 + 162]) as usize * 2048;
    let second = root + 68 + usize::from(bytes[root + 68]);
    let old = number(&bytes[second + 2..second + 6]);
    let appended = (bytes.len() / 2048) as u32;
    let block = bytes[old as usize * 2048..(old as usize + 1) * 2048].to_vec();
    bytes.extend(block);
    bytes[second + 2..second + 6].copy_from_slice(&appended.to_le_bytes());
    bytes[second + 6..second + 10].copy_from_slice(&appended.to_be_bytes());
    let total = (bytes.len() / 2048) as u32;
    bytes[16 * 2048 + 80..16 * 2048 + 84].copy_from_slice(&total.to_le_bytes());
    bytes[16 * 2048 + 84..16 * 2048 + 88].copy_from_slice(&total.to_be_bytes());
    let mut reader = read(&bytes, Namespace::Primary);
    let mut payload = Vec::new();
    reader.extract(0, &mut payload).unwrap();
    assert_eq!(payload, vec![7; 4096]);
    bytes[second + 25] |= 0x80;
    assert!(IsoReader::open(Cursor::new(bytes), Limits::default()).is_err());
}

#[test]
fn level_one_and_joliet_profiles_validate_and_preserve_volume_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let name = "a".repeat(100);
    fs::write(source.join(&name), b"names").unwrap();
    for level in 1..=3 {
        let output = temp.path().join(format!("level-{level}.iso"));
        write_iso9660_with_options(
            &source,
            &output,
            &IsoOptions {
                level: IsoLevel::Level1,
                joliet: true,
                joliet_level: level,
                joliet_max_name: 103,
                filename_policy: FilenamePolicy::Mangle,
                volume_metadata: VolumeMetadata {
                    publisher: "TEST PUBLISHER".into(),
                    application: "OPTICAL IMAGE".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        let bytes = fs::read(output).unwrap();
        assert_eq!(read(&bytes, Namespace::Joliet).entries()[0].name, name);
        assert_eq!(
            &bytes[17 * 2048 + 88..17 * 2048 + 91],
            match level {
                1 => b"%/@",
                2 => b"%/C",
                _ => b"%/E",
            }
        );
        assert_eq!(&bytes[16 * 2048 + 318..16 * 2048 + 332], b"TEST PUBLISHER");
        assert!(read(&bytes, Namespace::Primary).entries()[0].name.len() <= 12);
    }
    let invalid = temp.path().join("invalid.iso");
    assert!(
        write_iso9660_with_options(
            &source,
            &invalid,
            &IsoOptions {
                level: IsoLevel::Level1,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!invalid.exists());
}

#[cfg(unix)]
#[test]
fn rock_ridge_continuations_preserve_names_permissions_links_and_shared_inodes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("Mixed Directory")).unwrap();
    let name = "é".repeat(110);
    fs::write(source.join(&name), b"long name").unwrap();
    fs::write(source.join("Mixed Directory/script.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(
        source.join("Mixed Directory/script.sh"),
        fs::Permissions::from_mode(0o751),
    )
    .unwrap();
    fs::hard_link(
        source.join("Mixed Directory/script.sh"),
        source.join("alias"),
    )
    .unwrap();
    symlink("Mixed Directory/script.sh", source.join("link")).unwrap();
    symlink("../missing", source.join("dangling")).unwrap();
    let output = temp.path().join("disc.iso");
    let options = IsoOptions {
        rock_ridge: true,
        filename_policy: FilenamePolicy::Mangle,
        unix_metadata: UnixMetadataOptions {
            preserve: true,
            ..Default::default()
        },
        ..Default::default()
    };
    write_iso9660_with_options(&source, &output, &options).unwrap();
    let bytes = fs::read(&output).unwrap();
    let reader = read(&bytes, Namespace::RockRidge);
    let find = |name: &str| {
        reader
            .entries()
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
    };
    assert_eq!(
        find("link").link_target.as_deref(),
        Some("Mixed Directory/script.sh")
    );
    assert_eq!(find("dangling").link_target.as_deref(), Some("../missing"));
    assert_eq!(
        find("Mixed Directory/script.sh")
            .unix
            .as_ref()
            .unwrap()
            .mode
            & 0o7777,
        0o751
    );
    assert_eq!(
        find("alias").unix.as_ref().unwrap().serial,
        find("Mixed Directory/script.sh")
            .unix
            .as_ref()
            .unwrap()
            .serial
    );
    assert_eq!(find("alias").unix.as_ref().unwrap().links, 2);
    assert_eq!(find(&name).size, 9);
    assert_eq!(
        read(&bytes, Namespace::PreferRockRidge).entries()[0].name,
        reader.entries()[0].name
    );
    let second = temp.path().join("second.iso");
    write_iso9660_with_options(&source, &second, &options).unwrap();
    assert_eq!(bytes, fs::read(second).unwrap());
    if let Some(command) = external("bsdtar", "BSDTAR") {
        let destination = temp.path().join("extracted");
        fs::create_dir(&destination).unwrap();
        let result = Command::new(command)
            .arg("-xf")
            .arg(&output)
            .arg("-C")
            .arg(&destination)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read_link(destination.join("link")).unwrap(),
            std::path::Path::new("Mixed Directory/script.sh")
        );
        assert_eq!(
            fs::metadata(destination.join("Mixed Directory/script.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o751
        );
        assert_eq!(fs::read(destination.join(&name)).unwrap(), b"long name");
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(destination.join("alias")).unwrap().ino(),
            fs::metadata(destination.join("Mixed Directory/script.sh"))
                .unwrap()
                .ino()
        );
    }
}

#[test]
fn extended_boot_catalog_and_address_patches_leave_source_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let original = vec![0x33; 4096];
    fs::write(source.join("bios.bin"), &original).unwrap();
    fs::write(source.join("efi.img"), vec![0xef; 8192]).unwrap();
    let mut bios = BootEntry::bios("bios.bin");
    bios.boot_info_table = true;
    bios.grub2_boot_info = true;
    let mut alt = bios.clone();
    alt.bootable = false;
    alt.selection[0] = 7;
    let output = temp.path().join("disc.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            advanced_boot: AdvancedBootOptions {
                entries: vec![bios, alt, BootEntry::efi("efi.img")],
                catalog_id: "OPTICAL IMAGE".into(),
            },
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = fs::read(output).unwrap();
    let catalog = number(&bytes[17 * 2048 + 71..17 * 2048 + 75]) as usize * 2048;
    assert_eq!(&bytes[catalog + 4..catalog + 17], b"OPTICAL IMAGE");
    assert_eq!(bytes[catalog + 64], 0x90);
    assert_eq!(bytes[catalog + 96], 0);
    assert_eq!(bytes[catalog + 108], 7);
    assert_eq!(bytes[catalog + 128], 0x91);
    assert_eq!(bytes[catalog + 129], 0xef);
    let block = number(&bytes[catalog + 40..catalog + 44]);
    let start = block as usize * 2048;
    assert_eq!(number(&bytes[start + 8..start + 12]), 16);
    assert_eq!(number(&bytes[start + 12..start + 16]), block);
    assert_eq!(
        u64::from_le_bytes(bytes[start + 2548..start + 2556].try_into().unwrap()),
        u64::from(block) * 4 + 5
    );
    let checksum = bytes[start + 64..start + 4096]
        .chunks_exact(4)
        .fold(0u32, |sum, word| sum.wrapping_add(number(word)));
    assert_eq!(number(&bytes[start + 20..start + 24]), checksum);
    assert_eq!(fs::read(source.join("bios.bin")).unwrap(), original);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
#[test]
fn hybrid_gpt_has_consistent_primary_backup_tables_and_embedded_efi_partition() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("efi.img"), vec![0x72; 8192]).unwrap();
    fs::write(source.join("bios.bin"), vec![0x31; 4096]).unwrap();
    for layout in [HybridLayout::Mbr, HybridLayout::Gpt, HybridLayout::MbrGpt] {
        let output = temp.path().join(format!("{layout:?}.iso"));
        write_iso9660_with_options(
            &source,
            &output,
            &IsoOptions {
                advanced_boot: AdvancedBootOptions {
                    entries: vec![BootEntry::bios("bios.bin"), BootEntry::efi("efi.img")],
                    ..Default::default()
                },
                hybrid: Some(HybridOptions {
                    layout,
                    efi_partition: Some("efi.img".into()),
                    mbr_boot_code: vec![0x90; 440],
                    mbr_patch: MbrBootPatch::Syslinux,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let bytes = fs::read(&output).unwrap();
        assert_eq!(&bytes[510..512], &[0x55, 0xaa]);
        let catalog = number(&bytes[17 * 2048 + 71..17 * 2048 + 75]) as usize * 2048;
        let bios = number(&bytes[catalog + 40..catalog + 44]);
        assert_eq!(
            u64::from_le_bytes(bytes[432..440].try_into().unwrap()),
            u64::from(bios) * 4
        );
        let efi = number(&bytes[catalog + 104..catalog + 108]);
        if layout != HybridLayout::Gpt {
            assert_eq!(number(&bytes[470..474]), efi * 4);
            assert_eq!(number(&bytes[474..478]), 16);
        }
        if layout != HybridLayout::Mbr {
            let last = bytes.len() / 512 - 1;
            for offset in [512, last * 512] {
                let mut header = bytes[offset..offset + 92].to_vec();
                assert_eq!(&header[..8], b"EFI PART");
                let expected = number(&header[16..20]);
                header[16..20].fill(0);
                assert_eq!(crc32(&header), expected);
                let array_lba = u64::from_le_bytes(header[72..80].try_into().unwrap()) as usize;
                let array = &bytes[array_lba * 512..array_lba * 512 + 16384];
                assert_eq!(crc32(array), number(&header[88..92]));
                assert_eq!(
                    u64::from_le_bytes(array[32..40].try_into().unwrap()),
                    u64::from(efi) * 4
                );
            }
            if layout == HybridLayout::Gpt
                && let Some(command) = external("sgdisk", "SGDISK")
            {
                let result = Command::new(command)
                    .arg("-v")
                    .arg(&output)
                    .output()
                    .unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(
                    String::from_utf8_lossy(&result.stdout).contains("No problems found"),
                    "{}",
                    String::from_utf8_lossy(&result.stdout)
                );
            }
        }
        assert_eq!(read(&bytes, Namespace::Primary).entries().len(), 2);
        if let Some(command) = external("xorriso", "XORRISO") {
            let result = Command::new(command)
                .arg("-indev")
                .arg(&output)
                .args(["-report_el_torito", "plain", "-report_system_area", "plain"])
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let report = format!(
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(report.contains("El Torito boot img"), "{report}");
            if layout != HybridLayout::Mbr {
                assert!(report.contains("GPT"), "{report}");
            }
        }
    }
}

#[test]
fn invalid_profiles_and_resource_limits_do_not_publish() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"x").unwrap();
    for options in [
        IsoOptions {
            max_metadata_bytes: 1,
            ..Default::default()
        },
        IsoOptions {
            max_image_bytes: 1,
            ..Default::default()
        },
        IsoOptions {
            extent_bytes: 1,
            ..Default::default()
        },
        IsoOptions {
            joliet_level: 4,
            ..Default::default()
        },
        IsoOptions {
            hybrid: Some(HybridOptions {
                layout: HybridLayout::Gpt,
                ..Default::default()
            }),
            ..Default::default()
        },
        IsoOptions {
            advanced_boot: AdvancedBootOptions {
                entries: vec![BootEntry {
                    emulation: BootEmulation::Floppy1440,
                    ..BootEntry::bios("file")
                }],
                ..Default::default()
            },
            ..Default::default()
        },
    ] {
        let output = temp.path().join("failed.iso");
        assert!(write_iso9660_with_options(&source, &output, &options).is_err());
        assert!(!output.exists());
    }
}

#[test]
fn long_symbolic_links_use_chained_continuations_and_cycles_are_rejected() {
    use libmkiso::tree_source::{FileTreeSource, TreeEntry, TreeEntryKind, TreeInventory};
    struct Source(TreeInventory);
    impl FileTreeSource for Source {
        fn inventory(&self, entries: usize, bytes: usize) -> std::io::Result<TreeInventory> {
            self.0.validate_budget(entries, bytes)?;
            Ok(self.0.clone())
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let target = std::iter::repeat_n("x".repeat(200), 15)
        .collect::<Vec<_>>()
        .join("/");
    // The optical format supports targets longer than macOS host symlinks.
    let source = Source(TreeInventory {
        entries: vec![TreeEntry {
            path: "link".into(),
            native_name: b"link".to_vec(),
            metadata: Default::default(),
            object: 1,
            kind: TreeEntryKind::Symlink(target.clone()),
            streams: Vec::new(),
        }],
        ..Default::default()
    });
    let output = libmkiso::stage_iso9660_from_tree_source(
        &source,
        temp.path(),
        &IsoOptions {
            rock_ridge: true,
            ..Default::default()
        },
        || Ok(()),
    )
    .unwrap();
    let mut bytes = fs::read(output).unwrap();
    assert_eq!(
        read(&bytes, Namespace::RockRidge).entries()[0]
            .link_target
            .as_deref(),
        Some(target.as_str())
    );
    #[cfg(not(target_os = "macos"))] // Host extraction cannot create a 3 KiB macOS symlink.
    if let Some(command) = external("xorriso", "XORRISO") {
        let image = temp.path().join("independent-long.iso");
        fs::write(&image, &bytes).unwrap();
        let destination = temp.path().join("independent-long-link");
        let result = Command::new(command)
            .args(["-osirrox", "on", "-indev"])
            .arg(image)
            .args(["-extract", "/link"])
            .arg(&destination)
            .output()
            .unwrap();
        if result.status.success() {
            assert_eq!(
                fs::read_link(destination).unwrap(),
                std::path::Path::new(&target)
            );
        } else {
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains("Rock Ridge path too long"),
                "{diagnostic}"
            );
            eprintln!("SKIP long-link interoperability: this xorriso rejects 3 KiB targets");
        }
    }
    let root = number(&bytes[16 * 2048 + 158..16 * 2048 + 162]) as usize * 2048;
    let child =
        root + usize::from(bytes[root]) + usize::from(bytes[root + usize::from(bytes[root])]);
    let name = usize::from(bytes[child + 32]);
    let ce = child + 33 + name + usize::from(name.is_multiple_of(2));
    let block = number(&bytes[ce + 4..ce + 8]);
    let offset = number(&bytes[ce + 12..ce + 16]);
    let size = number(&bytes[ce + 20..ce + 24]);
    let chain = block as usize * 2048 + offset as usize + size as usize - 28;
    assert_eq!(&bytes[chain..chain + 2], b"CE");
    for (field, value) in [(4, block), (12, offset), (20, size)] {
        bytes[chain + field..chain + field + 4].copy_from_slice(&value.to_le_bytes());
        bytes[chain + field + 4..chain + field + 8].copy_from_slice(&value.to_be_bytes());
    }
    assert!(
        IsoReader::open_with_options(
            Cursor::new(&bytes),
            ReadOptions {
                namespace: Namespace::RockRidge,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn floppy_and_hard_disk_emulation_are_encoded_and_boot_catalog_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("floppy.img"), vec![0; 1440 * 1024]).unwrap();
    let mut disk = vec![0; 4096];
    disk[446 + 4] = 0x0b;
    disk[454..458].copy_from_slice(&1u32.to_le_bytes());
    disk[458..462].copy_from_slice(&7u32.to_le_bytes());
    disk[510..512].copy_from_slice(&[0x55, 0xaa]);
    fs::write(source.join("disk.img"), disk).unwrap();
    let mut floppy = BootEntry::bios("floppy.img");
    floppy.emulation = BootEmulation::Floppy1440;
    floppy.image.load_sectors = 1;
    let mut disk = BootEntry::bios("disk.img");
    disk.emulation = BootEmulation::HardDisk;
    disk.system_type = 0x0b;
    disk.image.load_sectors = 1;
    let output = temp.path().join("emulation.iso");
    write_iso9660_with_options(
        &source,
        &output,
        &IsoOptions {
            advanced_boot: AdvancedBootOptions {
                entries: vec![floppy.clone(), disk],
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let bytes = fs::read(output).unwrap();
    let catalog = number(&bytes[17 * 2048 + 71..17 * 2048 + 75]) as usize * 2048;
    assert_eq!(bytes[catalog + 33], 2);
    assert_eq!(bytes[catalog + 97], 4);
    assert_eq!(bytes[catalog + 100], 0x0b);
    let invalid = temp.path().join("too-many.iso");
    assert!(
        write_iso9660_with_options(
            &source,
            &invalid,
            &IsoOptions {
                advanced_boot: AdvancedBootOptions {
                    entries: vec![floppy; 32],
                    ..Default::default()
                },
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!invalid.exists());
}

#[cfg(unix)]
#[test]
fn independent_xorriso_rock_ridge_producer_is_readable() {
    use std::os::unix::fs::symlink;
    let Some(command) = external("xorriso", "XORRISO") else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("Mixed Name.txt"), b"independent").unwrap();
    symlink("../Mixed Name.txt", source.join("link")).unwrap();
    let output = temp.path().join("external.iso");
    let result = Command::new(command)
        .args(["-as", "mkisofs", "-R", "-J", "-o"])
        .arg(&output)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = fs::read(output).unwrap();
    let mut reader = read(&bytes, Namespace::RockRidge);
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "Mixed Name.txt")
        .unwrap();
    let mut payload = Vec::new();
    reader.extract(index, &mut payload).unwrap();
    assert_eq!(payload, b"independent");
    assert_eq!(
        reader
            .entries()
            .iter()
            .find(|entry| entry.name == "link")
            .unwrap()
            .link_target
            .as_deref(),
        Some("../Mixed Name.txt")
    );
}
