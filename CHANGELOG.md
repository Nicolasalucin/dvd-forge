# Changelog

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
