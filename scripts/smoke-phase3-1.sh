#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/apps/desktop"

echo "== Etapa 3.1: bindings TypeScript =="
npm run bindings
rg -q 'export type Project' src/lib/api/bindings.ts
rg -q 'export type BitratePlan' src/lib/api/bindings.ts
rg -q 'export type OpticalDevice' src/lib/api/bindings.ts
rg -q 'export type MediaProbe' src/lib/api/bindings.ts
rg -q 'export type JobEventKind' src/lib/api/bindings.ts
rg -q 'burnListDevices' src/lib/api/bindings.ts

echo
echo "== Svelte/TypeScript =="
npm run check
npm run build

echo
echo "== Backend Rust =="
cd "$ROOT"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

if compgen -G "/dev/sr*" >/dev/null; then
  echo
  echo "== Comando burn_list_devices con hardware =="
  cargo test -p dvdforge-desktop --test commands_smoke -- --ignored
else
  echo
  echo "Sin /dev/sr*: se omite la prueba de hardware."
fi

echo
echo "Etapa 3.1 verificada."
