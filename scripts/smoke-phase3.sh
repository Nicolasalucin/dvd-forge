#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/apps/desktop"

echo "== Fase 3: bindings TypeScript =="
npm run bindings
rg -q 'renderStart' src/lib/api/bindings.ts
rg -q 'burnStart' src/lib/api/bindings.ts
rg -q 'openFolder' src/lib/api/bindings.ts
rg -q 'menuSaveBackground' src/lib/api/bindings.ts
rg -q 'mediaPlaybackSupport' src/lib/api/bindings.ts
rg -q 'mediaPreviewUrl' src/lib/api/bindings.ts

echo
echo "== Svelte/TypeScript y build Vite =="
npm run check
npm run build

echo
echo "== Backend Rust =="
cd "$ROOT"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

echo
echo "== Compilación completa de la aplicación Tauri (sin bundle) =="
cd "$ROOT/apps/desktop"
npm run tauri -- build --no-bundle --debug
cd "$ROOT"

echo
echo "== Componentes del taller y producción =="
test -f apps/desktop/src/lib/editor/CanvasStage.svelte
test -f apps/desktop/src/lib/editor/exportMenu.ts
test -f apps/desktop/src/lib/editor/buttonLayout.ts
test -f apps/desktop/src/lib/editor/layers/ButtonsLayer.ts
test -f apps/desktop/src/lib/editor/layers/NavigationLayer.ts
test -f apps/desktop/src/lib/components/StudioHeader.svelte
test -f apps/desktop/src/lib/components/MediaPanel.svelte
test -f apps/desktop/src/lib/components/MenuWorkspace.svelte
test -f apps/desktop/src/lib/components/TrimmerStudio.svelte
test -f apps/desktop/src/lib/components/Simulator.svelte
test -f apps/desktop/src/lib/components/VmMap.svelte
test -f apps/desktop/src/lib/components/TracksPanel.svelte
test -f apps/desktop/src/lib/components/MotionPanel.svelte
test -f apps/desktop/src/lib/components/VbrPanel.svelte
test -f apps/desktop/src/lib/components/Inspector.svelte
test -f apps/desktop/src/lib/components/StoryboardBar.svelte
test -f apps/desktop/src/lib/components/ProductionModal.svelte
test -f apps/desktop/src/lib/components/NewProjectModal.svelte
test -f apps/desktop/src/lib/production.ts
test -f apps/desktop/src/lib/playback.ts
rg -q 'export async function exportMenu' apps/desktop/src/lib/editor/exportMenu.ts
rg -q 'buttonLayout' apps/desktop/src/lib/editor/exportMenu.ts
rg -q 'export async function compositeProjectMenus' apps/desktop/src/lib/production.ts
rg -q 'compositeProjectMenus' apps/desktop/src/lib/components/ProductionModal.svelte
rg -q 'menu_navigation' apps/desktop/src-tauri/src/commands/menu.rs
rg -q 'menuNavigation' apps/desktop/src/lib/components/Simulator.svelte
rg -q 'studioMode === "vmMap"' apps/desktop/src/App.svelte
rg -q 'studioMode === "tracks"' apps/desktop/src/App.svelte
rg -q 'studioMode === "motion"' apps/desktop/src/App.svelte
rg -q 'studioMode === "vbr"' apps/desktop/src/App.svelte
rg -q 'loadPlaybackSupport' apps/desktop/src/lib/components/TrimmerStudio.svelte
rg -q 'shapeMask' apps/desktop/src/lib/editor/exportMenu.ts
rg -q 'media_playback_support' apps/desktop/src-tauri/src/commands/media.rs
rg -q 'media_preview_url' apps/desktop/src-tauri/src/commands/media.rs
rg -q 'previewUrlFor' apps/desktop/src/lib/playback.ts
rg -q 'layer_break_sector' crates/dvdforge-plan/src/lib.rs
rg -q 'shift_srt' crates/dvdforge-menu/src/lib.rs
rg -q 'AuthoredSubtitleTrack' crates/dvdforge-dvd/src/authoring.rs
rg -q 'menu_save_background' apps/desktop/src-tauri/src/commands/menu.rs
rg -q 'Channel<JobEvent>' apps/desktop/src-tauri/src/commands/render.rs
rg -q 'Channel<BurnProgress>' apps/desktop/src-tauri/src/commands/burn.rs
rg -q 'render_start' apps/desktop/src-tauri/src/commands/render.rs
rg -q 'burn_start' apps/desktop/src-tauri/src/commands/burn.rs

if compgen -G "/dev/sr*" >/dev/null; then
  echo
  echo "== burn_list_devices con hardware =="
  cargo test -p dvdforge-desktop --test commands_smoke \
    burn_list_devices_sees_the_physical_drive -- --ignored
else
  echo
  echo "Sin /dev/sr*: se omite la prueba de hardware."
fi

echo
echo "== Render end-to-end desde el comando IPC render_start =="
cargo test -p dvdforge-desktop --test commands_smoke \
  render_start_forwards_job_events_and_produces_iso -- --ignored

echo
echo "Fase 3 verificada: editor Konva, export PNG, IPC, render y ISO."
