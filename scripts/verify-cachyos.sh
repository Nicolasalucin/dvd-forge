#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

failures=0
check_binary() {
    local name="$1"
    if command -v "$name" >/dev/null 2>&1; then
        printf '[OK]    %s -> %s\n' "$name" "$(command -v "$name")"
    else
        printf '[FALTA] %s\n' "$name"
        failures=$((failures + 1))
    fi
}

echo "== DVD Forge: verificación de CachyOS =="
echo "Directorio: $ROOT"
echo

for tool in ffmpeg ffprobe dvdauthor spumux xorriso growisofs dvd+rw-mediainfo eject; do
    check_binary "$tool"
done

if command -v gst-inspect-1.0 >/dev/null 2>&1; then
    missing_gst=()
    for element in autoaudiosink autovideosink; do
        if ! gst-inspect-1.0 "$element" >/dev/null 2>&1; then
            missing_gst+=("$element")
        fi
    done
    if [[ ${#missing_gst[@]} -eq 0 ]]; then
        printf '[OK]    GStreamer -> autoaudiosink + autovideosink\n'
    else
        printf '[AVISO] GStreamer no encuentra %s; instala gst-plugins-good para la vista previa\n' "${missing_gst[*]}"
    fi
else
    printf '[AVISO] gst-inspect-1.0 no encontrado; la vista previa de vídeo quedará desactivada\n'
fi

if command -v genisoimage >/dev/null 2>&1; then
    printf '[OK]    genisoimage -> %s (fallback de -dvd-video)\n' "$(command -v genisoimage)"
elif command -v mkisofs >/dev/null 2>&1; then
    printf '[OK]    mkisofs -> %s (fallback de -dvd-video)\n' "$(command -v mkisofs)"
else
    printf '[AVISO] genisoimage/mkisofs no encontrado; xorriso puede no aceptar -dvd-video\n'
fi

if [[ -x /usr/local/bin/tsMuxeR ]]; then
    TSMUXER=/usr/local/bin/tsMuxeR
elif command -v tsMuxeR >/dev/null 2>&1; then
    TSMUXER="$(command -v tsMuxeR)"
else
    TSMUXER=""
fi

if [[ -n "$TSMUXER" ]]; then
    banner="$("$TSMUXER" --version 2>&1 || true)"
    if grep -qi 'tsMuxeR version' <<<"$banner"; then
        printf '[OK]    tsMuxeR -> %s (%s)\n' "$TSMUXER" "$(grep -io 'tsMuxeR version [0-9.]*' <<<"$banner" | head -1)"
    else
        printf '[FALTA] tsMuxeR no mostró su banner de versión\n'
        failures=$((failures + 1))
    fi
else
    printf '[FALTA] tsMuxeR\n'
    failures=$((failures + 1))
fi

echo
if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "Instala las dependencias con:"
    echo "  sudo pacman -S --needed ffmpeg dvdauthor dvd+rw-tools libisoburn udftools"
fi

if [[ "$failures" -ne 0 ]]; then
    echo
    echo "Hay $failures herramienta(s) ausente(s)."
    exit 1
fi

echo "== Ejecutando dvdforge-cli doctor --json =="
cargo run --quiet -p dvdforge-cli -- doctor --json

echo
echo "Toolchain verificado."
