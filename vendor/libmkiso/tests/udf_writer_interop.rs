//! Populated writer images checked with an independent 7-Zip UDF implementation.
#![cfg(feature = "native-writer")]

use libmkiso::{AllocationMode, UdfImage, UdfOptions, UdfPartition, UdfRevision};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

fn independent_reader() -> Option<OsString> {
    let candidates = std::env::var_os("SEVEN_ZIP")
        .map(|path| vec![path])
        .unwrap_or_else(|| vec!["7zz".into(), "7z".into()]);
    for executable in candidates {
        if let Ok(output) = Command::new(&executable).arg("i").output() {
            let banner = String::from_utf8_lossy(&output.stdout);
            let modern = banner
                .lines()
                .filter(|line| line.starts_with("7-Zip"))
                .flat_map(str::split_whitespace)
                .filter_map(|word| word.split('.').next()?.parse::<u32>().ok())
                .any(|major| major >= 20);
            if modern {
                return Some(executable);
            }
            eprintln!(
                "older 7-Zip lacks newer UDF profile support; set SEVEN_ZIP to 7zz 20 or later"
            );
        }
    }
    eprintln!("independent UDF reader unavailable; set SEVEN_ZIP to run interoperability checks");
    None
}

#[test]
fn independent_reader_extracts_populated_revision_and_partition_profiles() {
    let Some(executable) = independent_reader() else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let revisions = [
        UdfRevision::V102,
        UdfRevision::V150,
        UdfRevision::V200,
        UdfRevision::V201,
        UdfRevision::V250,
        UdfRevision::V260,
    ];
    let payload: Vec<u8> = (0..8193).map(|index| (index % 251) as u8).collect();
    let mut profiles = Vec::new();
    for revision in revisions {
        profiles.push((revision, UdfPartition::Physical, AllocationMode::Short));
        profiles.push((revision, UdfPartition::Physical, AllocationMode::Long));
        profiles.push((revision, UdfPartition::PhysicalSplit, AllocationMode::Long));
        profiles.push((revision, UdfPartition::Physical, AllocationMode::Embedded));
        if matches!(revision, UdfRevision::V250 | UdfRevision::V260) {
            profiles.push((
                revision,
                UdfPartition::Metadata { mirror: false },
                AllocationMode::Long,
            ));
            profiles.push((
                revision,
                UdfPartition::Metadata { mirror: true },
                AllocationMode::Long,
            ));
        }
    }
    for (index, (revision, partition, allocation)) in profiles.into_iter().enumerate() {
        let path = temp.path().join(format!("profile-{index}.udf"));
        let mut image = UdfImage::new();
        image
            .add_bytes("nested/payload.bin", payload.clone())
            .unwrap();
        image
            .add_bytes("small.txt", b"independent payload\n".to_vec())
            .unwrap();
        image.add_bytes("empty.bin", Vec::new()).unwrap();
        let options = UdfOptions {
            revision,
            partition,
            allocation,
            ..UdfOptions::default()
        };
        image
            .write(&path, &options)
            .unwrap_or_else(|error| panic!("{options:?}: {error}"));
        for (name, expected) in [
            ("nested/payload.bin", payload.as_slice()),
            ("small.txt", b"independent payload\n".as_slice()),
            ("empty.bin", &[]),
        ] {
            let extracted = Command::new(&executable)
                .args(["x", "-so", "-tUdf"])
                .arg(&path)
                .arg(name)
                .output()
                .unwrap();
            assert!(
                extracted.status.success(),
                "{options:?}/{name}: {}",
                String::from_utf8_lossy(&extracted.stderr)
            );
            assert_eq!(extracted.stdout, expected, "{options:?}/{name}");
        }
    }
}

