#![cfg(feature = "native-writer")]
use libmkiso::{
    iso9660::{IsoReader, Limits},
    write_iso9660,
};
use std::{fs, process::Command};

#[test]
fn nested_multisector_directories_roundtrip_deterministically() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested/empty")).unwrap();
    for i in 0..100 {
        fs::write(source.join(format!("file{i:03}.bin")), vec![i as u8; 3000]).unwrap();
    }
    fs::write(source.join("nested/hello.txt"), b"hello").unwrap();
    fs::write(source.join("zero"), []).unwrap();
    let output = temp.path().join("first.iso");
    write_iso9660(&source, &output).unwrap();
    let second = temp.path().join("second.iso");
    write_iso9660(&source, &second).unwrap();
    assert_eq!(fs::read(&output).unwrap(), fs::read(second).unwrap());
    let mut reader = IsoReader::open(fs::File::open(&output).unwrap(), Limits::default()).unwrap();
    assert_eq!(reader.entries().len(), 104);
    let entries = reader.entries().to_vec();
    for (i, entry) in entries.iter().enumerate().filter(|(_, e)| !e.directory) {
        let mut bytes = Vec::new();
        reader.extract(i, &mut bytes).unwrap();
        assert_eq!(
            bytes,
            fs::read(source.join(entry.name.to_ascii_lowercase())).unwrap()
        );
    }
    assert!(write_iso9660(&source, &output).is_err());
    let seven = ["7z", "7zz"]
        .into_iter()
        .find(|name| Command::new(name).arg("i").output().is_ok());
    if let Some(seven) = seven {
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
                fs::read(source.join(entry.name.to_ascii_lowercase())).unwrap()
            );
        }
    } else {
        eprintln!("7z unavailable: independent extraction skipped");
    }
}

#[test]
fn invalid_names_and_case_collisions_leave_no_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("disc.iso");
    fs::write(source.join("bad-name"), []).unwrap();
    assert!(write_iso9660(&source, &output).is_err());
    assert!(!output.exists());
    fs::remove_file(source.join("bad-name")).unwrap();
    // Host case-collision fixtures require a case-sensitive filesystem.
    // The deferred inventory collision test exercises every host.
    {
        fs::write(source.join("same.txt"), []).unwrap();
        fs::write(source.join("SAME.TXT"), []).unwrap();
        if fs::read_dir(&source).unwrap().count() == 2 {
            assert!(write_iso9660(&source, &output).is_err());
            assert!(!output.exists());
        }
    }
}

#[test]
fn empty_source_produces_readable_empty_volume() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("disc.iso");
    write_iso9660(&source, &output).unwrap();
    let reader = IsoReader::open(fs::File::open(output).unwrap(), Limits::default()).unwrap();
    assert!(reader.entries().is_empty());
}

#[test]
fn oversized_files_and_excessive_depth_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("disc.iso");
    let large = source.join("large.bin");
    fs::File::create(&large)
        .unwrap()
        .set_len(u64::from(u32::MAX) + 1)
        .unwrap();
    assert!(write_iso9660(&source, &output).is_err());
    assert!(!output.exists());
    fs::remove_file(large).unwrap();
    let mut nested = source.clone();
    for _ in 0..8 {
        nested.push("DIR");
    }
    fs::create_dir_all(nested).unwrap();
    assert!(write_iso9660(&source, &output).is_err());
    assert!(!output.exists());
}

#[cfg(unix)]
#[test]
fn symlinks_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    std::os::unix::fs::symlink(&source, source.join("LOOP")).unwrap();
    let output = temp.path().join("disc.iso");
    assert!(write_iso9660(&source, &output).is_err());
    assert!(!output.exists());
}
