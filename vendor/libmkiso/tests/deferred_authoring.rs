#![cfg(feature = "native-writer")]
use libmkiso::{tree_source::*, *};
use std::{io, sync::Arc};
struct Inventory(TreeInventory);
impl FileTreeSource for Inventory {
    fn inventory(&self, entries: usize, bytes: usize) -> io::Result<TreeInventory> {
        self.0.validate_budget(entries, bytes)?;
        Ok(self.0.clone())
    }
}
fn source() -> Inventory {
    let content = DeferredContent::new(Arc::new(BufferContent::new(1, vec![0x5a; 200_000])));
    Inventory(TreeInventory {
        entries: vec![TreeEntry {
            path: "FILE.BIN".into(),
            native_name: b"FILE.BIN".to_vec(),
            object: 1,
            kind: TreeEntryKind::File(vec![TreeExtent::Data(content)]),
            streams: vec![],
            metadata: Default::default(),
        }],
        ..Default::default()
    })
}
#[test]
fn staged_udf_and_iso_emit_deferred_payloads() {
    let directory = tempfile::tempdir().unwrap();
    let image = UdfImage::from_tree_source(&source(), 10, 4096).unwrap();
    let (path, hash) = image
        .stage_with_cancel(directory.path(), &UdfOptions::default(), || Ok(()))
        .unwrap();
    assert_eq!(hash.len(), 64);
    let bytes = std::fs::read(&path).unwrap();
    let reader = UdfReader::open(&bytes, UdfLimits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "FILE.BIN")
        .unwrap();
    assert_eq!(
        reader.read_entry(index, 200_000).unwrap(),
        vec![0x5a; 200_000]
    );
    let independent = ["7zz", "7z"].into_iter().find(|executable| {
        std::process::Command::new(executable)
            .arg("i")
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|line| line.starts_with("7-Zip"))
                    .flat_map(str::split_whitespace)
                    .filter_map(|word| word.split('.').next()?.parse::<u32>().ok())
                    .any(|major| major >= 20)
            })
    });
    if let Some(independent) = independent {
        let output = std::process::Command::new(independent)
            .args(["e", "-so", "-tUdf"])
            .arg(&path)
            .arg("FILE.BIN")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, vec![0x5a; 200_000]);
    }
    let path =
        stage_iso9660_from_tree_source(&source(), directory.path(), &IsoOptions::default(), || {
            Ok(())
        })
        .unwrap();
    let mut reader =
        IsoReader::open(std::fs::File::open(path).unwrap(), IsoLimits::default()).unwrap();
    let index = reader
        .entries()
        .iter()
        .position(|entry| entry.name == "FILE.BIN")
        .unwrap();
    assert_eq!(
        reader.read_entry(index, 200_000).unwrap(),
        vec![0x5a; 200_000]
    );
}
#[test]
fn cancellation_and_host_drift_leave_no_published_image() {
    let directory = tempfile::tempdir().unwrap();
    let image = UdfImage::from_tree_source(&source(), 10, 4096).unwrap();
    assert!(
        image
            .stage_with_cancel(directory.path(), &UdfOptions::default(), || anyhow::bail!(
                "cancelled"
            ))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let input = directory.path().join("file.bin");
    std::fs::write(&input, b"before").unwrap();
    let content = host_file_content(&input).unwrap();
    std::fs::write(&input, b"after").unwrap();
    assert!(content.read_exact_at(0, &mut [0]).is_err());
}

#[test]
fn named_stream_payload_exceeds_inventory_budget_and_failures_cleanup() {
    let directory = tempfile::tempdir().unwrap();
    let mut inventory = source();
    inventory.0.entries[0].streams.push(TreeStream {
        metadata: Default::default(),
        name: "extra".into(),
        native_name: b"extra".to_vec(),
        content: DeferredContent::new(Arc::new(BufferContent::new(2, vec![7; 300_000]))),
    });
    let image = UdfImage::from_tree_source(&inventory, 10, 4096).unwrap();
    let options = UdfOptions {
        revision: UdfRevision::V200,
        ..Default::default()
    };
    let (path, _) = image
        .stage_with_cancel(directory.path(), &options, || Ok(()))
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let reader = UdfReader::open(&bytes, UdfLimits::default()).unwrap();
    let stream = reader
        .entries()
        .iter()
        .position(|entry| entry.stream.is_some())
        .unwrap();
    assert_eq!(
        reader.read_entry(stream, 300_000).unwrap(),
        vec![7; 300_000]
    );
    assert!(
        stage_iso9660_from_tree_source(
            &inventory,
            directory.path(),
            &IsoOptions::default(),
            || Ok(())
        )
        .is_err()
    );
    struct Failing;
    impl ContentSource for Failing {
        fn identity(&self) -> SourceIdentity {
            SourceIdentity {
                object: 1,
                generation: 0,
                size: 200_000,
            }
        }
        fn validate(&self, _: SourceIdentity) -> io::Result<()> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("injected deferred read failure"))
        }
    }
    inventory.0.entries[0].kind = TreeEntryKind::File(vec![TreeExtent::Data(
        DeferredContent::new(Arc::new(Failing)),
    )]);
    let image = UdfImage::from_tree_source(&inventory, 10, 4096).unwrap();
    let count = std::fs::read_dir(directory.path()).unwrap().count();
    assert!(
        image
            .stage_with_cancel(directory.path(), &options, || Ok(()))
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), count);
}

