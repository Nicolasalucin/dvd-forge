#![cfg(feature = "native-writer")]
use libmkiso::udf::{Limits, UdfReader};
const BLOCK: usize = 2048;

fn retag(bytes: &mut [u8], protected: usize) {
    bytes[10..12].copy_from_slice(&(protected as u16).to_le_bytes());
    let mut crc = 0u16;
    for byte in &bytes[16..16 + protected] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    bytes[8..10].copy_from_slice(&crc.to_le_bytes());
    bytes[4] = bytes[..16]
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn get32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn fixture() -> (Vec<u8>, usize, u32) {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    std::fs::create_dir_all(source.join("boot")).unwrap();
    std::fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
    std::fs::write(source.join("boot/etfsboot.com"), [1; 4096]).unwrap();
    std::fs::write(source.join("efi/microsoft/boot/efisys.bin"), [2; 4096]).unwrap();
    std::fs::write(source.join("payload.txt"), b"hello world").unwrap();
    let image = tmp.path().join("image.iso");
    libmkiso::write_iso(&source, &image).unwrap();
    let bytes = std::fs::read(image).unwrap();
    let entry = bytes
        .chunks_exact(BLOCK)
        .enumerate()
        .find(|(_, block)| {
            block[0..2] == 261u16.to_le_bytes()
                && block[27] == 5
                && block[56..64] == 11u64.to_le_bytes()
        })
        .unwrap()
        .0
        * BLOCK;
    let payload = get32(&bytes, entry + 180);
    (bytes, entry, payload)
}
fn read(bytes: &[u8]) -> Vec<u8> {
    let reader = UdfReader::open(bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "payload.txt")
        .unwrap();
    reader.read_entry(index, 8192).unwrap()
}

#[test]
fn allocated_and_unallocated_sparse_extents_extract_zeroes() {
    for kind in [1u32, 2] {
        let (mut bytes, entry, payload) = fixture();
        let block = &mut bytes[entry..entry + BLOCK];
        block[56..64].copy_from_slice(&2059u64.to_le_bytes());
        put32(block, 172, 24);
        put32(block, 176, (kind << 30) | 2048);
        put32(block, 180, if kind == 1 { payload } else { u32::MAX });
        put32(block, 184, 11);
        put32(block, 188, payload);
        put32(block, 192, 0);
        put32(block, 196, 0);
        retag(block, 184);
        let mut expected = vec![0; 2048];
        expected.extend_from_slice(b"hello world");
        assert_eq!(read(&bytes), expected);
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let index = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "payload.txt")
            .unwrap();
        let mut classified = Vec::new();
        reader
            .visit_classified_extents(index, |logical, class, length| {
                classified.push((logical, class, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            classified[0],
            (
                0,
                if kind == 1 {
                    libmkiso::UdfExtentKind::AllocatedUnrecorded
                } else {
                    libmkiso::UdfExtentKind::Unallocated
                },
                2048
            )
        );
        let mut legacy = Vec::new();
        reader
            .visit_extents(index, |logical, offset, length| {
                legacy.push((logical, offset, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(legacy[0], (0, None, 2048));
    }
}

fn chain_fixture(header_only_crc: bool) -> (Vec<u8>, usize) {
    let (mut bytes, entry, payload) = fixture();
    let partition_descriptor = 259 * BLOCK;
    let start = get32(&bytes, partition_descriptor + 188) as usize;
    let old_blocks = get32(&bytes, partition_descriptor + 192);
    let aed_offset = (start + old_blocks as usize) * BLOCK;
    bytes.resize(bytes.len().max(aed_offset + BLOCK), 0);
    put32(&mut bytes, partition_descriptor + 192, old_blocks + 1);
    retag(
        &mut bytes[partition_descriptor..partition_descriptor + BLOCK],
        496,
    );
    let aed = &mut bytes[aed_offset..aed_offset + BLOCK];
    aed.fill(0);
    aed[0..2].copy_from_slice(&258u16.to_le_bytes());
    aed[2..4].copy_from_slice(&2u16.to_le_bytes());
    put32(aed, 12, old_blocks);
    put32(aed, 20, 8);
    put32(aed, 24, 11);
    put32(aed, 28, payload);
    retag(aed, if header_only_crc { 8 } else { 16 });
    let fe = &mut bytes[entry..entry + BLOCK];
    put32(fe, 176, 0xc000_0800);
    put32(fe, 180, old_blocks);
    retag(fe, 168);
    (bytes, aed_offset)
}

#[test]
fn allocation_continuation_reads_both_udf_crc_profiles() {
    for header_only in [false, true] {
        assert_eq!(read(&chain_fixture(header_only).0), b"hello world");
    }
}

#[test]
fn allocation_continuation_cycles_and_bad_ranges_are_rejected() {
    for cycle in [false, true] {
        let (mut bytes, offset) = chain_fixture(false);
        let aed = &mut bytes[offset..offset + BLOCK];
        if cycle {
            put32(aed, 24, 0xc000_0800);
            let location = get32(aed, 12);
            put32(aed, 28, location);
        } else {
            put32(aed, 20, 2048);
        }
        retag(aed, 16);
        assert!(UdfReader::open(&bytes, Limits::default()).is_err());
    }
}

#[test]
fn unrecorded_allocated_extents_still_require_valid_partition_ranges() {
    let (mut bytes, entry, _) = fixture();
    let fe = &mut bytes[entry..entry + BLOCK];
    put32(fe, 176, 0x4000_000b);
    put32(fe, 180, u32::MAX);
    retag(fe, 168);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn long_allocation_continuations_extract_recorded_data() {
    let (mut bytes, offset) = chain_fixture(false);
    let aed = &mut bytes[offset..offset + BLOCK];
    put32(aed, 20, 16);
    retag(aed, 24);
    let entry = bytes
        .chunks_exact(BLOCK)
        .enumerate()
        .find(|(_, block)| {
            block[0..2] == 261u16.to_le_bytes()
                && block[27] == 5
                && block[56..64] == 11u64.to_le_bytes()
        })
        .unwrap()
        .0
        * BLOCK;
    let fe = &mut bytes[entry..entry + BLOCK];
    fe[34..36].copy_from_slice(&1u16.to_le_bytes());
    put32(fe, 172, 16);
    fe[184..192].fill(0);
    retag(fe, 176);
    assert_eq!(read(&bytes), b"hello world");
}

#[test]
fn continuation_extent_is_charged_against_metadata_budget() {
    let (bytes, _) = chain_fixture(false);
    let limits = Limits {
        max_metadata_bytes: 2047,
        ..Limits::default()
    };
    assert!(matches!(
        UdfReader::open(&bytes, limits),
        Err(libmkiso::udf::Error::ResourceLimit(_))
    ));
}

fn large_sparse_fixture() -> Vec<u8> {
    let (mut bytes, entry, _) = fixture();
    let fe = &mut bytes[entry..entry + BLOCK];
    fe[56..64].copy_from_slice(&(196_609u64).to_le_bytes());
    put32(fe, 176, 0x8003_0001);
    put32(fe, 180, u32::MAX);
    retag(fe, 168);
    bytes
}

#[test]
fn sparse_payload_streams_in_bounded_zero_chunks_and_respects_buffer_limit() {
    #[derive(Default)]
    struct CountWrites {
        lengths: Vec<usize>,
    }
    impl std::io::Write for CountWrites {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert!(bytes.iter().all(|byte| *byte == 0));
            self.lengths.push(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bytes = large_sparse_fixture();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "payload.txt")
        .unwrap();
    assert!(matches!(
        reader.read_entry(index, 196_608),
        Err(libmkiso::udf::Error::ResourceLimit(_))
    ));
    let mut output = CountWrites::default();
    assert_eq!(reader.extract(index, &mut output).unwrap(), 196_609);
    assert_eq!(output.lengths, [65_536, 65_536, 65_536, 1]);
}

#[test]
fn sparse_extraction_propagates_output_errors() {
    struct FailingWriter;
    impl std::io::Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "test output closed",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let bytes = large_sparse_fixture();
    let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "payload.txt")
        .unwrap();
    assert!(
        matches!(reader.extract(index, &mut FailingWriter), Err(libmkiso::udf::Error::Io(error)) if error.kind() == std::io::ErrorKind::BrokenPipe)
    );
}

#[test]
fn allocation_padding_and_alignment_are_checked_across_aed_boundaries() {
    for first_length in [12, 2048] {
        let (mut bytes, aed_offset) = chain_fixture(false);
        let entry = bytes
            .chunks_exact(BLOCK)
            .enumerate()
            .find(|(_, block)| {
                block[0..2] == 261u16.to_le_bytes()
                    && block[27] == 5
                    && block[56..64] == 11u64.to_le_bytes()
            })
            .unwrap()
            .0
            * BLOCK;
        let continuation_block = get32(&bytes, entry + 180);
        let fe = &mut bytes[entry..entry + BLOCK];
        if first_length == 12 {
            fe[56..64].copy_from_slice(&23u64.to_le_bytes());
        }
        put32(fe, 172, 16);
        put32(fe, 176, 0x8000_0000 | first_length);
        put32(fe, 180, u32::MAX);
        put32(fe, 184, 0xc000_0800);
        put32(fe, 188, continuation_block);
        retag(fe, 176);
        // The AED follows either a non-aligned body extent or an oversized tail.
        assert_eq!(get32(&bytes, aed_offset + 24), 11);
        assert!(UdfReader::open(&bytes, Limits::default()).is_err());
    }
}

#[test]
fn unallocated_long_extent_rejects_unknown_partition_reference() {
    let (mut bytes, entry, _) = fixture();
    let fe = &mut bytes[entry..entry + BLOCK];
    fe[34..36].copy_from_slice(&1u16.to_le_bytes());
    put32(fe, 172, 16);
    put32(fe, 176, 0x8000_000b);
    put32(fe, 180, u32::MAX);
    fe[184..186].copy_from_slice(&1u16.to_le_bytes());
    fe[186..192].fill(0);
    retag(fe, 176);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn preallocated_tail_is_validated_but_not_extracted() {
    let (mut bytes, entry, payload) = fixture();
    let block = &mut bytes[entry..entry + BLOCK];
    put32(block, 172, 16);
    put32(block, 184, (1 << 30) | 2048);
    put32(block, 188, payload);
    retag(block, 176);
    assert_eq!(read(&bytes), b"hello world");
    put32(&mut bytes[entry..entry + BLOCK], 188, u32::MAX);
    retag(&mut bytes[entry..entry + BLOCK], 176);
    assert!(UdfReader::open(&bytes, Limits::default()).is_err());
}

#[test]
fn recorded_and_unallocated_tails_are_rejected() {
    for kind in [0u32, 2] {
        let (mut bytes, entry, payload) = fixture();
        let block = &mut bytes[entry..entry + BLOCK];
        put32(block, 172, 16);
        put32(block, 184, (kind << 30) | 2048);
        put32(block, 188, payload);
        retag(block, 176);
        assert!(UdfReader::open(&bytes, Limits::default()).is_err());
    }
}

#[test]
fn empty_file_can_have_preallocated_tail() {
    let (mut bytes, entry, payload) = fixture();
    let block = &mut bytes[entry..entry + BLOCK];
    block[56..64].copy_from_slice(&0u64.to_le_bytes());
    put32(block, 176, (1 << 30) | 2048);
    put32(block, 180, payload);
    retag(block, 168);
    assert!(read(&bytes).is_empty());
}

#[test]
fn sparse_continuation_classification_distinguishes_both_zero_classes() {
    for kind in [1u32, 2] {
        let (mut bytes, aed_offset) = chain_fixture(false);
        let aed = &mut bytes[aed_offset..aed_offset + BLOCK];
        put32(aed, 24, (kind << 30) | 11);
        retag(aed, 16);
        let reader = UdfReader::open(&bytes, Limits::default()).unwrap();
        let index = reader
            .entries()
            .iter()
            .position(|entry| entry.name == "payload.txt")
            .unwrap();
        let mut classes = Vec::new();
        reader
            .visit_classified_extents(index, |logical, class, length| {
                classes.push((logical, class, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            classes,
            [(
                0,
                if kind == 1 {
                    libmkiso::UdfExtentKind::AllocatedUnrecorded
                } else {
                    libmkiso::UdfExtentKind::Unallocated
                },
                11
            )]
        );
        assert_eq!(reader.read_entry(index, 11).unwrap(), [0; 11]);
        let mut legacy = Vec::new();
        reader
            .visit_extents(index, |logical, offset, length| {
                legacy.push((logical, offset, length));
                Ok(())
            })
            .unwrap();
        assert_eq!(legacy, [(0, None, 11)]);
    }
}
