#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROJECT="fixtures/projects/avchd-dvd5.json"
OUT="build/avchd"
ISO="$OUT/disc.iso"

echo "== Frontend Tauri: bindings, svelte-check y build =="
cd "$ROOT/apps/desktop"
npm run bindings
rg -q 'projectNew' src/lib/api/bindings.ts
rg -q 'avchdFrameRate' src/lib/api/bindings.ts
npm run check
npm run build
cd "$ROOT"

echo
echo "== Fase 4: formato, clippy, tests y build del workspace =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace

echo
echo "== Render AVCHD end-to-end =="
rm -rf "$OUT"
cargo run -q -p dvdforge-cli -- render "$PROJECT" --mode avchd -o "$OUT"
test -f "$ISO"

echo
echo "== Estructura UDF/BDMV del ISO =="
XORRISO_TREE=""
if command -v xorriso >/dev/null 2>&1; then
    set +e
    XORRISO_TREE=$(xorriso -indev "$ISO" -find / -maxdepth 2 -exec lsdl 2>/dev/null)
    XORRISO_STATUS=$?
    set -e
    echo "xorriso exit=$XORRISO_STATUS"
fi

if grep -q 'BDMV' <<<"$XORRISO_TREE"; then
    TREE="$XORRISO_TREE"
    echo "Árbol leído con xorriso."
elif command -v 7z >/dev/null 2>&1; then
    echo "xorriso no enumera este ISO UDF 2.50; se usa 7z para inspeccionar."
    TREE=$(7z l "$ISO")
    grep -q 'Version = 2.50' <<<"$TREE"
else
    echo "No hay xorriso con soporte UDF ni 7z para inspeccionar el ISO." >&2
    exit 1
fi

for required in \
    "BDMV/index.bdmv" \
    "BDMV/MovieObject.bdmv" \
    "BDMV/PLAYLIST" \
    "BDMV/CLIPINF" \
    "BDMV/STREAM"
do
    if ! grep -q "$required" <<<"$TREE"; then
        echo "Falta $required en el ISO AVCHD." >&2
        exit 1
    fi
done
echo "Estructura AVCHD OK: index.bdmv, MovieObject.bdmv, PLAYLIST, CLIPINF y STREAM."

echo
echo "== Capítulos inyectados en el metafile =="
META="$OUT/meta/avchd.meta"
grep -q -- '--custom-chapters=' "$META"
CHAPTER_COUNT=$(( $(grep -o ';' "$META" | wc -l) + 1 ))
if [[ $CHAPTER_COUNT -lt 2 ]]; then
    echo "Se esperaban al menos dos capítulos; encontrados: $CHAPTER_COUNT" >&2
    exit 1
fi
echo "Capítulos AVCHD: $CHAPTER_COUNT."

echo
echo "== Perfil H.264 del stream AVCHD =="
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT
if command -v 7z >/dev/null 2>&1; then
    7z x -o"$TMP_DIR" "$ISO" 'BDMV/STREAM/00000.m2ts' >/dev/null
    ffprobe -v error -select_streams v:0 \
        -show_entries stream=codec_name,profile,level,width,height,r_frame_rate \
        -of default=nw=1 "$TMP_DIR/BDMV/STREAM/00000.m2ts" | tee "$TMP_DIR/ffprobe.txt"
    grep -q 'codec_name=h264' "$TMP_DIR/ffprobe.txt"
    grep -q 'profile=High' "$TMP_DIR/ffprobe.txt"
    grep -q 'level=41' "$TMP_DIR/ffprobe.txt"
    grep -q 'width=1280' "$TMP_DIR/ffprobe.txt"
    grep -q 'height=720' "$TMP_DIR/ffprobe.txt"
    grep -q 'r_frame_rate=24000/1001' "$TMP_DIR/ffprobe.txt"
fi

echo
echo "== Reproducción de prueba con mpv =="
ABS_ISO=$(realpath "$ISO")
MPV_LOG="$TMP_DIR/mpv.log"
set +e
timeout 30 mpv --no-config --vo=null --ao=null --frames=60 "$ABS_ISO" >"$MPV_LOG" 2>&1
MPV_STATUS=$?
set -e
if [[ $MPV_STATUS -ne 0 ]]; then
    echo "mpv no abre el ISO UDF directamente; se usa el dispositivo Blu-ray virtual."
    set +e
    timeout 60 mpv --no-config --vo=null --ao=null --frames=60 \
        --bluray-device="$ABS_ISO" bd:// >"$MPV_LOG" 2>&1
    MPV_STATUS=$?
    set -e
fi
if [[ $MPV_STATUS -ne 0 ]]; then
    cat "$MPV_LOG"
    echo "mpv no pudo reproducir el ISO AVCHD." >&2
    exit 1
fi
tail -5 "$MPV_LOG"

echo
echo "Fase 4 verificada: H.264 High@4.1, UDF 2.50, BDMV, capítulos y reproducción."
