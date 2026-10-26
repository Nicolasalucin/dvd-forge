#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Fase 1.2: formato, clippy y tests =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo
echo "== Render completo del fixture =="
cargo run --quiet -p dvdforge-cli -- render \
  fixtures/projects/minimal.json \
  -o build/out

test -f build/out/disc.iso
test "$(stat -c%s build/out/disc.iso)" -gt 0

echo
echo "== Inspección del ISO con xorriso =="
xorriso -indev build/out/disc.iso \
  -find / -maxdepth 2 -exec lsdl > build/out/iso-list.txt

grep -q 'VIDEO_TS/VIDEO_TS.IFO' build/out/iso-list.txt
grep -q 'VIDEO_TS/VTS_01_0.IFO' build/out/iso-list.txt
jq -e '.hasVideoTsIfo == true and .hasVts01Ifo == true' build/out/render.json

echo
echo "== Reproducción con mpv =="
# En el mpv/libdvdnav de CachyOS, dvd://1 devuelve "Title out of range";
# el primer título se abre con dvd://0. Se intenta primero la forma pedida
# y se cae al índice 0 si esta combinación de mpv lo requiere.
if [[ -n "${MPV_URL:-}" ]]; then
    mpv --no-config --vo=null --ao=null --frames=100 \
      "$MPV_URL" --dvd-device=build/out/disc.iso
elif ! mpv --no-config --vo=null --ao=null --frames=100 \
      dvd://1 --dvd-device=build/out/disc.iso; then
    echo "dvd://1 no está disponible en este mpv/libdvdnav; usando dvd://0."
    mpv --no-config --vo=null --ao=null --frames=100 \
      dvd://0 --dvd-device=build/out/disc.iso
fi

echo
echo "Fase 1.2 verificada: ISO reproducible y reproducible con mpv."
