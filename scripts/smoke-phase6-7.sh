#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Formato Rust =="
cargo fmt --all -- --check

echo
echo "== Clippy del workspace =="
cargo clippy --workspace --all-targets -- -D warnings

echo
echo "== Tests del workspace =="
cargo test --workspace

echo
echo "== Tests específicos Fase 6/7 =="
echo "-- ICS multi-página y un BOG por botón"
cargo test -q -p dvdforge-hdmv multiple_pages
echo "-- Perfiles Blu-ray y guardas de grabación BD-25/BD-50"
cargo test -q -p dvdforge-burn
echo "-- Caché SQLite de artefactos"
cargo test -q -p dvdforge-store sqlite_cache_round_trips_and_rejects_missing_files

echo
echo "== Bindings, svelte-check y build =="
cd "$ROOT/apps/desktop"
npm run bindings
npm run check
npm run build
cd "$ROOT"

echo
echo "== DVD: región, subtítulos y menú animado =="
cargo test -p dvdforge-render --test render_smoke \
  renders_dvd_with_region_mask -- --ignored --nocapture
cargo test -p dvdforge-render --test render_smoke \
  renders_fixture_with_external_subtitle_to_iso -- --ignored --nocapture
cargo test -p dvdforge-render --test render_smoke \
  renders_motion_menu_with_video_button_to_iso -- --ignored --nocapture

echo
echo "== AVCHD sin menús con UDF 2.50 metadata partition =="
cargo test -p dvdforge-render --test render_smoke \
  renders_menu_less_avchd_with_metadata_udf -- --ignored --nocapture

echo
echo "== Render BD-25 con menú HDMV IG =="
cargo test -p dvdforge-render --test render_smoke \
  renders_bd25_with_native_hdmv_menu -- --ignored --nocapture

echo
echo "== Render AVCHD DVD5 con menú HDMV IG =="
cargo test -p dvdforge-render --test render_smoke \
  renders_avchd_dvd5_with_native_hdmv_menu -- --ignored --nocapture

echo
echo "== Render BD-25 con menús HDMV anidados y audio =="
cargo test -p dvdforge-render --test render_smoke \
  renders_bd25_with_nested_hdmv_menus -- --ignored --nocapture

echo
echo "== Navegación HDMV anidada con libbluray =="
if pkg-config --exists libbluray; then
  PROBE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dvdforge-hdmv-probe.XXXXXX")"
  trap 'rm -rf "$PROBE_DIR"' EXIT
  cargo run -q -p dvdforge-cli -- render \
    fixtures/projects/bd25-nested-menu.json -o "$PROBE_DIR/out" --no-probe >/dev/null
  if command -v 7z >/dev/null 2>&1; then
    if 7z l "$PROBE_DIR/out/disc.iso" | grep -q "IsMetadata: 1"; then
      echo "UDF 2.50 con metadata partition confirmada por 7z."
    else
      echo "ERROR: el ISO no expone una metadata partition UDF 2.50." >&2
      exit 1
    fi
  fi
  cc -O2 -o "$PROBE_DIR/probe" scripts/hdmv-nested-probe.c \
    $(pkg-config --cflags --libs libbluray)
  "$PROBE_DIR/probe" "$PROBE_DIR/out/disc.iso"
  rm -rf "$PROBE_DIR"
  trap - EXIT
else
  echo "libbluray no está instalado; se omite la comprobación de navegación."
fi

echo
echo "OK: fase 6/7 verificada."
