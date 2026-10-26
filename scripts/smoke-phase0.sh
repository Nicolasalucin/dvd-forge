#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Fase 0: formato, clippy y tests =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo
echo "== Fase 0: doctor =="
cargo run --quiet -p dvdforge-cli -- doctor --json
