fn main() {
    tauri_build::build();
    link_windows_resource_into_examples();
}

/// `tauri_build` links the Windows resource (with the common-controls v6
/// manifest) into binaries only. The `export-bindings` example links the
/// same UI libraries and fails to start without it
/// (STATUS_ENTRYPOINT_NOT_FOUND), so the example gets the same resource.
fn link_windows_resource_into_examples() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Ok(out_dir) = std::env::var("OUT_DIR") else {
        return;
    };
    let out_dir = std::path::Path::new(&out_dir);
    for name in ["resource.lib", "libresource.a"] {
        let resource = out_dir.join(name);
        if resource.is_file() {
            println!("cargo:rustc-link-arg-examples={}", resource.display());
            return;
        }
    }
}
