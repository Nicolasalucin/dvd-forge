#![cfg(feature = "native-writer")]
use libmkiso::{IsoOptions, IsoTimestamp, write_iso9660_with_options};
use std::fs;

#[test]
fn custom_label_and_timestamp_are_stored_and_reproducible() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("hello.txt"), b"hello").unwrap();
    let options = IsoOptions {
        volume_label: "MY_DISC".into(),
        timestamp: IsoTimestamp {
            year: 2026,
            month: 10,
            day: 5,
            hour: 12,
            minute: 34,
            second: 56,
        },
        ..IsoOptions::default()
    };
    let output = temp.path().join("first.iso");
    let second = temp.path().join("second.iso");
    write_iso9660_with_options(&source, &output, &options).unwrap();
    write_iso9660_with_options(&source, &second, &options).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert_eq!(bytes, fs::read(second).unwrap());
    let primary = &bytes[16 * 2048..17 * 2048];
    assert_eq!(&primary[40..72], b"MY_DISC                         ");
    assert_eq!(&primary[813..830], b"2026100512345600\0");
    assert_eq!(&primary[830..847], b"2026100512345600\0");
    assert_eq!(&primary[174..181], &[126, 10, 5, 12, 34, 56, 0]);
    let root_sector = u32::from_le_bytes(primary[158..162].try_into().unwrap()) as usize;
    let root = &bytes[root_sector * 2048..(root_sector + 1) * 2048];
    assert_eq!(&root[18..25], &[126, 10, 5, 12, 34, 56, 0]);
}

#[test]
fn invalid_writer_options_leave_no_published_or_temporary_image() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let output = temp.path().join("bad.iso");
    for options in [
        IsoOptions {
            volume_label: String::new(),
            ..IsoOptions::default()
        },
        IsoOptions {
            volume_label: "bad label".into(),
            ..IsoOptions::default()
        },
        IsoOptions {
            volume_label: "A".repeat(33),
            ..IsoOptions::default()
        },
        IsoOptions {
            volume_label: "A".repeat(17),
            joliet: true,
            ..IsoOptions::default()
        },
        IsoOptions {
            timestamp: IsoTimestamp {
                year: 2026,
                month: 2,
                day: 29,
                ..IsoTimestamp::default()
            },
            ..IsoOptions::default()
        },
    ] {
        assert!(write_iso9660_with_options(&source, &output, &options).is_err());
        assert!(!output.exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