#[test]
fn policies_report_transformations_and_gate_faithful_staging() {
    use libmkiso::preservation::{PartitionProfile, Policy, Profile};
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::Udf {
        revision: 0x102,
        partition: PartitionProfile::Physical,
    };
    let (_, report) =
        UdfImage::from_tree_source_with_policy(&source(), 10, 4096, profile, Policy::ContentOnly)
            .unwrap();
    assert!(report.allowed());
    assert!(!report.faithful());
    assert!(!report.issues.is_empty());
    assert!(
        UdfImage::from_tree_source_with_policy(&source(), 10, 4096, profile, Policy::Faithful)
            .is_err()
    );
    assert!(
        stage_iso9660_from_tree_source_with_policy(
            &source(),
            directory.path(),
            &IsoOptions::default(),
            Policy::Faithful,
            || Ok(())
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let (path, report) = stage_iso9660_from_tree_source_with_policy(
        &source(),
        directory.path(),
        &IsoOptions::default(),
        Policy::ContentOnly,
        || Ok(()),
    )
    .unwrap();
    assert!(report.allowed());
    assert!(!report.faithful());
    assert!(path.exists());
    drop(path);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn retained_reader_authors_large_files_streams_links_and_holes_without_extraction() {
    use libmkiso::source::ReadAt;
    use libmkiso::udf_tree::UdfTreeSource;
    use std::io::{Read, Seek, SeekFrom};
    struct ImageSource {
        path: std::path::PathBuf,
        size: u64,
    }
    impl ReadAt for ImageSource {
        fn len(&self) -> u64 {
            self.size
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            let mut file = std::fs::File::open(&self.path)?;
            file.seek(SeekFrom::Start(offset))?;
            file.read(buffer)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let options = UdfOptions {
        revision: UdfRevision::V201,
        ..Default::default()
    };
    let mut original = UdfImage::new();
    original.add_bytes("file", vec![0x65; 200_000]).unwrap();
    original.add_hard_link("alias", "file").unwrap();
    original
        .add_named_stream("file", "extra", vec![0x73; 300_000])
        .unwrap();
    original
        .add_root_stream("root-data", vec![0x72; 100_000])
        .unwrap();
    original
        .add_system_stream("system-data", vec![0x79; 100_000])
        .unwrap();
    original
        .add_sparse_file(
            "sparse",
            vec![
                UdfFileExtent::Data(vec![0x61; 2048]),
                UdfFileExtent::Hole(2048),
                UdfFileExtent::AllocatedHole(2048),
                UdfFileExtent::Data(b"end".to_vec()),
            ],
        )
        .unwrap();
    original.add_symlink("link", "file").unwrap();
    let input = directory.path().join("input.udf");
    original.write(&input, &options).unwrap();
    let size = std::fs::metadata(&input).unwrap().len();
    let reader =
        UdfReader::open_source(ImageSource { path: input, size }, UdfLimits::default()).unwrap();
    #[allow(clippy::arc_with_non_send_sync)]
    let source = UdfTreeSource(Arc::new(reader));
    let inventory = source.inventory(20, 32 * 1024).unwrap();
    inventory.validate_budget(20, 32 * 1024).unwrap();
    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| matches!(entry.kind, TreeEntryKind::HardLink(_)))
    );
    assert!(inventory.entries.iter().any(|entry| matches!(&entry.kind, TreeEntryKind::File(extents) if extents.iter().any(|extent| matches!(extent, TreeExtent::Hole(_))))));
    assert!(inventory.entries.iter().any(|entry| matches!(&entry.kind, TreeEntryKind::File(extents) if extents.iter().any(|extent| matches!(extent, TreeExtent::AllocatedHole(_))))));
    let copy = UdfImage::from_tree_source(&source, 20, 32 * 1024).unwrap();
    drop(source);
    drop(inventory);
    let (staged, _) = copy
        .stage_with_cancel(directory.path(), &options, || Ok(()))
        .unwrap();
    let bytes = std::fs::read(&staged).unwrap();
    let reader = UdfReader::open(&bytes, UdfLimits::default()).unwrap();
    let find = |name: &str| {
        reader
            .entries()
            .iter()
            .position(|entry| entry.name == name)
            .unwrap()
    };
    let file = find("file");
    let alias = find("alias");
    assert_eq!(reader.entries()[file].icb, reader.entries()[alias].icb);
    assert_eq!(
        reader.read_entry(file, 200_000).unwrap(),
        vec![0x65; 200_000]
    );
    assert_eq!(
        reader.entries()[find("link")].link_target.as_deref(),
        Some("file")
    );
    for (name, size, value) in [
        ("extra", 300_000, 0x73),
        ("root-data", 100_000, 0x72),
        ("system-data", 100_000, 0x79),
    ] {
        let index = reader
            .entries()
            .iter()
            .position(|entry| {
                entry
                    .stream
                    .as_ref()
                    .is_some_and(|stream| stream.name == name)
            })
            .unwrap();
        assert_eq!(
            reader.read_entry(index, size).unwrap(),
            vec![value; size as usize]
        );
    }
    let sparse = reader.read_entry(find("sparse"), 8192).unwrap();
    assert!(sparse[..2048].iter().all(|&byte| byte == 0x61));
    assert!(sparse[2048..6144].iter().all(|&byte| byte == 0));
    assert_eq!(&sparse[6144..], b"end");
    // 7-Zip rejects UDF symbolic-link file types. Keep the combined topology
    // fixture checked here, and validate ordinary staged content independently
    // in staged_udf_and_iso_emit_deferred_payloads.
}

#[test]
fn drift_after_last_payload_read_is_rejected_before_retaining_output() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct LateDrift {
        copied: Arc<AtomicBool>,
        drift: Arc<AtomicBool>,
    }
    impl ContentSource for LateDrift {
        fn identity(&self) -> SourceIdentity {
            SourceIdentity {
                object: 1,
                generation: 0,
                size: 3,
            }
        }
        fn validate(&self, _: SourceIdentity) -> io::Result<()> {
            if self.drift.load(Ordering::Relaxed) {
                Err(io::Error::other("late generation drift"))
            } else {
                Ok(())
            }
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
            let count = 3u64.saturating_sub(offset).min(buffer.len() as u64) as usize;
            buffer[..count].copy_from_slice(&b"abc"[offset as usize..offset as usize + count]);
            if offset + count as u64 == 3 {
                self.copied.store(true, Ordering::Relaxed);
            }
            Ok(count)
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let copied = Arc::new(AtomicBool::new(false));
    let drift = Arc::new(AtomicBool::new(false));
    let source = Inventory(TreeInventory {
        entries: vec![TreeEntry {
            path: "FILE.BIN".into(),
            native_name: b"FILE.BIN".to_vec(),
            object: 1,
            metadata: Default::default(),
            streams: vec![],
            kind: TreeEntryKind::File(vec![TreeExtent::Data(DeferredContent::new(Arc::new(
                LateDrift {
                    copied: copied.clone(),
                    drift: drift.clone(),
                },
            )))]),
        }],
        ..Default::default()
    });
    let checkpoint = || {
        if copied.load(Ordering::Relaxed) {
            drift.store(true, Ordering::Relaxed);
        }
        Ok(())
    };
    let image = UdfImage::from_tree_source(&source, 10, 4096).unwrap();
    assert!(
        image
            .stage_with_cancel(directory.path(), &UdfOptions::default(), checkpoint)
            .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    copied.store(false, Ordering::Relaxed);
    drift.store(false, Ordering::Relaxed);
    assert!(
        stage_iso9660_from_tree_source(&source, directory.path(), &IsoOptions::default(), || {
            if copied.load(Ordering::Relaxed) {
                drift.store(true, Ordering::Relaxed);
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
