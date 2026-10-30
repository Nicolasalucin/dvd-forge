#![cfg(feature = "native-writer")]
use libmkiso::{
    AllocationMode, UdfFileExtent, UdfImage, UdfOptions, UdfPartition, UdfRevision,
    udf::{EntryKind, Limits, UdfReader},
};

const BLOCK: u64 = 2048;
const GIB: u64 = 1024 * BLOCK * 512;

#[test]
fn sparse_udf_250_file_over_four_gib_round_trips_without_buffering_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    let head = vec![0x41; BLOCK as usize];
    let tail = b"DVD Forge large extent tail".to_vec();
    image
        .add_sparse_file(
            "movies/huge.m2ts",
            vec![
                UdfFileExtent::Data(head.clone()),
                UdfFileExtent::Hole(4 * GIB),
                UdfFileExtent::AllocatedHole(BLOCK),
                UdfFileExtent::Data(tail.clone()),
            ],
        )
        .unwrap();
    let options = UdfOptions {
        revision: UdfRevision::V250,
        partition: UdfPartition::Metadata { mirror: true },
        allocation: AllocationMode::Long,
        extent_blocks: 16,
        ..Default::default()
    };
    let path = dir.path().join("large.udf");
    image.write(&path, &options).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "movies/huge.m2ts")
        .expect("large file entry");
    let entry = &reader.entries()[index];
    assert_eq!(entry.kind, EntryKind::File);
    assert_eq!(
        entry.size,
        head.len() as u64 + 4 * GIB + BLOCK + tail.len() as u64
    );

    let mut prefix = vec![0u8; head.len()];
    assert_eq!(reader.read_at(index, 0, &mut prefix).unwrap(), head.len());
    assert_eq!(prefix, head);

    let mut suffix = vec![0u8; tail.len()];
    let tail_offset = entry.size - tail.len() as u64;
    assert_eq!(
        reader.read_at(index, tail_offset, &mut suffix).unwrap(),
        tail.len()
    );
    assert_eq!(suffix, tail);

    // The middle remains a hole and must read as zeroes, not as host garbage.
    let mut middle = [0xffu8; 64];
    assert_eq!(
        reader.read_at(index, 2 * GIB, &mut middle).unwrap(),
        middle.len()
    );
    assert!(middle.iter().all(|byte| *byte == 0));
}

#[test]
fn udf_250_writes_and_validates_extended_file_entry_tag_266() {
    let dir = tempfile::tempdir().unwrap();
    let mut image = UdfImage::new();
    image.add_bytes("extended.bin", vec![0x5a; 8193]).unwrap();
    let options = UdfOptions {
        revision: UdfRevision::V250,
        partition: UdfPartition::Physical,
        allocation: AllocationMode::Extended,
        extent_blocks: 1,
        ..Default::default()
    };
    let path = dir.path().join("tag266.udf");
    image.write(&path, &options).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut seen = 0usize;
    let mut nonzero_information = 0usize;
    for block in bytes.chunks_exact(BLOCK as usize) {
        if u16::from_le_bytes([block[0], block[1]]) != 266 {
            continue;
        }
        seen += 1;
        let length = u16::from_le_bytes([block[10], block[11]]) as usize;
        assert!(length >= 208, "Tag 266 descriptor too short: {length}");
        let end = 16 + length;
        assert!(end <= block.len(), "Tag 266 descriptor exceeds block");
        let checksum = block[..16]
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 4)
            .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
        assert_eq!(checksum, block[4], "Tag 266 checksum must be valid");
        let information_length = u64::from_le_bytes([
            block[56], block[57], block[58], block[59], block[60], block[61], block[62], block[63],
        ]);
        if information_length > 0 {
            nonzero_information += 1;
            assert!(information_length <= 8193);
        }
    }
    assert!(seen > 0, "UDF 2.50 image should contain Tag 266 EFE");
    assert!(
        nonzero_information > 0,
        "at least one Tag 266 EFE must carry a non-zero information length"
    );
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "extended.bin")
        .expect("extended entry");
    assert_eq!(reader.read_entry(index, 16_384).unwrap().len(), 8193);
}
