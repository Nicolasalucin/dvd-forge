#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_DIR="$ROOT/fixtures/projects/assets"
FFMPEG="${FFMPEG:-ffmpeg}"

if ! command -v "$FFMPEG" >/dev/null 2>&1; then
    echo "No se encontró ffmpeg. Instálalo antes de generar los fixtures." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"

echo "Generando fixtures en $OUT_DIR"

"$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc2=size=1280x720:rate=30000/1001:duration=20" \
    -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=20" \
    -c:v libx264 -preset ultrafast -crf 28 -pix_fmt yuv420p \
    -c:a aac -b:a 192k -ar 48000 -ac 2 -shortest \
    "$OUT_DIR/episode-01.mp4"

"$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "testsrc=size=1280x720:rate=30000/1001:duration=15" \
    -f lavfi -i "sine=frequency=554:sample_rate=48000:duration=15" \
    -c:v libx264 -preset ultrafast -crf 28 -pix_fmt yuv420p \
    -c:a aac -b:a 192k -ar 48000 -ac 2 -shortest \
    "$OUT_DIR/episode-02.mp4"

"$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "smptebars=size=1280x720:rate=30000/1001:duration=5" \
    -f lavfi -i "sine=frequency=330:sample_rate=48000:duration=5" \
    -c:v libx264 -preset ultrafast -crf 28 -pix_fmt yuv420p \
    -c:a aac -b:a 192k -ar 48000 -ac 2 -shortest \
    "$OUT_DIR/intro.mp4"

"$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "color=c=0x101828:s=1920x1080:d=1" \
    -frames:v 1 "$OUT_DIR/background.png"

"$FFMPEG" -hide_banner -loglevel error -y \
    -f lavfi -i "sine=frequency=220:sample_rate=48000:duration=30" \
    -ac 2 "$OUT_DIR/menu-music.wav"

echo
echo "Fixtures generados:"
find "$OUT_DIR" -maxdepth 1 -type f -printf '  %f  %s bytes\n' | sort
