#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

rustup target add i686-unknown-linux-gnu i686-pc-windows-msvc >/dev/null 2>&1 || true

echo "== 32-bit Linux: CLI and authoring stack =="
cargo check -p dvdforge-cli --target i686-unknown-linux-gnu

echo
echo "== 32-bit Windows: IMAPI2 burn backend =="
cargo check -p dvdforge-burn --target i686-pc-windows-msvc

echo
echo "OK: comprobaciones de compilación de 32 bits completadas."
