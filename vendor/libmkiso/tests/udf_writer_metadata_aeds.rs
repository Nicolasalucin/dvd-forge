#![cfg(feature = "native-writer")]
use libmkiso::udf::{Limits, UdfReader};
use libmkiso::{AllocationMode, UdfImage, UdfOptions, UdfPartition, UdfRevision};

#[test]
fn large_fragmented_metadata_files_use_aed_chains_and_recover_from_mirrors() {
    let mut tree = UdfImage::new();
    for index in 0..7600 {
        tree.add_bytes(format!("file{index:05}"), Vec::new())
            .unwrap();
    }
    let temporary = tempfile::tempdir().unwrap();
    let output = temporary.path().join("metadata.udf");
    let options = UdfOptions {
        revision: UdfRevision::V250,
        partition: UdfPartition::Metadata { mirror: true },
        allocation: AllocationMode::Long,
        metadata_extent_blocks: 32,
        ..UdfOptions::default()
    };
    tree.write(&output, &options).unwrap();
    let mut bytes = std::fs::read(output).unwrap();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    assert_eq!(reader.entries().len(), 7600);
    assert!(reader.entries().iter().all(|entry| entry.size == 0));
    let block = 2048;
    // First metadata fragment starts at physical partition block 32; its gap
    // starts at block 64 and carries the first allocation-extent descriptor.
    let aed = (320 + 64) * block;
    assert_eq!(&bytes[aed..aed + 2], &258u16.to_le_bytes());
    drop(reader);
    bytes[aed + 8] ^= 1;
    assert_eq!(
        UdfReader::open(&bytes, Limits::default())
            .unwrap()
            .entries()
            .len(),
        7600
    );
}
