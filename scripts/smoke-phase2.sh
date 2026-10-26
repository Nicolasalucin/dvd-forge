#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Fase 2: formato, clippy y tests =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo
echo "== Detección de unidades ópticas =="
DEVICES_JSON="$(cargo run --quiet -p dvdforge-cli -- devices --json)"
printf '%s\n' "$DEVICES_JSON" | jq -e 'type == "array"' >/dev/null
COUNT="$(printf '%s\n' "$DEVICES_JSON" | jq 'length')"
printf 'Unidades detectadas: %s\n' "$COUNT"
printf '%s\n' "$DEVICES_JSON" | jq .

echo
echo "Fase 2 verificada: el listado de unidades devuelve un array JSON válido."
