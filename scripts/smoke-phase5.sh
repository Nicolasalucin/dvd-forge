#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Frontend Tauri =="
cd "$ROOT/apps/desktop"
npm run check
npm run build
cd "$ROOT"

echo
echo "== Formato Rust =="
cargo fmt --all -- --check

echo
echo "== Clippy del workspace =="
cargo clippy --workspace --all-targets -- -D warnings

echo
echo "== Tests del workspace =="
cargo test --workspace

echo
echo "== Tests de parsers macOS (ejecutables en Linux) =="
cargo test -p dvdforge-burn --lib macos:: -- --nocapture

echo
echo "== Tests de lógica IMAPI2 (ejecutables en Linux) =="
cargo test -p dvdforge-burn --lib windows:: -- --nocapture

echo
echo "== Tests de resolución de toolchain por plataforma =="
cargo test -p dvdforge-toolchain -- --nocapture

echo
echo "== Build del workspace =="
cargo build --workspace


echo
echo "== Doctor de herramientas en CachyOS =="
mkdir -p build
cargo run -q -p dvdforge-cli -- doctor --json > build/toolchain-phase5.json
python3 - <<'PY'
import json
from pathlib import Path
report = json.loads(Path("build/toolchain-phase5.json").read_text())
if not report.get("ok"):
    missing = report.get("missingRequired") or report.get("missing_required") or []
    raise SystemExit(f"toolchain incompleto: {missing}")
print(f"Toolchain OK: {report.get('platform')} ({len(report.get('tools', []))} herramientas)")
PY

echo
echo "Fase 5 verificada: macOS, Windows/IMAPI2, toolchain multiplataforma y CI."
