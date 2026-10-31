# Changelog

## 0.6.0 — 2026-10-31

### Menu Designer 2.0

- Added five curated templates: Cinema Dark, Multi-Page TV Series Grid,
  Classic Criterion, Retro CRT and Minimalist.
- Templates are defined in Rust (`dvdforge-menu::templates`) and exposed to the
  desktop through `menu_templates` / `menu_apply_template`.
- Non-destructive pagination: episodes are partitioned by template capacity,
  page 0 stays the VMGM root, later pages are VTSM EpisodeSelect menus with
  Prev/Next GotoMenu actions, and custom buttons/items/assets are preserved on
  Extras pages.
- Added `W_TEMPLATE_CAPACITY`, `W_MENU_PAGE_OVERFLOW` and
  `W_TEMPLATE_ASSET_MISSING` validation diagnostics.
- Added a template gallery and page-aware menu studio state.

### Motion and transitions

- Added `Menu.transition`: a pre-rendered intro clip with duration/easing
  metadata rendered as a pre-roll before the menu loop.
- DVD and HDMV menu clips concatenate the transition, fade at the boundary,
  delay overlays and audio, and offset the SPU/IG start to the transition end.
- Menu cache identity now includes motion/transition sources, durations and
  easing; `ENCODER_REV` is 6.

### Navigation

- Added deterministic D-Pad symmetry normalization and reachability fallback,
  with `E_NAV_ROUTE_CONFLICT` / `W_NAV_ROUTE_APPROX`.
- Added a canonical `routeHash` to `NavigationGraph`; the simulator displays
  the same route fingerprint used by DVD and HDMV authoring.

### Verification

- Added `BDMV-IG-BUTTONS`, `BDMV-IG-NAV` and `BDMV-MOTION-THUMB`.
- Added `DVD-SPU-BUTTONS` for menu SPU/button structure.
- Extended the smoke suite with DVD/BD multi-page template renders and DVD/BD
  motion-transition renders; all ignored tests pass locally with `bd_info` and
  `dvdforge-verify`.

## 0.5.0 — 2026-10-30

### Native subtitles

- New workspace crate `dvdforge-subtitle` parses SRT, ASS/SSA and WebVTT in
  Rust and rasterizes styled text with `ab_glyph`/`image`.
- Blu-ray/AVCHD text tracks are now rendered to HDMV PGS `.sup` display sets
  and muxed into the combined MKV instead of being ignored.
- DVD subtitles are rasterized to native four-color PNGs and authored through
  Spumux `<spu>` XML, replacing the hardcoded TextSub path; ASS/SSA colors,
  sizes and alignments are preserved by the parser.
- Added font resolution through `fontdb` with explicit project-relative font
  paths, deterministic font fingerprints, safe-area layout, outline/shadow,
  forced cues and bounded 1080p rasterization.
- Persisted `SubtitleTrack.style` (font, size, outline/shadow, colors,
  alignment, safe area, max lines) with schema defaults and regenerated IPC
  bindings; DVD and AVCHD render each track with its own style.

### Audio mastering

- Added per-track `AudioMastering`: optional EBU R128 `loudnorm`, source/stereo/
  5.1 channel modes, explicit up/downmix matrices, codec preference and
  pass-through metadata.
- DVD and AVCHD apply loudness/channel pre-filters, gain and delay in one
  `-af`/`-filter:a:N` chain; the AVCHD encoder now honors per-track bitrate and
  channel layout.
- AVCHD supports multiple audio tracks end to end: FFmpeg per-stream mapping,
  tsMuxeR meta lines, native PMT/MPLS/CLPI descriptors and language registration.
- DTS/LPCM/AC-3 pass-through is selected when `AudioMastering` has no active
  filters and the probed source codec matches; otherwise AC-3 re-encoding is
  used.
- Mastering fields and measured target values are included in the render cache
  key; `ENCODER_REV` was bumped.

### Verification and tests

- Added parser, layout, PGS, four-color Spumux and FFmpeg PGS smoke coverage.
- Validated generated PGS with FFmpeg and Spumux output with DVDAuthor 0.7.2.
- Added verifier checks `BDMV-PGS-DS`, `BDMV-PGS-TS`,
  `BDMV-AUDIO-STREAM`, `DVD-SPU-SEQ` and `DVD-SPU-PALETTE`.
- Added native multi-audio BDMV and tsMuxeR meta/argument tests.
- Preserved v0.4.1 audio warning fixes and all existing i686/native BDMV tests.

## 0.4.1 — 2026-10-30

### Bug fixes

- Episode `gain_db` is now applied to DVD and AVCHD/Blu-ray audio instead of
  only being included in the cache hash.
- DVD and AVCHD audio delay filters are composed into one `-af` chain; negative
  delays pad the track back to the known program duration instead of truncating
  the tail.
- AVCHD/Blu-ray now honors the per-track AC-3 bitrate and channel layout
  (`AudioFormat::Ac3Stereo`/`Ac3Surround51`) instead of always using the
  project-wide channel count.
- External ASS/SSA/VTT subtitle tracks are converted to SRT through FFmpeg before
  Spumux authoring, with an explicit `W_SUBTITLE_STYLE_LOST` warning, instead of
  being fed to Spumux as if they were SubRip.
- Unsupported external subtitle extensions now fail with a clear message instead
  of producing a corrupt subtitle stream.
