//! Independent UDF extraction checks. A missing 7z executable is explicitly skipped.
#![cfg(feature = "native-writer")]
use libmkiso::write_iso;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
};
fn reader() -> Option<&'static str> {
    for executable in ["7z", "7zz"] {
        if Command::new(executable).arg("i").output().is_ok() {
            return Some(executable);
        }
    }
    eprintln!("SKIP: independent UDF interoperability requires 7z or 7zz");
    None
}
fn fixture(source: &Path) {
    fs::create_dir_all(source.join("boot")).unwrap();
    fs::create_dir_all(source.join("efi/microsoft/boot")).unwrap();
    fs::create_dir_all(source.join("deep/a/b/c/d/e/f/g/h/i")).unwrap();
    fs::write(source.join("boot/etfsboot.com"), [0x51; 4096]).unwrap();
    fs::write(source.join("efi/microsoft/boot/efisys.bin"), [0x52; 4096]).unwrap();
    fs::write(
        source.join("deep/a/b/c/d/e/f/g/h/i/日本語.txt"),
        b"Unicode and deep paths",
    )
    .unwrap();
    fs::write(source.join("empty"), []).unwrap();
    fs::create_dir_all(source.join("many")).unwrap();
    for i in 0..200 {
        fs::write(
            source.join(format!("many/file-{i:03}.txt")),
            format!("entry {i}"),
        )
        .unwrap();
    }
}
fn extract(reader: &str, image: &Path, output: &Path) {
    let result = Command::new(reader)
        .arg("x")
        .arg("-y")
        .arg(format!("-o{}", output.display()))
        .arg(image)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "7z extraction failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn independent_reader_extracts_every_byte_from_reproducible_udf_media() {
    let Some(reader) = reader() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fixture(&source);
    let first = tmp.path().join("first.iso");
    let second = tmp.path().join("second.iso");
    write_iso(&source, &first).unwrap();
    write_iso(&source, &second).unwrap();
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    let extracted = tmp.path().join("extracted");
    extract(reader, &first, &extracted);
    for i in 0..200 {
        let relative = format!("many/file-{i:03}.txt");
        assert_eq!(
            fs::read(source.join(&relative)).unwrap(),
            fs::read(extracted.join(&relative)).unwrap()
        );
    }
    for relative in [
        "boot/etfsboot.com",
        "efi/microsoft/boot/efisys.bin",
        "deep/a/b/c/d/e/f/g/h/i/日本語.txt",
        "empty",
    ] {
        assert_eq!(
            fs::read(source.join(relative)).unwrap(),
            fs::read(extracted.join(relative)).unwrap(),
            "content mismatch: {relative}"
        );
    }
}
#[test]
#[ignore = "writes and extracts over 8 GiB; run explicitly with --ignored"]
fn independent_reader_extracts_file_larger_than_four_gib() {
    let Some(reader) = reader() else { return };
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    fixture(&source);
    let size = (1u64 << 32) + 4096;
    let marker = b"larger-than-four-gib";
    let mut file = fs::File::create(source.join("install.wim")).unwrap();
    file.set_len(size).unwrap();
    file.seek(SeekFrom::Start(size - marker.len() as u64))
        .unwrap();
    file.write_all(marker).unwrap();
    drop(file);
    let image = tmp.path().join("large.iso");
    write_iso(&source, &image).unwrap();
    let extracted = tmp.path().join("extracted");
    extract(reader, &image, &extracted);
    let mut file = fs::File::open(extracted.join("install.wim")).unwrap();
    assert_eq!(file.metadata().unwrap().len(), size);
    file.seek(SeekFrom::Start(size - marker.len() as u64))
        .unwrap();
    let mut tail = vec![0; marker.len()];
    file.read_exact(&mut tail).unwrap();
    assert_eq!(tail, marker);
}
