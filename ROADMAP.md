# DVD Forge Roadmap — v0.5.0 to v1.0.0

Status: post-v0.4.0 planning document  
Baseline: v0.4.0 (2026-10-30)  
Scope: native authoring, determinism, player compatibility, desktop UX and LTS packaging

This roadmap is intentionally implementation-oriented. Every milestone names the
crates/modules to touch, the new diagnostics or verifier check IDs it introduces,
and the gates that must pass before the milestone ships. Check IDs and public
diagnostic codes become stable API once released: do not rename them later.

## 1. Baseline and guiding principles

### 1.1 v0.4.0 architecture

```text
crates/
  dvdforge-domain/       project schema v1, validation, menus, navigation, geometry
  dvdforge-project/      JSON load/save/validate
  dvdforge-media/        FFprobe adapter
  dvdforge-menu/         SPU rasterization, Spumux/TextSub XML
  dvdforge-plan/         rate planning, DVD-9 ECC, capacity budgets
  dvdforge-dvd/          MPEG-2/AC-3 encoding, DVDAuthor XML, IFO patching
  dvdforge-avchd/        H.264/AC-3 encode, tsMuxeR metadata, native BDMV muxer
  dvdforge-hdmv/         IG compiler, native M2TS/MPLS/CLPI/MovieObject builders
  dvdforge-iso/          DVD ISO builder, BDMV UDF 2.50 builder
  dvdforge-burn/         growisofs/xorriso, FreeBSD camcontrol, IMAPI2, drutil
  dvdforge-jobs/         job events, stages, diagnostics
  dvdforge-toolchain/    portable discovery, doctor, encoder probes
  dvdforge-store/        SQLite recent projects, jobs, Smart Cache v2
  dvdforge-testkit/      fixtures and test helpers
  dvdforge-cli/          doctor, probe, plan, render, verify, devices, burn
  dvdforge-render/       end-to-end orchestration, cache, streaming I/O
  dvdforge-verify/       independent read-only UDF/BDMV/DVD verifier
vendor/libmkiso/         UDF/ISO writer + independent reader
apps/desktop/            Svelte 5 + Vite + Tailwind + Konva
apps/desktop/src-tauri/  Tauri v2 commands, Spectra bindings
```

Current verifier check IDs: `UDF-AVDP`, `UDF-VRS`, `UDF-PVD`,
`UDF-PARTITION`, `UDF-FSD`, `UDF-ROOT`, `UDF-TAG266`, `UDF-MIRROR`,
`UDF-EXTENT`, `UDF-DIGEST`, `BDMV-INDEX`, `BDMV-MOBJ`, `BDMV-MPLS`,
`BDMV-CLPI`, `BDMV-M2TS`, `BDMV-PID`, `BDMV-IG`, `BDMV-PGS`,
`DVD-VTS-IFO`, `DVD-IFO-BUP`, `DVD-PGC`, `DVD-LAYER-BREAK`,
`DVD-REGION`, `DVD-VOB`.

Current render/toolchain diagnostics relevant to future work include
`W_BITMAP_SUBTITLE_UNSUPPORTED`, `W_PGS_LAYOUT_MISMATCH`,
`W_PGS_DELAY_IGNORED`, `W_NATIVE_MUX_FALLBACK`, `W_HDMV_IG_ATTACH`,
`W_HDMV_IG_COMPILE`, `W_NAV_NO_NEIGHBOR`, `W_NAV_UNREACHABLE`,
`W_AUDIO_LAYOUT_MISMATCH`, `W_BUTTON_OUTSIDE_SAFE_AREA`,
`W_HW_ENCODER_FALLBACK` and the `W_CACHE_*` family.

### 1.2 Rules every milestone must keep

1. **Bounded memory.** No production path may whole-file read an ISO, M2TS,
   VOB or BD payload. New code uses 64 KiB–1 MiB buffers or stream-copy APIs;
   i686 remains a release target with the 256 MB RSS budget.
2. **Independent verifier.** `dvdforge-verify` never reuses writer code to
   validate. New checks are read-only, stable-ID, and fail/warn/block burning
   with the existing semantics.
3. **Schema v1 compatibility.** Additive project fields use `#[serde(default)]`
   and `specta::Type`. The v1.0.0 freeze only happens after a migration audit;
   new fields remain optional through all pre-1.0 releases.
