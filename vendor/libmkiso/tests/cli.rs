use std::process::Command;
fn mkiso() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mkiso"))
}
#[test]
fn global_json_is_order_independent_and_errors_are_structured() {
    for args in [
        vec!["--json", "inspect", "/missing-mkiso-image"],
        vec!["inspect", "/missing-mkiso-image", "--json"],
    ] {
        let out = mkiso().args(args).output().unwrap();
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(report["status"], "error");
        assert_eq!(out.status.code(), Some(5));
    }
}
#[test]
fn multiboot_init_and_add_validate_identifiers() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("menu.toml");
    let iso = temp.path().join("original.iso");
    std::fs::write(&iso, b"fixture").unwrap();
    assert!(
        mkiso()
            .arg("multiboot")
            .arg("init")
            .arg(&manifest)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        mkiso()
            .arg("multiboot")
            .arg("add")
            .arg(&manifest)
            .arg(&iso)
            .args(["--id", "debian", "--title", "Debian"])
            .status()
            .unwrap()
            .success()
    );
    let original = std::fs::read(&manifest).unwrap();
    assert_eq!(
        mkiso()
            .arg("multiboot")
            .arg("add")
            .arg(&manifest)
            .arg(&iso)
            .args(["--id", "debian", "--title", "Duplicate"])
            .status()
            .unwrap()
            .code(),
        Some(3)
    );
    assert_eq!(std::fs::read(&manifest).unwrap(), original);
}
#[test]
fn report_cannot_be_primary_output_even_when_it_does_not_exist() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("data.iso");
    let out = mkiso()
        .arg("create")
        .arg(temp.path())
        .arg("--output")
        .arg(&output)
        .arg("--report")
        .arg(&output)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(!output.exists());
}
#[cfg(not(feature = "progress"))]
#[test]
fn explicit_progress_requires_feature() {
    let out = mkiso()
        .args(["inspect", "/missing", "--progress", "always", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("--features progress")
    );
}

#[test]
fn create_verify_extract_preserves_source_and_json_stdout() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("hello.txt"), b"original payload").unwrap();
    let image = temp.path().join("data.iso");
    let create = mkiso()
        .arg("create")
        .arg(&root)
        .arg("--output")
        .arg(&image)
        .args([
            "--timestamp",
            "2026-10-06T00:00:00Z",
            "--joliet",
            "--reproducible",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        create.status.success(),
        "{}",
        String::from_utf8_lossy(&create.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    assert_eq!(result["status"], "ok");
    let verify = mkiso()
        .arg("verify")
        .arg(&image)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    let extracted = temp.path().join("extracted");
    let extract = mkiso()
        .arg("extract")
        .arg(&image)
        .arg("--output")
        .arg(&extracted)
        .output()
        .unwrap();
    assert!(
        extract.status.success(),
        "{}",
        String::from_utf8_lossy(&extract.stderr)
    );
    assert_eq!(
        std::fs::read(extracted.join("hello.txt")).unwrap(),
        b"original payload"
    );
    assert_eq!(
        std::fs::read(root.join("hello.txt")).unwrap(),
        b"original payload"
    );
}

#[test]
fn boot_prepare_preserves_assets_and_initramfs_order() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    std::fs::write(assets.join("bootx64.efi"), b"original signed bytes").unwrap();
    let kernel = temp.path().join("kernel");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    for p in [&kernel, &first, &second] {
        std::fs::write(p, b"payload").unwrap();
    }
    let output = temp.path().join("prepared");
    let result = mkiso()
        .args(["boot", "prepare", "--loader", "grub", "--arch", "x86_64"])
        .arg("--kernel")
        .arg(&kernel)
        .arg("--initrd")
        .arg(&first)
        .arg("--initrd")
        .arg(&second)
        .arg("--assets")
        .arg(&assets)
        .arg("--output")
        .arg(&output)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read(output.join("assets/bootx64.efi")).unwrap(),
        b"original signed bytes"
    );
    assert!(
        std::fs::read_to_string(output.join("boot/grub/grub.cfg"))
            .unwrap()
            .contains("initrd /boot/initrd-0 /boot/initrd-1")
    );
}

