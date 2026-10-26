# Repository guidelines

The standalone Rust 2024 package is named libmkiso. Preserve public
API names, MIT licensing, fixtures, and historical validation evidence.
Run `cargo fmt -- --check`, `cargo test --all-features --locked`,
`cargo test --no-default-features --locked`, and
`cargo clippy --all-targets --all-features --locked -- -D warnings`.
Check the portable reader with `cargo check --no-default-features --target wasm32-unknown-unknown --locked`.
Independent readers may skip when tools are absent. Large-file ignored tests
write/extract over 8 GiB. Firmware and Windows installation correctness require
separate disposable-environment gates; a successful image write is not proof.

The mkiso binary requires `cli`; `progress` enables optional terminal bars.
Media orchestration lives in the cli-gated boot_media module. Test the cli
feature without progress as well as all features to preserve both paths.
