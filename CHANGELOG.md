# Changelog

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
