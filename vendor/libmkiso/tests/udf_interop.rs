//! Independent udftools formatter checks for baseline physical-partition profiles.
use libmkiso::udf::{Limits, UdfReader};
use std::{fs, process::Command};

#[test]
fn independent_udftools_empty_volumes_read_across_revisions_and_allocations() {
    let formatter = std::env::var_os("MKUDFFS").unwrap_or_else(|| "mkudffs".into());
    if Command::new(&formatter).arg("--help").output().is_err() {
        eprintln!("mkudffs unavailable: independent producer check skipped (set MKUDFFS)");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    for revision in ["1.02", "1.50", "2.00", "2.01"] {
        for allocation in ["inicb", "short", "long"] {
            let image = temp.path().join(format!("udf-{revision}-{allocation}.img"));
            let formatted = Command::new(&formatter)
                .args(["--new-file", "--blocksize=2048", "--media-type=hd"])
                .arg(format!("--udfrev={revision}"))
                .arg(format!("--ad={allocation}"))
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
                .unwrap_or_else(|error| panic!("{revision}/{allocation}: {error}"));
            assert!(reader.entries().is_empty());
        }
    }
}
