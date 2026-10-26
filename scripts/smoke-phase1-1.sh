#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== Etapa 1.1: formato, clippy y tests =="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo
echo "== Plan de bitrate del fixture =="
cargo run --quiet -p dvdforge-cli -- plan \
  fixtures/projects/minimal.json \
  -o build/plan.json

jq -e '.feasible == true and .fillPpm >= 980000' build/plan.json

if [[ -f fixtures/projects/assets/episode-01.mp4 ]]; then
    echo
    echo "== FFprobe sobre el fixture generado =="
    cargo run --quiet -p dvdforge-cli -- probe \
      fixtures/projects/assets/episode-01.mp4 \
      --json > build/probe.json
    jq -e '.video.width == 1280 and .audio[0].sampleRate == 48000' build/probe.json

    echo
    echo "== Codificación corta MPEG-2 + AC-3 + DVD-PS =="
    cargo test -p dvdforge-dvd --test encode_smoke -- --ignored
else
    echo
    echo "fixtures/projects/assets/episode-01.mp4 no existe; se omite la prueba de codificación."
    echo "Ejecuta ./scripts/generate-fixtures.sh para generarlo."
fi

echo
echo "Etapa 1.1 verificada."
