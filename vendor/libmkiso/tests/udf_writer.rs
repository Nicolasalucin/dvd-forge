#![cfg(feature = "native-writer")]
use libmkiso::{
    AllocationMode, IcbStrategy, UdfFileExtent, UdfImage, UdfOptions, UdfPartition, UdfRevision,
    udf::{EntryKind, Limits, UdfReader},
};
fn revisions() -> [UdfRevision; 6] {
    [
        UdfRevision::V102,
        UdfRevision::V150,
        UdfRevision::V200,
        UdfRevision::V201,
        UdfRevision::V250,
        UdfRevision::V260,
    ]
}
#[test]
fn all_revisions_round_trip_recorded_embedded_sparse_and_continuation_files() {
    let dir = tempfile::tempdir().unwrap();
    for revision in revisions() {
        for allocation in [
            AllocationMode::Short,
            AllocationMode::Long,
            AllocationMode::Extended,
            AllocationMode::Embedded,
        ] {
            let mut image = UdfImage::new();
            image
                .add_bytes("nested/日本語.bin", vec![42; 4097])
                .unwrap();
            image.add_bytes("small", b"small".to_vec()).unwrap();
            image.add_bytes("empty", vec![]).unwrap();
            let options = UdfOptions {
                revision,
                allocation,
                extent_blocks: 1,
                ..Default::default()
            };
            let path = dir
                .path()
                .join(format!("{:?}-{:?}.udf", revision, allocation));
            image.write(&path, &options).unwrap();
            let bytes = std::fs::read(path).unwrap();
            let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
            for entry in reader.entries() {
                if entry.kind == EntryKind::File {
                    let index = reader
                        .entries()
                        .iter()
                        .position(|e| e.name == entry.name)
                        .unwrap();
                    let contents = reader.read_entry(index, 10000).unwrap();
                    match entry.name.as_str() {
                        "nested/日本語.bin" => assert_eq!(contents, vec![42; 4097]),
                        "small" => assert_eq!(contents, b"small"),
                        "empty" => assert!(contents.is_empty()),
                        _ => panic!("unexpected entry"),
                    }
                }
            }
        }
    }
}
#[test]
fn populated_virtual_sparable_and_metadata_maps_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    for (revision, partition, allocation) in [
        (
            UdfRevision::V150,
            UdfPartition::Virtual,
            AllocationMode::Long,
        ),
        (
            UdfRevision::V201,
            UdfPartition::Virtual,
            AllocationMode::Long,
        ),
        (
            UdfRevision::V201,
            UdfPartition::Sparable { packet_blocks: 32 },
            AllocationMode::Short,
        ),
        (
            UdfRevision::V250,
            UdfPartition::Metadata { mirror: false },
            AllocationMode::Long,
        ),
        (
            UdfRevision::V260,
            UdfPartition::Metadata { mirror: true },
            AllocationMode::Long,
        ),
    ] {
        let mut image = UdfImage::new();
        image.add_bytes("nested/data", vec![7; 8193]).unwrap();
        let options = UdfOptions {
            revision,
            partition,
            allocation,
            extent_blocks: 1,
            ..Default::default()
        };
        let path = dir
            .path()
            .join(format!("{:?}-{:?}.udf", revision, partition).replace(':', "_"));
        image.write(&path, &options).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let idx = reader
            .entries()
            .iter()
            .position(|e| e.name == "nested/data")
            .unwrap();
        assert_eq!(reader.read_entry(idx, 10000).unwrap(), vec![7; 8193]);
    }
}
#[test]
fn streams_links_sparse_preallocated_tails_and_aeds_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    for strategy in [
        IcbStrategy::Direct,
        IcbStrategy::Indirect,
        IcbStrategy::Strategy4096,
    ] {
        let mut image = UdfImage::new();
        image.add_bytes("file", vec![9; 300 * 2048 + 3]).unwrap();
        image.add_hard_link("alias", "file").unwrap();
        image.add_symlink("link", "../file").unwrap();
        image
            .add_named_stream("file", "metadata", b"named".to_vec())
            .unwrap();
        image
            .add_system_stream("system", b"system".to_vec())
            .unwrap();
        image
            .add_sparse_file(
                "sparse",
                vec![
                    UdfFileExtent::Data(vec![1; 2048]),
                    UdfFileExtent::Hole(4096),
                    UdfFileExtent::Data(vec![2; 3]),
                ],
            )
            .unwrap();
        image.set_preallocated_blocks("sparse", 3).unwrap();
        let options = UdfOptions {
            revision: UdfRevision::V201,
            allocation: AllocationMode::Long,
            icb_strategy: strategy,
            extent_blocks: 1,
            ..Default::default()
        };
        let path = dir.path().join(format!("{:?}.udf", strategy));
        image.write(&path, &options).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let find = |name: &str| {
            reader
                .entries()
                .iter()
                .position(|e| e.name == name)
                .unwrap()
        };
        assert_eq!(
            reader.entries()[find("file")].icb,
            reader.entries()[find("alias")].icb
        );
        assert_eq!(
            reader.entries()[find("link")].link_target.as_deref(),
            Some("../file")
        );
        let named = reader
            .entries()
            .iter()
            .position(|e| e.kind == EntryKind::NamedStream)
            .unwrap();
        assert_eq!(reader.read_entry(named, 10000).unwrap(), b"named");
        let system = reader
            .entries()
            .iter()
            .position(|e| e.kind == EntryKind::SystemStream)
            .unwrap();
        assert_eq!(reader.read_entry(system, 10000).unwrap(), b"system");
        let mut expected = vec![1; 2048];
        expected.extend(vec![0; 4096]);
        expected.extend([2; 3]);
        assert_eq!(reader.read_entry(find("sparse"), 10000).unwrap(), expected);
    }
}