4. **Diagnostics are product API.** Every new failure path gets a stable
   `E_*`/`W_*` code, Spanish message text, and (where it affects the editor) an
   entry in the i18n catalog. Never replace a code with a new meaning.
5. **Deterministic cache.** Any render-affecting field added to a project or
   encoder path must be added to `EpisodeRenderInputs`/`inputs_hash` and bump
   `ENCODER_REV` when effective arguments change.
6. **Native-first fallbacks.** tsMuxeR remains optional; new authoring features
   must work on the native Rust/FFmpeg path before they are considered shipped.
7. **No silent skipping.** Unsupported combinations produce a warning and a
   deterministic output shape, never a partially authored disc.

### 1.3 Release quality bar

Every release, including point releases, must pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --exclude dvdforge-desktop --target i686-unknown-linux-gnu
cargo test -p dvdforge-plan -p dvdforge-verify -p dvdforge-hdmv --target i686-unknown-linux-gnu
cd apps/desktop && npm run check && npm run build
cd vendor/libmkiso && cargo fmt -- --check && cargo test --all-features --locked
```

Plus: no new `fs::read` of a multi-GB asset, no unchecked `usize` extent math,
no verifier writes, no regression in `bd_info` acceptance for native BDMV, and
an updated `CHANGELOG.md`/`README`.

---

## 2. v0.5.0 — Native Subtitle Rendering Engine and Audio Mastering

### 2.1 Objective

Remove the two remaining external/format compromises: bitmap subtitle
authoring becomes a native, pure-Rust pipeline, and audio receives a mastering
stage before AC-3/MPEG-2/TS muxing.

### 2.2 Native subtitle engine

**New crate:** `crates/dvdforge-subtitle/`

Suggested modules:

```text
src/lib.rs                  public types, warnings, render facade
src/parse/srt.rs            SRT parser + timing normalization
src/parse/ass.rs            ASS/SSA styles, overrides, positioning
src/parse/vtt.rs            WebVTT cues, regions, voices
src/script.rs               common cue/line/run model
src/layout.rs               line breaking, safe areas, alignment, ruby-free text
src/raster.rs               ab_glyph shaping, outline/shadow, alpha coverage
src/pgs.rs                  HDMV PGS display sets (PCS/WDS/PDS/ODS/END)
src/spu.rs                  DVD 4-color SPU bitmaps (2-bit indexes + alpha)
src/fonts.rs                font collection, fallback chain, metrics cache
```

Dependencies: `ab_glyph` (already compatible with the GPL build), `image`
(workspace), `unicode-segmentation`, optional `fontdb` for system-font lookup.
No FFmpeg subtitle filter is used for authoring; FFmpeg remains the encoder.

Deliverables:

- Common subtitle model for `.srt`, `.ass`/`.ssa` and `.vtt`, including
  forced/default flags, language, delay, trim offset and style overrides.
- Pure-Rust rasterizer:
  - 1080p/720p PGS display sets with correct PTS/DTS (90 kHz), PCS/WDS/PDS/ODS
    ordering, palette/object RLE, window placement, composition states and
    epoch handling.
  - DVD SPU bitmaps with exactly four palette entries (transparent, background,
    text, outline), anti-aliased coverage mapped to the 2-bit selector plane,
    correct control-sequence order, packet lengths and durations.
- Style model: font family, size, weight, italic, outline width/color, shadow
  offset/color, safe-area margins, alignment, line spacing, max lines, and
  per-title style sets.
- Forced-track routing metadata and default/forced dispositions in PGS/SPU.
- Integration:
  - `dvdforge-domain/src/project.rs`: `SubtitleStyle`, `FontSpec`,
    `SafeArea` (all `serde(default)` on `SubtitleTrack`).
  - `dvdforge-menu/src/lib.rs`: render native SPU files without Spumux for the
    DVD path.
  - `dvdforge-avchd/src/lib.rs`/`native.rs`: mux generated PGS into the
    combined MKV/M2TS and register tracks in MPLS/CLPI (reuse v0.4.0 paths).
  - `dvdforge-render/src/lib.rs`: choose text-subtitle pipeline (native) or
    legacy Spumux path; cache hashes include style/font identity.
  - `dvdforge-plan/src/lib.rs`: estimate PGS/SPU overhead per track and add
    capacity warnings.

**New diagnostics:**

| Code | Meaning |
|---|---|
| `E_SUBTITLE_PARSE` | Input cannot be parsed or has no usable cues. |
| `E_SUBTITLE_TIMING` | Negative/non-monotonic cue timing after trim/delay. |
| `E_SUBTITLE_FONT` | Requested font/weight cannot be resolved and no fallback exists. |
| `E_SUBTITLE_RASTER` | Bitmap exceeds PGS/SPU limits or cannot be encoded. |
| `E_SUBTITLE_STYLE` | Style values out of range or unsupported for target. |
| `W_SUBTITLE_SAFE_AREA` | Text/glyph extends outside the title-safe area. |
| `W_SUBTITLE_OVERFLOW` | Cue exceeds max lines/objects and is split or clipped. |
| `W_SUBTITLE_GLYPH_FALLBACK` | A glyph came from a fallback font. |
| `W_SUBTITLE_FORCED_MISMATCH` | Forced subtitle has no matching language/period. |
| `W_SUBTITLE_DROP` | Cue removed because it falls outside the program timeline. |

**New verifier checks:**

| Check ID | Validation |
|---|---|
| `BDMV-PGS-DS` | PGS display-set segment ordering, palette/object references and PCS composition state. |
| `BDMV-PGS-TS` | PGS PTS/DTS monotonicity, epoch start and video-timeline alignment. |
| `DVD-SPU-SEQ` | SPU control-sequence ordering, packet lengths and duration fields. |
| `DVD-SPU-PALETTE` | Four-entry palette, alpha/contrast and selector-plane consistency. |

### 2.3 Audio mastering pipeline

**Primary module:** `crates/dvdforge-render/src/audio.rs`, with argument
builders in `dvdforge-dvd/src/lib.rs` and `dvdforge-avchd/src/lib.rs`.

Deliverables:

- Optional two-pass EBU R128 `loudnorm` (I, LRA, TP targets) with a
  deterministic second pass; cache key includes measured values and target.
- Channel layouts:
  - stereo → 5.1 AC-3 upmix (center/ambience synthesis, LFE policy, phase-safe);
  - 5.1 → stereo downmix (ITU coefficients, dialogue normalization);
  - 5.1 passthrough where target supports it.
- Blu-ray audio pass-through: AC-3, DTS (where muxer/player profile allows) and
  LPCM; explicit `E_AUDIO_CODEC_UNSUPPORTED` when a target cannot carry it.
- Per-track delay compensation applied before muxing, not by dropping samples;
  exact frame/sample accounting for AC-3 frame boundaries and MPEG-2 PES.
- `dvdforge-plan` budget accounts for AC-3/LPCM/DTS payload and container
  overhead.

**New diagnostics:**

| Code | Meaning |
|---|---|
| `E_AUDIO_CODEC_UNSUPPORTED` | Target cannot carry the requested pass-through codec. |
| `E_AUDIO_CHANNEL_LAYOUT` | Source/target layout combination is invalid. |
| `E_AUDIO_DELAY` | Delay cannot be represented within frame/sample limits. |
| `W_AUDIO_LOUDNESS` | Loudness measurement out of target or unavailable. |
| `W_AUDIO_CLIP` | True-peak/clipping risk after gain or upmix. |
| `W_AUDIO_UPMIX` | Stereo-to-5.1 synthesized content is used. |
| `W_AUDIO_DOWNMIX` | Multichannel-to-stereo downmix is used. |
| `W_AUDIO_PASSTHROUGH` | Pass-through selected; no re-encode validation possible. |

**New verifier checks:**

| Check ID | Validation |
|---|---|
| `BDMV-AUDIO-STREAM` | PMT/MPLS/CLPI audio coding types, PIDs, channel counts and language. |
| `DVD-AUDIO-STREAM` | VTS audio attributes, AC-3/LPCM coding mode, sample rate and channels. |
| `DVD-AUDIO-DELAY` | Audio/video start PTS alignment within DVD tolerance. |

### 2.4 v0.5.0 acceptance criteria

- Golden-file tests for SRT/ASS/VTT → PGS/SPU, including styles, forced flags,
  safe areas and 23.976/25/29.97 timelines.
- Decode generated PGS with FFmpeg/`sup` and libbluray; generated SPU parsed by
  an independent test parser and by `dvdforge-verify`.
- Native and tsMuxeR BDMV paths both pass `bd_info` and the verifier.
- Loudness: measured output within ±0.5 LU of target for fixture tones; AC-3
  frame boundary test; 5.1 up/downmix null tests.
- 32-bit: subtitle rasterizer peak buffer bounded; i686 check/tests stay green.
- `cargo test --workspace` and full v0.4.0 gate list pass.

### 2.5 v0.5.1 — Live subtitle style preview and forced routing

Targets: `apps/desktop/src/lib/components/studio/`, `stores/project.svelte.ts`,
`apps/desktop/src-tauri/src/commands/menu.rs`, `dvdforge-domain/src/project.rs`.

- Canvas overlay preview using the same `dvdforge-subtitle` layout data as the
  final render (no divergent CSS approximation).
- Inspector controls for font/outline/shadow/safe area, with debounced
  regeneration of a low-res preview only.
- Forced-subtitle routing: language + forced-only selection, per-title override,
  automatic default-track selection, and a generated `SetStream` command.
- New diagnostics: `W_FORCED_ROUTE_AMBIGUOUS`, `E_FORCED_ROUTE_CONFLICT`,
  `W_SUBTITLE_PREVIEW_DRIFT`.
- Verification: visual regression snapshots for preview vs. rendered frame;
  route table tests for every menu/title/episode path; no IPC type changes
  without regenerated bindings.

---

## 3. v0.6.0 — Menu Designer 2.0, Templates, Motion Thumbnails, D-Pad Router

### 3.1 Objective

Turn the menu editor from a manual button grid into a deterministic design
system that still produces valid DVD VM commands and HDMV IG streams.

### 3.2 Template library

Targets: `crates/dvdforge-domain/src/menu.rs`, new
`crates/dvdforge-menu/src/templates.rs`, `apps/desktop/src/lib/components/menu/`.

Deliverables:

- Built-in templates: Cinema Dark, Multi-Page TV Series Grid, Classic
  Criterion, Retro CRT, Minimalist.
- Template model independent from project JSON: palette, typography, button
  shapes, background treatment, menu audio policy, animation policy.
- Multi-page grid pagination: episode count → page count → button geometry →
  Prev/Next actions; page overflow is a warning, never a silent truncation.
- Template application is non-destructive: existing buttons/assets are
  preserved and remapped where possible.
- New diagnostics: `W_TEMPLATE_CAPACITY`, `W_MENU_PAGE_OVERFLOW`,
  `W_TEMPLATE_ASSET_MISSING`.
- New checks: `BDMV-IG-BUTTONS` (button count/geometry/overlap), `BDMV-IG-NAV`
  (neighbor graph validity), `DVD-SPU-BUTTONS` (SPU button rectangles and
  highlight palette).

### 3.3 Motion thumbnails and transitions

Targets: `dvdforge-render/src/lib.rs` (FFmpeg filtergraph builders),
`dvdforge-menu/src/lib.rs`, `dvdforge-domain/src/menu.rs`.

- Clip loops inside buttons: source in/out, loop duration, crop/scale, overlays,
  and background compositing, all expressed as a deterministic filtergraph.
- Intro-to-menu transition clips with duration/easing metadata; authoring uses
  a pre-rendered clip, not a player-side effect.
- Cache keys include source fingerprint, clip range, loop duration, overlay
  composition and transition parameters.
- New diagnostics: `W_MOTION_THUMBNAIL_FALLBACK`, `W_INTRO_TRANSITION_TRUNCATED`,
  `W_MOTION_LOOP_SHORTENED`.
- New check: `BDMV-MOTION-THUMB` (clip/playlist references and duration).

### 3.4 Deterministic D-Pad auto-router

Targets: `crates/dvdforge-domain/src/navigation.rs`, menu serializers in
`dvdforge-menu`/`dvdforge-hdmv`.

- Solve a conflict-free directional graph over button rectangles using a
  deterministic ordering (row/column bands, then nearest-center with stable
  tie-breakers). Manual overrides always win.
- Guarantee every button is reachable from the default focus, and that
  Up/Down/Left/Right are symmetric where geometry allows.
- Emit `E_NAV_ROUTE_CONFLICT` for unsolvable overrides, `W_NAV_ROUTE_APPROX`
  when a neighbor is only approximate.
- Interactive remote simulator in Svelte 5 uses the same route table as
  authoring; keyboard arrows/Enter must drive it.
- Verification: property tests on randomized rectangles (no panics, no missing
  targets, determinism across runs); snapshot route tables for every template.

### 3.5 v0.6.0 acceptance criteria

- All templates render DVD SPU and Blu-ray IG without verifier failures.
- Pagination golden tests for 1/6/12/24 episodes.
- Motion thumbnail and transition smoke tests on FFmpeg; cache invalidation
  tests.
- Route determinism hash stable across platforms and repeated runs.
- UI: remote preview + keyboard navigation tested with `npm run check` and
  component smoke tests.

### 3.6 v0.6.1 — Chapter sub-menus and audio crossfade

Targets: `dvdforge-domain/src/menu.rs`, `dvdforge-render/src/lib.rs`, menu
serializers, Studio canvas.

- Multi-page chapter selection sub-menus generated per title/episode.
- Audio loop crossfading between menu tracks and intro/outro transitions with
  exact frame durations; no clicks at loop boundaries.
- New diagnostics: `W_CHAPTER_MENU_OVERFLOW`, `W_AUDIO_CROSSFADE_CLIPPED`,
  `W_AUDIO_LOOP_GAP`.
- Verification: chapter action matrix, loop-boundary sample analysis, player
  simulator route coverage.

---

## 4. v0.7.0 — Smart Stream Copy, Play-All Logic, ISO Inspector

### 4.1 Smart Stream Copy

Targets: `crates/dvdforge-plan/src/stream_copy.rs` (new),
`crates/dvdforge-render/src/lib.rs`, `crates/dvdforge-media/src/lib.rs`,
`crates/dvdforge-dvd/src/lib.rs`, `crates/dvdforge-avchd/src/lib.rs`.

Deliverables:

- Compatibility detector for:
  - DVD-Video: MPEG-2 program stream, 720×480/576, permitted frame rates,
    closed GOP ≤18 frames, AC-3/LPCM, DVD bitrate envelope;
  - AVCHD/Blu-ray: H.264 High profile, permitted resolution/frame rate, closed
    GOP, AC-3/LPCM/DTS policy, VBV/bitrate envelope.
- Trim/keyframe analysis: stream copy only at keyframe-aligned trim points;
  otherwise fall back to re-encode or offer a split/align repair.
- Render bypass with FFmpeg `-c:v copy` plus remux-only flags; cache key gains
  `encoding_mode` and the exact copy decision.
- New diagnostics: `E_STREAM_COPY_INCOMPATIBLE`, `W_STREAM_COPY_FALLBACK`,
  `W_STREAM_COPY_KEYFRAME`, `W_STREAM_COPY_TRIM_ALIGN`,
  `W_STREAM_COPY_CONTAINER`.
- Verification: fixture matrix of compliant/noncompliant streams; output
  checksums for remux-only paths; A/V sync tests; no re-encode assertion via
  FFmpeg logs.

### 4.2 True Play-All vs. single-episode return

Targets: `dvdforge-dvd/src/authoring.rs`, `dvdforge-hdmv/src/opcodes.rs`,
`dvdforge-hdmv/src/native.rs`, `dvdforge-render/src/lib.rs`,
menu action compiler.

Deliverables:

- DVD: VMGM/VTS GPRM state machine distinguishing Play All from a single-title
  jump; chapter cells return to the correct menu, Play All chains episodes.
- Blu-ray: MovieObject GPR commands and PlayPL/PlayPL_PM sequences with a
  per-title mode register; menu buttons set GPR0/GPR1 explicitly.
- Simulator consumes the same transition table used by authoring.
- New diagnostics: `E_PLAY_ALL_LOGIC`, `W_PLAY_ALL_FALLBACK`,
  `W_GPRM_COLLISION`.
- New checks: `DVD-GPRM` (GPRM ranges/commands), `DVD-PLAYALL` (cell/command
  chain), `BDMV-GPR` (GPR command ranges), `BDMV-PLAYALL` (MovieObject chain).

### 4.3 Standalone ISO and folder inspector

Targets: `dvdforge-verify/src/inspect.rs` (new), `report.rs`,
`apps/desktop/src-tauri/src/commands/inspect.rs` (new),
`apps/desktop/src/lib/components/inspector/`, generated bindings.

- Read-only inspector API for ISO images and `VIDEO_TS`/`BDMV` folders.
- Views: UDF partition/extent map, Tag 266 metadata entries, layer break,
  stream inventory/bitrates, playlist/clip graph, IFO/PGC tree, warnings.
- Open external discs into a draft project (import unencrypted VIDEO_TS/BDMV;
  media paths normalized; no copy unless requested).
- All access is bounded and read-only; the same file handle/lifetime rules as
  `verify_iso`.
- New report sections/check IDs: `UDF-INSPECT`, `BDMV-INSPECT`,
  `DVD-INSPECT`; new diagnostic `E_INSPECT_UNSUPPORTED`,
  `W_INSPECT_ENCRYPTED`, `W_INSPECT_PARTIAL`.
- Verification: inspector golden snapshots for DVD-9/BD-50 fixtures; encrypted
  disc is refused with `W_INSPECT_ENCRYPTED`; no writes are observed during
  inspection; Tauri commands registered and bindings regenerated.

### 4.4 v0.7.0 acceptance criteria

- Stream-copy decision tests pass on real fixture probes; remux path keeps A/V
  sync and chapter marks.
- Play-All/single-return matrices pass in the simulator and on authored
  DVD/BDMV structures; verifier checks pass.
- Inspector opens external ISO/folder without modifying it; bindings and UI
  smoke tests pass.
- Full release gate list and i686 checks pass.

---

## 5. v0.8.0 — Crash-Safe Checkpointing, Batch Queue, Toolchain Provisioner

### 5.1 Render checkpoint and resume

Targets: `crates/dvdforge-render/src/checkpoint.rs` (new),
`crates/dvdforge-store/src/lib.rs` (DB migration),
`crates/dvdforge-jobs/src/lib.rs` (stage IDs), Smart Cache v2.

Deliverables:

- Per-job checkpoint file/table: job id, project hash, input hash, completed
  stages (probe, encode episode N, menu N, concat, M2TS mux, UDF, verify),
  artifact paths/hashes, toolchain hash, encoder rev.
- Resume only when inputs and toolchain are unchanged; otherwise mark stale and
  rebuild affected stages.
- Atomic checkpoint writes (temp + rename); no checkpoint is trusted after a
  crash until its stage artifact is revalidated.
- New diagnostics: `W_RENDER_CHECKPOINT_MISSING`, `W_RENDER_RESUME_STALE`,
  `E_RENDER_CHECKPOINT_CORRUPT`, `W_RENDER_RESUME_PARTIAL`.
- Verification: kill -9 during encode/mux/ISO and resume; corrupted checkpoint
  fails closed; no duplicate artifacts; Smart Cache hit/miss matrix.

### 5.2 Batch render queue

Targets: `crates/dvdforge-cli/src/batch.rs` (new),
`apps/desktop/src/lib/stores/queue.svelte.ts`, `commands/render.rs`,
`dvdforge-jobs`.

- Queue manifest (JSON): multiple projects, output dirs, options, concurrency 1
  (optical authoring is memory-bound), fail-fast vs. continue-on-error.
- CLI `dvdforge-cli batch queue.json [--json]` with per-item status, resume and
  aggregate exit code.
- Desktop queue panel with reorder, cancel, retry and persistent history.
- New diagnostics: `W_BATCH_PARTIAL`, `E_BATCH_ITEM`, `W_BATCH_CANCELLED`.
- Verification: queue runs two projects sequentially; one failure honors
  policy; cancellation leaves no lock; resume after restart.

### 5.3 Portable toolchain provisioner

Targets: `crates/dvdforge-toolchain/src/provision.rs` (new),
`toolchains/manifest.json`, `ToolchainDoctor.svelte`, release assets.

- One-click download of verified archives into `<exe_dir>/toolchain/` with
  SHA-256 + size + optional signature verification.
- Platform/arch manifest selection, atomic extract, safe path traversal checks,
  no execution of downloaded scripts, explicit license display.
- Cache/integrity state in SQLite or a sidecar manifest; offline reuse.
- New diagnostics: `E_TOOLCHAIN_CHECKSUM`, `W_TOOLCHAIN_DOWNLOAD_FALLBACK`,
  `E_TOOLCHAIN_ARCHIVE`, `W_TOOLCHAIN_UNVERIFIED`.
- Verification: checksum mismatch deletes staging; archive traversal rejected;
  offline second run uses cache; doctor shows provenance.

### 5.4 Self-contained project bundles

Targets: `crates/dvdforge-project/src/bundle.rs` (new), CLI import/export,
desktop open/save.

- `.dvdforge-bundle` format: project JSON + referenced assets + font/style
  sidecars + manifest (schema version, hashes, toolchain notes). Use a
  deterministic tar + zstd container; no absolute paths.
- Import validates hashes before writing; missing assets produce warnings.
- New diagnostics: `E_BUNDLE_CORRUPT`, `W_BUNDLE_EXTERNAL_ASSET`,
  `E_BUNDLE_SCHEMA`.
- Verification: round-trip bundle on Linux/Windows; tamper detection; assets
  outside project are either embedded or explicitly warned.

### 5.5 v0.8.1 — Player compatibility presets

Targets: `crates/dvdforge-domain/src/presets.rs` (new),
`dvdforge-render` encoder/mux flags, verifier profile checks.

- Presets: PS2 DVD-Video strict, PS3/PS4/PS5 BDMV/AVCHD, Sony standalone,
  Panasonic standalone.
- Presets pin GOP/VBV/profile, menu still behavior, audio codec choices, file
  naming and padding.
- New diagnostics: `W_PLAYER_PRESET_STRICT`, `E_PLAYER_PRESET_CONFLICT`.
- New checks: `DVD-COMPAT-PROFILE`, `BDMV-COMPAT-PROFILE`.
- Verification: preset matrix build + verifier; smoke playback notes per preset.

---

## 6. v0.9.0 — Frontend Splitting, Full Localization, Fuzzing, RC Hardening

### 6.1 Frontend code-splitting and accessibility

Targets: `apps/desktop/vite.config.ts`, `apps/desktop/src/routes/` (or feature
barrels), `lib/components/`, generated chunks.

- Route/feature-level dynamic imports; keep initial route under the existing
  warning threshold; measure raw/gzip sizes in CI.
- Keyboard-only operation for canvas, inspector, dialogs and simulator; focus
  traps; ARIA roles; reduced-motion mode.
- Verification: build chunk report; `svelte-check`; keyboard smoke tests;
  axe/Lighthouse accessibility checks in CI (non-blocking during RC, blocking
  for v1.0.0).

### 6.2 Localization

Targets: `apps/desktop/src/lib/i18n/messages.ts`, locale catalogs.

- Complete English/Spanish/Portuguese catalogs for every UI string and
  diagnostic; fallback to English only for unknown keys.
- New locale tests for missing keys, pluralization and RTL-safe layout.
- New diagnostic: `W_LOCALE_FALLBACK` (development/doctor only).
- Verification: `npm run check` with a locale-key completeness test; manual
  review of all three languages.

### 6.3 Fuzzing and property tests

Targets: `fuzz/Cargo.toml` (new), `fuzz/fuzz_targets/*.rs`,
`vendor/libmkiso/tests`, `dvdforge-verify`, `dvdforge-hdmv`.

- cargo-fuzz targets: UDF/ISO descriptor parser, BDMV navigation parser,
  M2TS/PMT parser, DVD IFO/PGC parser, subtitle PGS/SPU parser.
- Property tests: no panics/overflow on arbitrary bytes; bounded allocations;
  every error is typed; parser never writes.
- Nightly fuzz job with corpus minimization; 32-bit soak test (long-running
  virtual streams, RSS ceiling) in CI.
- Verification: 24h fuzz smoke with zero crashes; minimized corpus checked in;
  `cargo test` includes deterministic property seeds.

---

## 7. v1.0.0 — General Availability

### 7.1 Schema freeze and deterministic builds

Targets: `crates/dvdforge-domain/src/project.rs`, `dvdforge-project`,
`vendor/libmkiso`, `dvdforge-iso`, release scripts.

- Freeze Project Schema v1: add `docs/schema-v1.md`, deprecation policy,
  migration stubs for any future v2, golden old-project fixtures.
- Deterministic ISO output: fixed `SOURCE_DATE_EPOCH`, stable timestamps in UDF
  and ISO9660, sorted directory iteration, stable toolchain hash, no random
  temp names in image metadata.
- New check: `REPRO-ISO` (two builds byte-identical); CI verifies twice.
- Verification: double-build SHA-256 equality on DVD-5/DVD-9/BD-25/BD-50
  fixtures, independent of locale/time zone.

### 7.2 Packaging and distribution

Targets: new `packaging/` tree and release workflow.

- Arch/AUR `packaging/arch/PKGBUILD`, Flatpak manifest, Winget manifest,
  Homebrew formula, plus existing MSI/NSIS/AppImage/deb/dmg/CLI archives.
- Package metadata, licenses, desktop entries, icons, man/pages, portable
  toolchain instructions.
- Verification: install/uninstall smoke tests per package manager; release
  artifact checksum manifest; no undeclared runtime dependencies.

### 7.3 Player certification suite

Targets: `docs/certification/`, `scripts/player-matrix/`, verifier profiles.

- Certification matrix for PS2/PS3/PS4/PS5, Sony/Panasonic/Denon standalone
  players, VLC/MPC/libbluray software.
- Automated structural profile runs and manual playback checklists.
- Known-issue register with workarounds; `W_PLAYER_CERT_MISSING` for uncertified
  presets used in a release build.
- Verification: every release profile has at least one verified build per
  target family; failures are documented, not hidden.

### 7.4 v1.0.0 acceptance criteria

- All `v0.4.0`–`v0.9.0` gates, plus schema-freeze fixtures and reproducible
  double builds.
- Packaging install tests on Linux, Windows, macOS; FreeBSD CLI source build.
- Certification checklist signed off for the release matrix.
- `README`/`README.es`/`AGENTS`/`CHANGELOG` updated; version `1.0.0`.

---

## 8. Cross-milestone dependency order

| Workstream | Depends on |
|---|---|
| Native subtitle engine | v0.4 bounded I/O, `dvdforge-menu` SPU model, `dvdforge-avchd` PGS mux |
| Audio mastering | v0.4 native mux, render cache keys, `dvdforge-plan` budget |
| Menu templates/D-pad | v0.5 subtitle style preview only for typography reuse |
| Motion thumbnails | v0.6 templates + render filtergraph + cache v2 |
| Smart stream copy | v0.7 plan/media probe; Play-All independent |
| ISO inspector | `dvdforge-verify` read-only API; Tauri command plumbing |
| Checkpointing | Smart Cache v2 and store migrations |
| Batch/queue | checkpointing and job model |
| Toolchain provisioner | v0.4 portable discovery + manifest |
| Fuzzing | parser freeze candidates: libmkiso, verify, hdmv |
| Deterministic builds | v1.0 schema freeze + timestamp/toolchain hygiene |

---

## 9. Out of scope through v1.0.0

- BD-J authoring, AACS/BD+ decryption, CSS/DVD encryption.
- 4K UHD Blu-ray, Dolby Vision/HDR10+ authoring.
- Online/cloud rendering or a hosted service.
- Windows/macOS kernel or driver work; burning remains tool/OS API based.
- Lossy transcoding of protected content; inspector refuses encrypted discs.

---

## 10. Traceability: check and diagnostic ID policy

- New IDs in this document are reserved names; if an implementation chooses a
  different ID, update this roadmap before release and keep the old ID absent
  (never reuse it).
- Verifier IDs are grouped by container: `UDF-*`, `BDMV-*`, `DVD-*`, future
  `REPRO-*`. A check may warn or fail, but its meaning does not change.
- Diagnostic codes are grouped by domain: `E_AUDIO_*`, `W_SUBTITLE_*`,
  `W_MENU_*`, `W_NAV_*`, `W_STREAM_COPY_*`, `W_RENDER_*`, `W_BATCH_*`,
  `E_TOOLCHAIN_*`, `W_PLAYER_*`.
- Each release note lists newly added IDs and any intentionally deprecated
  aliases.