- `W_AVCHD_SUBTITLES_IGNORED` no longer fires for PGS tracks that are
  stream-copied; it is limited to text/non-PGS bitmap tracks.
- `W_SUBTITLE_STREAM_SOURCE` only applies to text tracks that need FFmpeg
  extraction, not to embedded PGS.
- Added regression tests for gain, padded negative delay, AVCHD filters and PGS
  warning behavior.

## 0.4.0 — 2026-10-30

### Performance and 32-bit memory

- All large media/ISO/M2TS paths use bounded 64 KiB–1 MiB streaming buffers:
  source fingerprints, subtitle/label hashes, UDF payloads, menu M2TS injection
  and burn verification no longer allocate whole files.
- `dvdforge-hdmv` adds `inject_ig_into_menu_m2ts_file`/`_stream`, processing
  192-byte M2TS packets through a 192 KiB buffer and checked `u32` packet counts.
- `vendor/libmkiso` shares node paths with `Arc<str>`, pre-reserves object
  storage, uses 1 MiB payload/digest buffers and adds a file-backed
  `FileSource`. The >4 GiB sparse test no longer reads the image into memory,
  and a 50 GiB BD-50 layout test was added.
- 32-bit Windows builds carry `/LARGEADDRESSAWARE`; i686 core tests and the
  vendored libmkiso tests run in CI.

### Toolchain and native authoring

- `DVDFORGE_TOOLCHAIN_DIR` and portable `toolchain/`/`bin/` directories beside
  the executable are searched before resources, system prefixes and PATH.
- tsMuxeR is optional. A native Rust + FFmpeg BDMV muxer builds the movie
  M2TS, `index.bdmv`, `MovieObject.bdmv`, MPLS, CLPI, chapter marks and HDMV IG
  menu clips when tsMuxeR is unavailable or the host is 32-bit.
- `MuxBackend` and `DVDFORGE_MUXER` select auto/native/tsMuxeR; 64-bit hosts
  keep the tsMuxeR default.

### Blu-ray PGS pass-through

- Active `hdmv_pgs_subtitle` tracks are stream-copied into AVCHD/Blu-ray and
  registered in MPLS STN and CLPI program/sequence info.
- DVD-Video keeps `W_BITMAP_SUBTITLE_UNSUPPORTED`; non-PGS bitmap codecs still
  warn. PGS delay emits `W_PGS_DELAY_IGNORED` and mixed episode layouts emit
  `W_PGS_LAYOUT_MISMATCH`.
- Cache inputs include PGS metadata; `ENCODER_REV` is now 3.

### FreeBSD

- New `dvdforge-burn` FreeBSD backend: `camcontrol` enumeration/eject,
  `dvd+rw-mediainfo` probing, `growisofs` burning with `xorriso` fallback and
  bounded read-back verification.
- FreeBSD tool discovery and system prefixes; CI compiles the headless
  workspace in a native FreeBSD VM and cross-checks the pure crates.

### Release

- Release profile uses thin LTO, one codegen unit and stripped symbols.
- CI/release matrix covers Windows x86_64/i686 (x86 WebView2), Linux
  x86_64/i686 and macOS arm64/x86_64. Publishes
  `dvdforge-cli-i686-pc-windows-msvc.zip` and
  `dvdforge-cli-i686-unknown-linux.tar.gz`.
- Workspace, desktop package and Tauri bundle versions bumped to 0.4.0.

## 0.3.0 — 2026-10-30

### Safety

- New `dvdforge-verify` read-only structural verifier for UDF, BDMV and
  DVD-Video images.
- `dvdforge-cli verify <iso> [--deep] [--json] [--strict]`.
- Render and burn now run structural verification automatically; a failed
  verification blocks burning.
- DVD-9 layer break is applied to the VTS PGC cell (SPLIP flag) and aligned to
  16-sector ECC blocks.
- Project save validates the project before writing.

### Performance

- Smart render cache v2 with `ArtifactKind`, full `fp2:` SHA-256 source
  fingerprints, canonical `inputs_hash`, encoder revision and toolchain hash.
- Menu videos and intro clips are cached in addition to episode artifacts.
- SQLite migration to `artifact_cache_v2`, cache stats and LRU pruning.
- MPEG-2 two-pass tuning: Lanczos scaling, RD/Trellis, dynamic DC precision,
  closed GOPs and exact PAL/NTSC GOP lengths.
- AVCHD H.264 hardware encoding (NVENC, QSV, VAAPI, AMF, VideoToolbox) with
  one-frame driver smoke tests and automatic CPU/libx264 fallback.

### Media import and UX

- FFprobe chapters, audio titles/default/commentary and subtitle stream
  metadata (language, title, default, forced, bitmap) are imported from MKV/MP4.
- Chapters can be imported from the source file inside the active trim range.
- Bitmap subtitles (PGS/VobSub/DVB) are detected and rejected with a diagnostic
  instead of breaking the Spumux text pipeline.
- IPC errors for recent projects, job history, menu previews and project save
  are surfaced instead of being silently discarded.

### Tests and release

- Added >4 GiB sparse UDF 2.50 and Tag 266 EFE tests.
- Added HDMV 12-byte opcode round-trips, IG TS reassembly and multi-width PGS
  slot tests.
- Added DVD IFO/BUP and PGC verifier tests, layer-break tests, cache-key
  invalidation tests and mid-file fingerprint tests.
- Version bumped to 0.3.0 across the workspace, desktop package and Tauri
  bundle.