#[test]
fn allocated_sparse_body_and_preallocation_have_distinct_extended_information_lengths() {
    let directory = tempfile::tempdir().unwrap();
    for allocation in [
        AllocationMode::Short,
        AllocationMode::Long,
        AllocationMode::Extended,
    ] {
        let mut image = UdfImage::new();
        image
            .add_sparse_file(
                "data",
                vec![
                    UdfFileExtent::Data(vec![1; 2048]),
                    UdfFileExtent::AllocatedHole(4096),
                    UdfFileExtent::Hole(2048),
                    UdfFileExtent::Data(vec![3; 7]),
                ],
            )
            .unwrap();
        image.set_preallocated_blocks("data", 400).unwrap();
        let path = directory.path().join(format!("{:?}.udf", allocation));
        image
            .write(
                &path,
                &UdfOptions {
                    revision: UdfRevision::V201,
                    allocation,
                    extent_blocks: 1,
                    ..Default::default()
                },
            )
            .unwrap();
        let bytes = std::fs::read(path).unwrap();
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let mut expected = vec![1; 2048];
        expected.extend(vec![0; 6144]);
        expected.extend([3; 7]);
        assert_eq!(reader.read_entry(0, 10000).unwrap(), expected);
        let mut classes = Vec::new();
        reader
            .visit_classified_extents(0, |logical, kind, length| {
                classes.push((logical, kind, length));
                Ok(())
            })
            .unwrap();
        assert!(classes.iter().any(|&(logical, kind, _)| logical == 2048
            && kind == libmkiso::UdfExtentKind::AllocatedUnrecorded));
        assert!(
            classes.iter().any(|&(logical, kind, _)| logical == 6144
                && kind == libmkiso::UdfExtentKind::Unallocated)
        );
        assert_eq!(
            classes.iter().map(|(_, _, length)| length).sum::<u64>(),
            8199
        );
    }
}
#[test]
fn streams_sparse_links_and_file_set_chains_work_across_partition_profiles() {
    let directory = tempfile::tempdir().unwrap();
    for partition in [
        UdfPartition::Physical,
        UdfPartition::PhysicalSplit,
        UdfPartition::Virtual,
        UdfPartition::Sparable { packet_blocks: 32 },
        UdfPartition::Metadata { mirror: false },
        UdfPartition::Metadata { mirror: true },
        UdfPartition::MetadataSparable {
            mirror: true,
            packet_blocks: 32,
        },
    ] {
        let revision = if matches!(
            partition,
            UdfPartition::Metadata { .. } | UdfPartition::MetadataSparable { .. }
        ) {
            UdfRevision::V260
        } else {
            UdfRevision::V201
        };
        let mut image = UdfImage::new();
        image
            .add_sparse_file(
                "dir/main",
                vec![
                    UdfFileExtent::Data(vec![6; 2048]),
                    UdfFileExtent::AllocatedHole(2048),
                    UdfFileExtent::Hole(2048),
                    UdfFileExtent::Data(vec![7; 3]),
                ],
            )
            .unwrap();
        image.add_hard_link("alias", "dir/main").unwrap();
        image.add_symlink("symlink", "dir/main").unwrap();
        image
            .add_named_stream("dir/main", "stream", b"stream".to_vec())
            .unwrap();
        image
            .add_system_stream("system", b"system".to_vec())
            .unwrap();
        let path = directory
            .path()
            .join(format!("{:?}.udf", partition).replace(':', "_"));
        let options = UdfOptions {
            revision,
            partition,
            allocation: AllocationMode::Long,
            file_set_descriptors: if matches!(
                partition,
                UdfPartition::Physical | UdfPartition::PhysicalSplit
            ) {
                3
            } else {
                1
            },
            icb_strategy: IcbStrategy::Indirect,
            extent_blocks: 1,
            metadata_extent_blocks: if revision == UdfRevision::V260 { 32 } else { 0 },
            ..Default::default()
        };
        image.write(&path, &options).unwrap();
        let bytes = std::fs::read(path).unwrap();
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let index = reader
            .entries()
            .iter()
            .position(|e| e.name == "dir/main")
            .unwrap();
        let mut expected = vec![6; 2048];
        expected.extend(vec![0; 4096]);
        expected.extend([7; 3]);
        assert_eq!(reader.read_entry(index, 10000).unwrap(), expected);
        let mut classes = Vec::new();
        reader
            .visit_classified_extents(index, |logical, kind, length| {
                classes.push((logical, kind, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            classes[1],
            (2048, libmkiso::UdfExtentKind::AllocatedUnrecorded, 2048)
        );
        assert_eq!(
            classes[2],
            (4096, libmkiso::UdfExtentKind::Unallocated, 2048)
        );

        assert!(
            reader
                .entries()
                .iter()
                .any(|e| e.kind == EntryKind::NamedStream)
        );
        assert!(
            reader
                .entries()
                .iter()
                .any(|e| e.kind == EntryKind::SystemStream)
        );
    }
}

#[test]
fn stream_unique_ids_flags_and_file_link_counts_follow_owner_relationships() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("main", b"main".to_vec()).unwrap();
    image.add_hard_link("alias", "main").unwrap();
    image.add_directory("directory/nested").unwrap();
    image
        .add_named_stream("main", "named", b"named".to_vec())
        .unwrap();
    image.add_root_stream("root", b"root".to_vec()).unwrap();
    image
        .add_system_stream("system", b"system".to_vec())
        .unwrap();
    assert!(image.add_system_stream("*UDF Forbidden", vec![]).is_err());
    let output = directory.path().join("streams.udf");
    image
        .write(
            &output,
            &UdfOptions {
                revision: UdfRevision::V201,
                allocation: AllocationMode::Long,
                ..Default::default()
            },
        )
        .unwrap();
    let bytes = std::fs::read(output).unwrap();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let descriptor = |index: usize| {
        let block = reader.entries()[index].icb.block as usize + 320;
        &bytes[block * 2048..(block + 1) * 2048]
    };
    let integer16 =
        |b: &[u8], offset: usize| u16::from_le_bytes(b[offset..offset + 2].try_into().unwrap());
    let integer64 =
        |b: &[u8], offset: usize| u64::from_le_bytes(b[offset..offset + 8].try_into().unwrap());
    let owner = reader
        .entries()
        .iter()
        .position(|e| e.name == "main")
        .unwrap();
    let uid = integer64(descriptor(owner), 200);
    assert_eq!(integer16(descriptor(owner), 48), 3);
    let dir = reader
        .entries()
        .iter()
        .position(|e| e.name == "directory")
        .unwrap();
    assert_eq!(integer16(descriptor(dir), 48), 2);
    for (index, entry) in reader.entries().iter().enumerate() {
        if let Some(stream) = &entry.stream {
            assert_ne!(integer16(descriptor(index), 34) & (1 << 13), 0);
            let expected = if stream.system || stream.owner.is_none() {
                0
            } else {
                uid
            };
            assert_eq!(integer64(descriptor(index), 200), expected);
        }
    }
    assert!(reader.entries().iter().any(|entry| {
        entry
            .stream
            .as_ref()
            .is_some_and(|stream| !stream.system && stream.owner.is_none())
    }));
}

#[cfg(unix)]
#[test]
fn directory_scanning_rejects_literal_backslash_names_and_symlink_targets() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("literal\\filename"), b"data").unwrap();
    let output = directory.path().join("image.udf");
    assert!(libmkiso::write_udf(&source, &output).is_err());
    assert!(!output.exists());
    std::fs::remove_file(source.join("literal\\filename")).unwrap();
    std::os::unix::fs::symlink("literal\\target", source.join("link")).unwrap();
    assert!(libmkiso::write_udf(&source, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn hard_linked_boot_image_stays_recorded_and_precedes_large_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("aaa-large", vec![9; 1024 * 1024]).unwrap();
    image.add_bytes("loader", vec![5; 512]).unwrap();
    image.add_hard_link("zz-alias", "loader").unwrap();
    let mut boot = libmkiso::BootImage::bios("zz-alias");
    boot.load_sectors = 1;
    let options = UdfOptions {
        revision: UdfRevision::V201,
        allocation: AllocationMode::Embedded,
        boot: libmkiso::BootOptions {
            bios: Some(boot),
            efi: None,
        },
        ..Default::default()
    };
    let output = directory.path().join("boot.udf");
    image.write(&output, &options).unwrap();
    let bytes = std::fs::read(output).unwrap();
    let catalog = &bytes[35 * 2048..36 * 2048];
    let boot_block = u32::from_le_bytes(catalog[40..44].try_into().unwrap()) as usize;
    assert_eq!(
        &bytes[boot_block * 2048..boot_block * 2048 + 512],
        &[5; 512]
    );
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let large = reader
        .entries()
        .iter()
        .find(|e| e.name == "aaa-large")
        .unwrap();
    let large_entry = &bytes[(large.icb.block as usize + 320) * 2048..];
    let large_block = 320 + u32::from_le_bytes(large_entry[220..224].try_into().unwrap()) as usize;
    assert!(boot_block < large_block);
}
