#![cfg(feature = "native-writer")]
use libmkiso::udf::{Limits, UdfReader};

fn fixture() -> (tempfile::TempDir, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::create_dir_all(source.join("boot")).unwrap();
    std::fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
    std::fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
    std::fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
    std::fs::write(source.join("payload.txt"), b"UDF payload").unwrap();
    std::fs::write(source.join("日本語.txt"), b"Unicode").unwrap();
    std::fs::write(source.join("empty"), []).unwrap();
    for index in 0..100 {
        std::fs::write(
            source.join(format!("file-{index:03}.txt")),
            [index as u8; 17],
        )
        .unwrap();
    }
    let output = directory.path().join("media.iso");
    libmkiso::write_iso(&source, &output).unwrap();
    let bytes = std::fs::read(output).unwrap();
    (directory, bytes)
}

#[test]
fn udf_writer_payload_is_read_by_portable_reader_and_7z() {
    let (directory, bytes) = fixture();
    let archive = UdfReader::open(&bytes, Limits::default()).unwrap();
    let file = archive
        .entries()
        .iter()
        .find(|file| file.name == "payload.txt")
        .unwrap();
    assert_eq!(
        archive
            .read_entry(
                archive
                    .entries()
                    .iter()
                    .position(|entry| entry.name == file.name)
                    .unwrap(),
                100
            )
            .unwrap(),
        b"UDF payload"
    );
    for (index, entry) in archive
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.directory)
    {
        assert_eq!(
            archive.read_entry(index, 8192).unwrap(),
            std::fs::read(directory.path().join("source").join(&entry.name)).unwrap()
        );
    }
    if std::process::Command::new("7z").arg("i").output().is_ok() {
        let output = std::process::Command::new("7z")
            .args(["e", "-so"])
            .arg(directory.path().join("media.iso"))
            .arg("payload.txt")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"UDF payload");
    }
}

#[test]
fn udf_descriptor_corruption_and_metadata_budget_are_rejected() {
    let (_, mut bytes) = fixture();
    let limits = Limits {
        max_metadata_bytes: 1,
        ..Limits::default()
    };
    assert!(UdfReader::open(&bytes, limits).is_err());
    let last = bytes.len() / 2048 - 1;
    for block in [256, last, last - 256] {
        bytes[block * 2048 + 20] ^= 1;
    }
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn truncated_images_and_payload_allocation_limits_are_rejected() {
    let (_, bytes) = fixture();
    let partition_blocks = u32::from_le_bytes(
        bytes[259 * 2048 + 192..259 * 2048 + 196]
            .try_into()
            .unwrap(),
    ) as usize;
    for length in [0, 16, 256 * 2048, (320 + partition_blocks) * 2048 - 1] {
        assert!(
            UdfReader::open(&bytes[..length], Limits::default()).is_err(),
            "length {length}"
        );
    }
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "payload.txt")
        .unwrap();
    assert!(reader.read_entry(index, 1).is_err());
    assert!(reader.read_entry(usize::MAX, 100).is_err());
    for limits in [
        Limits {
            max_entries: 1,
            ..Limits::default()
        },
        Limits {
            max_entry_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_total_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_input_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_nesting_depth: 0,
            ..Limits::default()
        },
    ] {
        assert!(UdfReader::open(&bytes, limits).is_err());
    }
}

fn retag(block: &mut [u8]) {
    let length = u16::from_le_bytes([block[10], block[11]]) as usize;
    let mut crc = 0u16;
    for &byte in &block[16..16 + length] {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    block[8..10].copy_from_slice(&crc.to_le_bytes());
    block[4] = block[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
}

#[test]
fn valid_checksums_do_not_allow_invalid_extents_or_profiles() {
    let (_, bytes) = fixture();
    // Root file entry is partition-relative block 1 in the writer fixture.
    for (offset, value) in [(180, u32::MAX), (176, 0x4000_0000), (172, 7)] {
        let mut corrupt = bytes.clone();
        let root = &mut corrupt[321 * 2048..322 * 2048];
        root[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        retag(root);
        assert!(UdfReader::open(&corrupt, Limits::default()).is_err());
    }
    let mut corrupt = bytes.clone();
    let root = &mut corrupt[321 * 2048..322 * 2048];
    root[34] = 1; // Long descriptors are outside the baseline profile.
    retag(root);
    assert!(UdfReader::open(&corrupt, Limits::default()).is_err());
}
