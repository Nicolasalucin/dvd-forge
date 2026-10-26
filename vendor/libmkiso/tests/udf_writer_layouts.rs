#![cfg(feature = "native-writer")]
use libmkiso::udf::{EntryKind, Limits, UdfReader};
use libmkiso::{AllocationMode, UdfImage, UdfOptions, UdfPartition, UdfRevision};

#[test]
fn split_physical_partitions_keep_payload_and_continuation_addresses_distinct() {
    let directory = tempfile::tempdir().unwrap();
    let payload = vec![0xa7; 600 * 2048 + 31];
    let mut image = UdfImage::new();
    image.add_bytes("nested/file", payload.clone()).unwrap();
    image.add_hard_link("alias", "nested/file").unwrap();
    for revision in [
        UdfRevision::V102,
        UdfRevision::V150,
        UdfRevision::V200,
        UdfRevision::V201,
        UdfRevision::V250,
        UdfRevision::V260,
    ] {
        for allocation in [
            AllocationMode::Long,
            AllocationMode::Extended,
            AllocationMode::Embedded,
        ] {
            let output = directory
                .path()
                .join(format!("split-{:x}-{allocation:?}.udf", revision.number()));
            let options = UdfOptions {
                revision,
                allocation,
                partition: UdfPartition::PhysicalSplit,
                extent_blocks: 1,
                ..Default::default()
            };
            image.write(&output, &options).unwrap();
            let bytes = std::fs::read(output).unwrap();
            let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
            let file = reader
                .entries()
                .iter()
                .position(|entry| entry.name == "nested/file")
                .unwrap();
            let alias = reader
                .entries()
                .iter()
                .position(|entry| entry.name == "alias")
                .unwrap();
            assert_eq!(
                reader.read_entry(file, payload.len() as u64).unwrap(),
                payload
            );
            assert_eq!(reader.entries()[file].icb, reader.entries()[alias].icb);
            assert_eq!(
                u32::from_le_bytes(
                    bytes[261 * 2048 + 268..261 * 2048 + 272]
                        .try_into()
                        .unwrap()
                ),
                2
            );
        }
    }
}

#[test]
fn fragmented_metadata_maps_cross_extent_boundaries_and_recover_duplicate_mirror() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    for index in 0..80 {
        image
            .add_bytes(
                format!("directory/file-{index:03}"),
                vec![index as u8; 2051],
            )
            .unwrap();
    }
    for revision in [UdfRevision::V250, UdfRevision::V260] {
        for partition in [
            UdfPartition::Metadata { mirror: false },
            UdfPartition::Metadata { mirror: true },
            UdfPartition::MetadataSparable {
                mirror: true,
                packet_blocks: 32,
            },
        ] {
            let output = directory
                .path()
                .join(format!("fragment-{revision:?}-{partition:?}.udf").replace(':', "_"));
            let options = UdfOptions {
                revision,
                partition,
                allocation: AllocationMode::Long,
                metadata_extent_blocks: 32,
                ..Default::default()
            };
            image.write(&output, &options).unwrap();
            let mut bytes = std::fs::read(output).unwrap();
            let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
            assert_eq!(
                reader
                    .entries()
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::File)
                    .count(),
                80
            );
            let file = reader
                .entries()
                .iter()
                .position(|entry| entry.name == "directory/file-079")
                .unwrap();
            assert_eq!(reader.read_entry(file, 2051).unwrap(), [79; 2051]);
            if matches!(
                partition,
                UdfPartition::Metadata { mirror: true }
                    | UdfPartition::MetadataSparable { mirror: true, .. }
            ) {
                // FSD is the first logical metadata block, at primary physical block 32.
                drop(reader);
                bytes[(320 + 32) * 2048 + 112] ^= 1;
                let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
                let file = reader
                    .entries()
                    .iter()
                    .position(|entry| entry.name == "directory/file-079")
                    .unwrap();
                assert_eq!(reader.read_entry(file, 2051).unwrap(), [79; 2051]);
            }
        }
    }
}

#[test]
fn hard_link_names_share_one_indexed_named_stream_directory() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("file", b"main".to_vec()).unwrap();
    image.add_hard_link("alias", "file").unwrap();
    image
        .add_named_stream("file", "note", b"stream".to_vec())
        .unwrap();
    let output = directory.path().join("linked-stream.udf");
    image
        .write(
            &output,
            &UdfOptions {
                revision: UdfRevision::V201,
                ..Default::default()
            },
        )
        .unwrap();
    let bytes = std::fs::read(output).unwrap();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let streams: Vec<_> = reader
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.stream.is_some())
        .collect();
    assert_eq!(streams.len(), 1);
    assert_eq!(reader.read_entry(streams[0].0, 6).unwrap(), b"stream");
    let owner = streams[0].1.stream.as_ref().unwrap().owner.unwrap();
    assert_eq!(reader.entries()[owner].name, "alias");
}

#[test]
fn authored_sparing_replacements_survive_loss_of_original_packets() {
    let directory = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("file", vec![0x98; 8193]).unwrap();
    for partition in [
        UdfPartition::Sparable { packet_blocks: 32 },
        UdfPartition::MetadataSparable {
            mirror: true,
            packet_blocks: 32,
        },
    ] {
        let output = directory
            .path()
            .join(format!("spared-{partition:?}.udf").replace(':', "_"));
        let options = UdfOptions {
            revision: if matches!(partition, UdfPartition::MetadataSparable { .. }) {
                UdfRevision::V260
            } else {
                UdfRevision::V201
            },
            partition,
            allocation: AllocationMode::Long,
            sparing_packets: vec![0, 32],
            ..Default::default()
        };
        image.write(&output, &options).unwrap();
        let mut bytes = std::fs::read(output).unwrap();
        bytes[320 * 2048..384 * 2048].fill(0);
        let reader = UdfReader::open(&bytes, Limits::default())
            .unwrap_or_else(|error| panic!("{partition:?}: {error}"));
        let file = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "file")
            .unwrap();
        assert_eq!(reader.read_entry(file, 8193).unwrap(), [0x98; 8193]);
    }
}