fn client(
    executable: &OsString,
    image: &Path,
    commands: &str,
    directory: &Path,
) -> std::process::Output {
    let mut child = Command::new(executable)
        .args(["-b", "2048"])
        .arg(image)
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(commands.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn independent_udfclient_extracts_virtual_and_sparable_payloads() {
    let executable = std::env::var_os("UDFCLIENT").unwrap_or_else(|| "udfclient".into());
    if Command::new(&executable).output().is_err() {
        eprintln!("UDFclient unavailable: VAT/sparing payload check skipped (set UDFCLIENT)");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..8193).map(|index| (index % 251) as u8).collect();
    for revision in [UdfRevision::V150, UdfRevision::V200, UdfRevision::V201] {
        for (index, partition) in [
            UdfPartition::Virtual,
            UdfPartition::Sparable { packet_blocks: 32 },
        ]
        .into_iter()
        .enumerate()
        {
            let work = temp.path().join(format!("{}-{index}", revision.number()));
            fs::create_dir(&work).unwrap();
            let path = work.join("image.udf");
            let mut image = UdfImage::new();
            image
                .add_bytes("nested/payload.bin", payload.clone())
                .unwrap();
            image.add_bytes("empty.bin", Vec::new()).unwrap();
            let options = UdfOptions {
                revision,
                partition,
                allocation: AllocationMode::Long,
                ..UdfOptions::default()
            };
            image.write(&path, &options).unwrap();
            let listed = client(&executable, &path, "quit\n", &work);
            let listing = String::from_utf8_lossy(&listed.stdout);
            let mount = listing
                .lines()
                .find_map(|line| {
                    if line.starts_with("drwx") {
                        line.split_whitespace()
                            .last()
                            .filter(|name| name.matches(':').count() >= 3)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{options:?}: no mounted filesystem: {listing}\n{}",
                        String::from_utf8_lossy(&listed.stderr)
                    )
                });
            fs::create_dir_all(work.join("extracted/nested")).unwrap();
            let commands = format!(
                "get /{mount}/nested/payload.bin extracted/nested/payload.bin\nget /{mount}/empty.bin extracted/empty.bin\nquit\n"
            );
            let result = client(&executable, &path, &commands, &work);
            assert!(
                result.status.success(),
                "{options:?}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                fs::read(work.join("extracted/nested/payload.bin")).unwrap_or_else(|error| panic!(
                    "{options:?}: {error}: {} {}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                )),
                payload
            );
            assert_eq!(
                fs::metadata(work.join("extracted/empty.bin"))
                    .unwrap()
                    .len(),
                0
            );
        }
    }
}

#[test]
fn independent_udfclient_extracts_continuations_sparse_files_and_hard_links() {
    use libmkiso::UdfFileExtent;
    let executable = std::env::var_os("UDFCLIENT").unwrap_or_else(|| "udfclient".into());
    if Command::new(&executable).output().is_err() {
        eprintln!("UDFclient unavailable: allocation feature check skipped (set UDFCLIENT)");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..512003).map(|index| (index % 251) as u8).collect();
    for (index, (revision, partition)) in [
        (UdfRevision::V201, UdfPartition::Physical),
        (UdfRevision::V201, UdfPartition::Virtual),
        (
            UdfRevision::V201,
            UdfPartition::Sparable { packet_blocks: 32 },
        ),
        (UdfRevision::V250, UdfPartition::Metadata { mirror: true }),
        (
            UdfRevision::V260,
            UdfPartition::MetadataSparable {
                mirror: true,
                packet_blocks: 32,
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let work = temp.path().join(index.to_string());
        fs::create_dir(&work).unwrap();
        let path = work.join("image.udf");
        let mut image = UdfImage::new();
        image.add_bytes("payload.bin", payload.clone()).unwrap();
        image.add_hard_link("alias.bin", "payload.bin").unwrap();
        image.add_symlink("link", "payload.bin").unwrap();
        image
            .add_named_stream("payload.bin", "extra", b"named data".to_vec())
            .unwrap();
        image
            .add_system_stream("auxiliary", b"system data".to_vec())
            .unwrap();
        image
            .add_sparse_file(
                "sparse.bin",
                vec![
                    UdfFileExtent::Data(vec![0x57; 2048]),
                    UdfFileExtent::Hole(2048),
                    UdfFileExtent::Data(vec![0x19; 3]),
                ],
            )
            .unwrap();
        let options = UdfOptions {
            revision,
            partition,
            allocation: AllocationMode::Long,
            extent_blocks: 1,
            ..UdfOptions::default()
        };
        image.write(&path, &options).unwrap();
        let listed = client(&executable, &path, "quit\n", &work);
        let listing = String::from_utf8_lossy(&listed.stdout);
        let mount = listing
            .lines()
            .find_map(|line| {
                if line.starts_with("drwx") {
                    line.split_whitespace()
                        .last()
                        .filter(|name| name.matches(':').count() >= 3)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                panic!(
                    "{options:?}: no mounted filesystem: {listing}\n{}",
                    String::from_utf8_lossy(&listed.stderr)
                )
            });
        let commands = format!(
            "get /{mount}/payload.bin payload.bin\nget /{mount}/alias.bin alias.bin\nget /{mount}/sparse.bin sparse.bin\nquit\n"
        );
        let result = client(&executable, &path, &commands, &work);
        let report = format!(
            "{} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let actual = fs::read(work.join("payload.bin"))
            .unwrap_or_else(|error| panic!("{options:?}: {error}: {report}"));
        assert!(
            actual == payload,
            "{options:?}: payload length {} expected {}, first mismatch {:?}: {report}",
            actual.len(),
            payload.len(),
            actual.iter().zip(&payload).position(|(a, b)| a != b)
        );
        let actual = fs::read(work.join("alias.bin")).unwrap();
        assert!(
            actual == payload,
            "{options:?}: alias length {} expected {}",
            actual.len(),
            payload.len()
        );
        let mut expected = vec![0x57; 2048];
        expected.extend([0; 2048]);
        expected.extend([0x19; 3]);
        assert_eq!(fs::read(work.join("sparse.bin")).unwrap(), expected);
    }
}

#[test]
fn independent_udfinfo_recognizes_volume_revisions_and_profiles() {
    let executable = std::env::var_os("UDFINFO").unwrap_or_else(|| "udfinfo".into());
    if Command::new(&executable).arg("--help").output().is_err() {
        eprintln!("udfinfo unavailable: independent volume validation skipped (set UDFINFO)");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    for (index, (revision, partition, allocation)) in [
        (
            UdfRevision::V102,
            UdfPartition::Physical,
            AllocationMode::Short,
        ),
        (
            UdfRevision::V150,
            UdfPartition::Virtual,
            AllocationMode::Long,
        ),
        (
            UdfRevision::V200,
            UdfPartition::Sparable { packet_blocks: 32 },
            AllocationMode::Short,
        ),
        (
            UdfRevision::V201,
            UdfPartition::PhysicalSplit,
            AllocationMode::Long,
        ),
        (
            UdfRevision::V250,
            UdfPartition::Metadata { mirror: false },
            AllocationMode::Long,
        ),
        (
            UdfRevision::V260,
            UdfPartition::MetadataSparable {
                mirror: true,
                packet_blocks: 32,
            },
            AllocationMode::Long,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let path = temp.path().join(format!("volume-{index}.udf"));
        let mut image = UdfImage::new();
        image
            .add_bytes(
                "payload.txt",
                b"independent volume descriptor validation".to_vec(),
            )
            .unwrap();
        let options = UdfOptions {
            revision,
            partition,
            allocation,
            ..UdfOptions::default()
        };
        image.write(&path, &options).unwrap();
        let result = Command::new(&executable).arg(&path).output().unwrap();
        assert!(
            result.status.success(),
            "{options:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8_lossy(&result.stdout);
        let expected = format!(
            "udfrev={}.{:02x}",
            revision.number() >> 8,
            revision.number() & 255
        );
        assert!(
            output.contains(&expected),
            "{options:?}: expected {expected}: {output}"
        );
    }
}

#[test]
fn independent_udfclient_reads_relocated_packets_after_original_storage_is_destroyed() {
    let executable = std::env::var_os("UDFCLIENT").unwrap_or_else(|| "udfclient".into());
    if Command::new(&executable).output().is_err() {
        eprintln!("UDFclient unavailable: relocated packet check skipped (set UDFCLIENT)");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..8193).map(|index| (index % 251) as u8).collect();
    for (index, (revision, partition)) in [
        (
            UdfRevision::V201,
            UdfPartition::Sparable { packet_blocks: 32 },
        ),
        (
            UdfRevision::V260,
            UdfPartition::MetadataSparable {
                mirror: true,
                packet_blocks: 32,
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let work = temp.path().join(index.to_string());
        fs::create_dir(&work).unwrap();
        let mut image = UdfImage::new();
        image.add_bytes("payload.bin", payload.clone()).unwrap();
        let mut options = UdfOptions {
            revision,
            partition,
            allocation: AllocationMode::Long,
            ..UdfOptions::default()
        };
        let baseline = work.join("baseline.udf");
        image.write(&baseline, &options).unwrap();
        let bytes = fs::read(&baseline).unwrap();
        let offset = bytes
            .windows(payload.len())
            .position(|window| window == payload)
            .unwrap();
        let absolute_block = offset / 2048;
        let packet = ((absolute_block - 320) / 32 * 32) as u32;
        options.sparing_packets.push(packet);
        let path = work.join("relocated.udf");
        image.write(&path, &options).unwrap();
        let mut damaged = fs::read(&path).unwrap();
        let packet_offset = (320 + packet as usize) * 2048;
        damaged[packet_offset..packet_offset + 32 * 2048].fill(0xa5);
        fs::write(&path, damaged).unwrap();
        let listed = client(&executable, &path, "quit\n", &work);
        let listing = String::from_utf8_lossy(&listed.stdout);
        let mount = listing
            .lines()
            .find_map(|line| {
                if line.starts_with("drwx") {
                    line.split_whitespace()
                        .last()
                        .filter(|name| name.matches(':').count() >= 3)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| {
                panic!(
                    "{options:?}: no mounted filesystem: {listing}\n{}",
                    String::from_utf8_lossy(&listed.stderr)
                )
            });
        let commands = format!("get /{mount}/payload.bin extracted.bin\nquit\n");
        let result = client(&executable, &path, &commands, &work);
        let actual = fs::read(work.join("extracted.bin")).unwrap_or_else(|error| {
            panic!(
                "{options:?}: {error}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            )
        });
        assert!(
            actual == payload,
            "{options:?}: corrupted original storage still influenced payload"
        );
    }
}
