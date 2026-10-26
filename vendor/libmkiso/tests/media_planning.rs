use libmkiso::boot_media::{
    manifest::{read_manifest, read_multiboot},
    plan::{plan_manifest, recheck_inputs},
};
use std::fs;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("tree")).unwrap();
    fs::write(temp.path().join("tree/file.txt"), b"original").unwrap();
    let manifest = temp.path().join("image.toml");
    fs::write(&manifest, "version = 1\n[image]\nsource = 'tree'\noutput = 'image.iso'\n[filesystem]\ntype = 'iso9660'\n").unwrap();
    (temp, manifest)
}

#[test]
fn plan_resolves_relative_paths_and_does_not_write_media() {
    let (temp, path) = fixture();
    let plan = plan_manifest(&path).unwrap();
    assert_eq!(
        plan.source,
        fs::canonicalize(temp.path().join("tree")).unwrap()
    );
    assert_eq!(plan.payload_bytes, 8);
    assert!(plan.unsupported.is_empty());
    assert!(!temp.path().join("image.iso").exists());
    recheck_inputs(&plan).unwrap();
}

#[test]
fn saved_plan_rejects_added_removed_and_same_size_changed_inputs() {
    let (temp, path) = fixture();
    let plan = plan_manifest(&path).unwrap();
    fs::write(temp.path().join("tree/file.txt"), b"modified").unwrap();
    assert!(recheck_inputs(&plan).is_err());
    fs::write(temp.path().join("tree/file.txt"), b"original").unwrap();
    fs::write(temp.path().join("tree/new.txt"), b"new").unwrap();
    assert!(recheck_inputs(&plan).is_err());
    fs::remove_file(temp.path().join("tree/new.txt")).unwrap();
    fs::remove_file(temp.path().join("tree/file.txt")).unwrap();
    assert!(recheck_inputs(&plan).is_err());
}

#[test]
fn unknown_fields_and_versions_are_rejected() {
    let (_temp, path) = fixture();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("version = 1", "version = 2")).unwrap();
    assert!(read_manifest(&path).is_err());
    fs::write(&path, format!("{original}typo = true\n")).unwrap();
    assert!(read_manifest(&path).is_err());
}

#[test]
fn duplicate_multiboot_ids_fail_before_image_reads() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("multi.toml");
    fs::write(&path, "version=1\n[menu]\ntitle='Menu'\n[boot]\nbackend='ventoy'\nfirmware=['bios','uefi']\nassets='assets'\n[[entries]]\nid='debian'\ntitle='Debian'\nimage='a.iso'\n[[entries]]\nid='debian'\ntitle='Other'\nimage='b.iso'\n").unwrap();
    let error = read_multiboot(&path).unwrap_err();
    assert!(error.to_string().contains("duplicate"));
}

#[cfg(unix)]
#[test]
fn output_hard_link_to_input_is_rejected() {
    let (temp, path) = fixture();
    fs::hard_link(
        temp.path().join("tree/file.txt"),
        temp.path().join("image.iso"),
    )
    .unwrap();
    assert!(plan_manifest(&path).is_err());
}

#[test]
fn boot_path_traversal_is_rejected() {
    let (_temp, path) = fixture();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{original}[boot]\nprofile='linux-grub'\narch='x86_64'\n[[boot.entries]]\nid='bios'\nfirmware='bios'\nimage='../outside'\n")).unwrap();
    assert!(read_manifest(&path).is_err());
}

#[test]
fn bridge_profile_is_reported_as_unavailable() {
    let (_temp, path) = fixture();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("'iso9660'", "'iso9660+udf'")).unwrap();
    assert!(!plan_manifest(&path).unwrap().unsupported.is_empty());
}

#[test]
fn saved_plan_rejects_empty_directory_and_resolved_setting_changes() {
    let (temp, path) = fixture();
    let plan = plan_manifest(&path).unwrap();
    let mut altered = plan.clone();
    altered.output = temp.path().join("different.iso");
    assert!(recheck_inputs(&altered).is_err());
    let mut altered = plan.clone();
    altered.rock_ridge = !altered.rock_ridge;
    assert!(recheck_inputs(&altered).is_err());
    fs::create_dir(temp.path().join("tree/empty")).unwrap();
    assert!(recheck_inputs(&plan).is_err());
}

#[test]
fn saved_plan_cannot_clear_unsupported_capability_gates() {
    let (_temp, path) = fixture();
    let original = fs::read_to_string(&path).unwrap();
    fs::write(&path, original.replace("'iso9660'", "'iso9660+udf'")).unwrap();
    let mut plan = plan_manifest(&path).unwrap();
    plan.unsupported.clear();
    assert!(recheck_inputs(&plan).is_err());
}

#[cfg(unix)]
#[test]
fn saved_plan_rejects_permission_changes_and_identical_replacement_files() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, path) = fixture();
    let source = temp.path().join("tree/file.txt");
    let plan = plan_manifest(&path).unwrap();
    let mode = fs::metadata(&source).unwrap().permissions().mode();
    fs::set_permissions(&source, fs::Permissions::from_mode(mode ^ 0o100)).unwrap();
    assert!(recheck_inputs(&plan).is_err());
    let plan = plan_manifest(&path).unwrap();
    let replacement = temp.path().join("replacement");
    fs::write(&replacement, b"original").unwrap();
    fs::rename(&replacement, &source).unwrap();
    assert!(recheck_inputs(&plan).is_err());
}

#[test]
fn planning_progress_counts_hashed_bytes_and_discovered_entries() {
    use libmkiso::boot_media::{
        optical::OperationContext,
        plan::plan_manifest_with_context,
        progress::{CancellationToken, Phase, ProgressEvent, ProgressState},
    };
    let (_temp, path) = fixture();
    let mut events = Vec::new();
    let mut observer = |event: &ProgressEvent| events.push(event.clone());
    let cancellation = CancellationToken::default();
    let plan = plan_manifest_with_context(
        &path,
        &mut OperationContext {
            observer: &mut observer,
            cancellation: &cancellation,
            operation_id: 0,
        },
    )
    .unwrap();
    let finished_hashes: u64 = events
        .iter()
        .filter(|e| e.phase == Phase::HashInputs && e.state == ProgressState::Finished)
        .map(|e| e.completed)
        .sum();
    assert_eq!(
        finished_hashes,
        plan.inputs.iter().map(|input| input.size).sum::<u64>()
    );
    assert!(events.iter().any(|e| e.phase == Phase::ScanSource
        && e.state == ProgressState::Finished
        && e.completed == 1));
}

#[test]
fn planning_hash_cancellation_emits_terminal_cancelled_event() {
    use libmkiso::boot_media::{
        Error,
        optical::OperationContext,
        plan::plan_manifest_with_context,
        progress::{CancellationToken, Phase, ProgressEvent, ProgressState},
    };
    let (_temp, path) = fixture();
    let cancellation = CancellationToken::default();
    let mut events = Vec::new();
    let mut observer = |event: &ProgressEvent| {
        if event.phase == Phase::HashInputs && event.state == ProgressState::Started {
            cancellation.cancel();
        }
        events.push(event.clone());
    };
    let result = plan_manifest_with_context(
        &path,
        &mut OperationContext {
            observer: &mut observer,
            cancellation: &cancellation,
            operation_id: 0,
        },
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(
        events
            .iter()
            .any(|e| e.phase == Phase::HashInputs && e.state == ProgressState::Cancelled)
    );
    assert!(
        events
            .iter()
            .any(|e| e.phase == Phase::ScanSource && e.state == ProgressState::Cancelled)
    );
}