#[test]
fn saved_plan_build_and_manifest_verification_detect_payload_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("hello.txt"), b"payload").unwrap();
    let manifest = temp.path().join("image.toml");
    std::fs::write(&manifest,"version=1\n[image]\nsource='root'\noutput='data.iso'\n[filesystem]\ntype='iso9660'\njoliet=true\n[reproducibility]\ntimestamp='2026-10-06T00:00:00Z'\n").unwrap();
    let plan = mkiso()
        .arg("plan")
        .arg(&manifest)
        .arg("--json")
        .output()
        .unwrap();
    assert!(plan.status.success());
    let saved = temp.path().join("plan.json");
    std::fs::write(&saved, &plan.stdout).unwrap();
    let build = mkiso()
        .arg("build")
        .arg(&saved)
        .arg("--reproducible")
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let image = temp.path().join("data.iso");
    let verify = mkiso()
        .arg("verify")
        .arg(&image)
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .unwrap();
    assert!(
        verify.status.success(),
        "{}",
        String::from_utf8_lossy(&verify.stderr)
    );
    std::fs::write(root.join("hello.txt"), b"changed").unwrap();
    assert_eq!(
        mkiso()
            .arg("verify")
            .arg(&image)
            .arg("--manifest")
            .arg(&manifest)
            .output()
            .unwrap()
            .status
            .code(),
        Some(6)
    );
    assert_eq!(
        mkiso()
            .arg("build")
            .arg(&saved)
            .arg("--replace")
            .output()
            .unwrap()
            .status
            .code(),
        Some(6)
    );
}

#[test]
fn build_override_cannot_replace_manifest_or_its_hardlink() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a"), b"data").unwrap();
    let manifest = temp.path().join("image.toml");
    let original =
        b"version=1\n[image]\nsource='root'\noutput='data.iso'\n[filesystem]\ntype='iso9660'\n";
    std::fs::write(&manifest, original).unwrap();
    let alias = temp.path().join("alias.toml");
    std::fs::hard_link(&manifest, &alias).unwrap();
    for output in [&manifest, &alias] {
        assert_eq!(
            mkiso()
                .arg("build")
                .arg(&manifest)
                .arg("--output")
                .arg(output)
                .arg("--replace")
                .output()
                .unwrap()
                .status
                .code(),
            Some(3)
        );
        assert_eq!(std::fs::read(&manifest).unwrap(), original);
    }
}

#[test]
fn json_usage_errors_keep_stdout_parseable() {
    let result = mkiso()
        .args(["inspect", "missing.iso", "--unknown-option", "--json"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["error"]["code"], "usage");
}

#[test]
fn firmware_report_cannot_replace_tpm_executable() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("media.img");
    let tpm = temp.path().join("swtpm");
    std::fs::write(&image, b"image").unwrap();
    std::fs::write(&tpm, b"executable bytes").unwrap();
    let out = mkiso()
        .args(["multiboot", "test"])
        .arg(&image)
        .args(["--firmware", "uefi", "--evidence-dir"])
        .arg(temp.path().join("evidence"))
        .arg("--swtpm")
        .arg(&tpm)
        .arg("--report")
        .arg(&tpm)
        .args(["--replace", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(std::fs::read(tpm).unwrap(), b"executable bytes");
}

#[test]
fn multiboot_report_cannot_mutate_manifest_assets() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    std::fs::create_dir(&assets).unwrap();
    let manifest = temp.path().join("menu.toml");
    std::fs::write(&manifest, "version=1\n[menu]\ntitle='menu'\n[boot]\nbackend='ventoy'\nfirmware=['bios']\nassets='assets'\n[[entries]]\nid='debian'\ntitle='Debian'\nimage='missing.iso'\n").unwrap();
    let report = assets.join("new-report.json");
    let out = mkiso()
        .args(["multiboot", "plan"])
        .arg(&manifest)
        .args(["--target", "usb", "--report"])
        .arg(&report)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(!report.exists());
}
