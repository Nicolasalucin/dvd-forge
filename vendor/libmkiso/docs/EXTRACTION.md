# Repository extraction — 2026-10-06

The former optical-image crate is now libmkiso. Its package includes the mkiso
binary with required feature cli. Media orchestration from boot-media is now
libmkiso::boot_media, gated by cli. Feature progress enables cli and indicatif;
default library builds do not enable CLI dependencies. Existing safety checks,
media operations, error codes, and feature-specific tests are retained.

Staged validation passed all-feature library/CLI/media tests and Clippy, CLI
tests without terminal progress, no-default-feature tests, the portable WASM
reader build, and two firmware-harness unit tests. The dependency tree without
default features contains only thiserror and its proc-macro dependencies.
Firmware boots, physical device writes, Windows installation and ignored
large-file tests were not rerun. Historical reports keep their original names.

Installed-package validation also passed after recovery: all-feature tests and
Clippy, cli-without-progress tests, no-default-feature tests, the library-only
WASM check, and firmware-harness unit tests. Cargo metadata confirms the mkiso
binary requires cli and CLI dependencies are optional.
