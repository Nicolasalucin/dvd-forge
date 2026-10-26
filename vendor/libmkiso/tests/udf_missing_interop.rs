//! Independent udftools checks for virtual and sparable partition profiles.
use libmkiso::udf::{EntryKind, Limits, UdfReader};
use std::{ffi::OsString, fs, process::Command};

fn formatter() -> Option<OsString> {
    let executable = std::env::var_os("MKUDFFS").unwrap_or_else(|| "mkudffs".into());
    if Command::new(&executable).arg("--help").output().is_err() {
        eprintln!("mkudffs unavailable: independent producer check skipped (set MKUDFFS)");
        return None;
    }
    Some(executable)
}

#[test]
fn independent_udftools_virtual_volumes_read_open_and_closed_discs() {
    let Some(formatter) = formatter() else { return };
    let temp = tempfile::tempdir().unwrap();
    for revision in ["1.50", "2.01"] {
        for closed in [false, true] {
            let image = temp.path().join(format!("vat-{revision}-{closed}.img"));
            let mut command = Command::new(&formatter);
            command
                .args(["--new-file", "--blocksize=2048", "--media-type=cdr"])
                .arg(format!("--udfrev={revision}"));
            if closed {
                command.arg("--closed");
            }
            let formatted = command.arg(&image).arg("4096").output().unwrap();
            assert!(
                formatted.status.success(),
                "{}",
                String::from_utf8_lossy(&formatted.stderr)
            );
            let bytes = fs::read(&image).unwrap();
            let reader = UdfReader::open(&bytes, Limits::default())
                .unwrap_or_else(|error| panic!("VAT {revision}/closed={closed}: {error}"));
            for (index, entry) in reader.entries().iter().enumerate() {
                assert_eq!(
                    entry.kind,
                    EntryKind::SystemStream,
                    "unexpected ordinary file: {entry:?}"
                );
                assert_eq!(
                    reader.read_entry(index, 8 << 20).unwrap().len() as u64,
                    entry.size
                );
            }
        }
    }
}

#[test]
fn independent_udftools_sparable_volumes_read_with_indirect_strategy() {
    let Some(formatter) = formatter() else { return };
    let temp = tempfile::tempdir().unwrap();
    for media in ["cdrw", "dvdrw"] {
        for strategy in ["4", "4096"] {
            let image = temp.path().join(format!("sparable-{media}-{strategy}.img"));
            let formatted = Command::new(&formatter)
                .args(["--new-file", "--blocksize=2048"])
                .arg(format!("--media-type={media}"))
                .arg("--udfrev=1.50")
                .arg(format!("--strategy={strategy}"))
                .arg(&image)
                .arg("4096")
                .output()
                .unwrap();
            assert!(
                formatted.status.success(),
                "{}",
                String::from_utf8_lossy(&formatted.stderr)
            );
            let bytes = fs::read(&image).unwrap();
            let reader = UdfReader::open(&bytes, Limits::default())
                .unwrap_or_else(|error| panic!("sparable {media}/strategy={strategy}: {error}"));
            assert_eq!(reader.entries().len(), 1);
            let entry = &reader.entries()[0];
            assert_eq!(entry.kind, EntryKind::File);
            assert_eq!(entry.name, "Non-Allocatable Space");
            assert_eq!(entry.size, 0);
            assert!(reader.read_entry(0, 0).unwrap().is_empty());
        }
    }
}
