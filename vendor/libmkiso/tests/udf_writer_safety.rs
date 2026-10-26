#![cfg(feature = "native-writer")]
use libmkiso::udf::{Limits, UdfReader};
use libmkiso::{
    AllocationMode, BootImage, BootOptions, UdfImage, UdfOptions, UdfPartition, UdfRevision,
};

#[test]
fn ambiguous_paths_and_duplicate_names_are_rejected() {
    for path in ["", "/file", "a//b", "a/./b", "a/../b", "a\\b", "a\0b", "a/"] {
        assert!(
            UdfImage::new().add_bytes(path, vec![1]).is_err(),
            "{path:?}"
        );
    }
    let mut image = UdfImage::new();
    image.add_bytes("file", vec![1]).unwrap();
    assert!(image.add_bytes("file/child", vec![2]).is_err());
    assert!(image.add_directory("file").is_err());
    image.add_named_stream("file", "stream", vec![3]).unwrap();
    assert!(image.add_named_stream("file", "stream", vec![4]).is_err());
}

#[test]
fn cancellation_and_existing_output_preserve_published_files() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("image.udf");
    let mut image = UdfImage::new();
    image.add_bytes("file", vec![7; 131072]).unwrap();
    let mut calls = 0;
    let result = image.write_with_cancel(&output, &UdfOptions::default(), || {
        calls += 1;
        if calls == 5 {
            anyhow::bail!("cancelled")
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    std::fs::write(&output, b"preserve").unwrap();
    assert!(image.write(&output, &UdfOptions::default()).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"preserve");
}

#[test]
fn invalid_revision_and_partition_combinations_publish_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let image = UdfImage::new();
    for partition in [
        UdfPartition::Virtual,
        UdfPartition::Sparable { packet_blocks: 32 },
        UdfPartition::Metadata { mirror: true },
    ] {
        let options = UdfOptions {
            partition,
            ..Default::default()
        };
        assert!(
            image
                .write(&directory.path().join("invalid.udf"), &options)
                .is_err()
        );
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn writer_resource_limits_reject_before_publication() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("nested/file", vec![1; 8192]).unwrap();
    for options in [
        UdfOptions {
            max_entry_bytes: 8191,
            ..Default::default()
        },
        UdfOptions {
            max_total_bytes: 8191,
            ..Default::default()
        },
        UdfOptions {
            max_image_bytes: 2048,
            ..Default::default()
        },
        UdfOptions {
            max_metadata_bytes: 1,
            ..Default::default()
        },
        UdfOptions {
            max_entries: 1,
            ..Default::default()
        },
        UdfOptions {
            max_nesting_depth: 1,
            ..Default::default()
        },
    ] {
        assert!(
            image
                .write(&directory.path().join("limited.udf"), &options)
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}

#[test]
fn boot_catalog_addresses_exact_original_payloads_across_partition_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    let bios = vec![0x31; 4096];
    let efi = vec![0x72; 8192];
    image.add_bytes("boot/bios.bin", bios.clone()).unwrap();
    image.add_bytes("boot/efi.bin", efi.clone()).unwrap();
    for (index, partition) in [
        UdfPartition::Physical,
        UdfPartition::Virtual,
        UdfPartition::Sparable { packet_blocks: 32 },
        UdfPartition::Metadata { mirror: true },
    ]
    .into_iter()
    .enumerate()
    {
        let output = directory.path().join(format!("boot-{index}.udf"));
        let options = UdfOptions {
            partition,
            revision: if matches!(partition, UdfPartition::Metadata { .. }) {
                UdfRevision::V260
            } else {
                UdfRevision::V201
            },
            allocation: AllocationMode::Embedded,
            boot: BootOptions {
                bios: Some(BootImage::bios("boot/bios.bin")),
                efi: Some(BootImage::efi("boot/efi.bin")),
            },
            ..Default::default()
        };
        image.write(&output, &options).unwrap();
        let bytes = std::fs::read(&output).unwrap();
        let catalog_block =
            u32::from_le_bytes(bytes[17 * 2048 + 71..17 * 2048 + 75].try_into().unwrap()) as usize;
        let catalog = &bytes[catalog_block * 2048..(catalog_block + 1) * 2048];
        for (offset, expected) in [(40, &bios), (104, &efi)] {
            let block =
                u32::from_le_bytes(catalog[offset..offset + 4].try_into().unwrap()) as usize;
            assert_eq!(
                &bytes[block * 2048..block * 2048 + expected.len()],
                expected
            );
        }
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let index = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "boot/efi.bin")
            .unwrap();
        assert_eq!(reader.read_entry(index, 8192).unwrap(), efi);
        let iso = libmkiso::iso9660::IsoReader::open(
            std::fs::File::open(output).unwrap(),
            libmkiso::iso9660::Limits::default(),
        )
        .unwrap();
        assert!(iso.entries().is_empty());
    }
}

#[test]
fn fixed_options_produce_identical_images_and_hashes() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("日本語/file", vec![0xf1; 4097]).unwrap();
    image
        .add_named_stream("日本語/file", "note", b"metadata".to_vec())
        .unwrap();
    for partition in [
        UdfPartition::Physical,
        UdfPartition::Metadata { mirror: true },
    ] {
        let options = UdfOptions {
            revision: UdfRevision::V260,
            partition,
            allocation: AllocationMode::Long,
            label: "MEDIA_日本語".into(),
            timestamp: libmkiso::IsoTimestamp {
                year: 2024,
                month: 2,
                day: 29,
                hour: 1,
                minute: 2,
                second: 3,
            },
            ..Default::default()
        };
        let first = directory
            .path()
            .join(format!("first-{partition:?}.udf").replace(':', "_"));
        let second = directory
            .path()
            .join(format!("second-{partition:?}.udf").replace(':', "_"));
        let first_hash = image.write(&first, &options).unwrap();
        let second_hash = image.write(&second, &options).unwrap();
        assert_eq!(first_hash, second_hash);
        let bytes = std::fs::read(first).unwrap();
        assert_eq!(bytes, std::fs::read(second).unwrap());
        assert_eq!(
            u16::from_le_bytes(bytes[272 * 2048 + 18..272 * 2048 + 20].try_into().unwrap()),
            2024
        );
    }
}
