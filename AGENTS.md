# AGENTS.md — Kinodisc engineering guide

This file is the high-signal orientation for any AI coding agent working in this
repository. Read it before changing code. The project ships optical disc images;
several invariants are hardware- or format-critical and are not obvious from a
single crate.

## What Kinodisc is

Kinodisc is a Rust + Tauri v2 + Svelte 5 optical authoring studio for:

- **DVD-Video** (DVD-5/DVD-9, MPEG-2, AC-3, DVDAuthor/Spumux, DVD-9 layer break).
- **AVCHD** (H.264 on DVD media, HDMV Interactive Graphics menus).
- **Blu-ray** (BD-25/BD-50, UDF 2.50 metadata partition, HDMV menus).

The desktop app and CLI share the same render pipeline in `kinodisc-render`.

> Roadmap: [ROADMAP.md](ROADMAP.md) is the authoritative v0.6.0–v1.0.0 plan.
> New check IDs and diagnostic codes proposed there are reserved before
> implementation; keep this file's invariants and the roadmap in sync.

## Repository map

```text
crates/
  kinodisc-domain/       Pure models, validation, geometry, menus, diagnostics
  kinodisc-project/      Project JSON load/save/validate (schema v1)
  kinodisc-media/        FFprobe adapter: streams, tags, chapters, dispositions
  kinodisc-menu/         SPU/menu layer rasterization, templates, Spumux/TextSub XML
  kinodisc-subtitle/     Native SRT/ASS/VTT parsing, ab_glyph rasterizer, PGS/SPU
  kinodisc-plan/         Bitrate planning, DVD-9 ECC layer-break sector math
  kinodisc-dvd/          MPEG-2/AC-3 encoding, DVDAuthor XML, IFO patching
  kinodisc-avchd/        H.264/AC-3 encoding, tsMuxeR metadata, native BDMV muxer
  kinodisc-hdmv/         HDMV IG segments, opcodes, native M2TS/MPLS/CLPI/MovieObject
  kinodisc-iso/          DVD-Video ISO builder and BDMV UDF 2.50 builder
  kinodisc-burn/         Platform burners: growisofs/xorriso, camcontrol, IMAPI2, drutil
  kinodisc-jobs/         Job events, stages, diagnostics-facing types
  kinodisc-toolchain/    Binary discovery, tool doctor, hardware encoder probes
  kinodisc-store/        SQLite recent projects, job history, artifact cache v2
  kinodisc-testkit/      Shared test helpers
  kinodisc-cli/          CLI: doctor, probe, plan, render, verify, devices, burn
  kinodisc-render/       End-to-end DVD/AVCHD render orchestration and cache
  kinodisc-verify/       Independent read-only UDF/BDMV/DVD structural verifier

vendor/libmkiso/         Vendored UDF/ISO writer+reader (upstream 0.1.5 + patches)
vendor/libmkiso/KINODISC-PATCH.md
                         Local patch rationale; read before touching vendor code

apps/desktop/            Svelte 5 + Vite + Tailwind frontend
apps/desktop/src-tauri/  Tauri v2 commands, Specta bindings, platform setup
```

Generated IPC bindings live at:

```text
apps/desktop/src/lib/api/bindings.ts
```

Do not edit that file manually. Regenerate it with `npm run bindings` (or
`cargo run -p kinodisc-desktop --bin export-bindings`).

## Critical invariants — do not break

### 1. `vendor/libmkiso`: 64-bit sectors, Tag 266, UDF layout

- All ISO/UDF sector and extent arithmetic that can exceed 4 GiB must use
  `u64`/`u128`; never `usize` or `u32`. The CLI and authoring stack are compiled
  for **i686** and must handle BD-25/BD-50 and >4 GiB M2TS files.
- UDF 2.50 metadata files must use **Extended File Entries (Tag 266)** with long
  allocation descriptors. The local patch is documented in
  `vendor/libmkiso/KINODISC-PATCH.md`; the rest is upstream 0.1.5.
- UDF images must keep the trailing AVDPs within the final 32-sector/64 KiB
  boundary, and metadata mirrors must remain readable.
- Do not replace the reader with the writer for validation. The verifier needs
  an independent parse.
- Run `vendor/libmkiso` tests from inside `vendor/libmkiso`; it is intentionally
  excluded from the workspace.

### 2. `kinodisc-render`: MPEG-2 flags and cache determinism

The DVD-Video software encode path is tuned deliberately:

- Scaling uses `flags=lanczos`.
- RD/Trellis: pass 2 uses `-mbd rd -cmp rd -subcmp rd -trellis 2`. Pass 1 is
  a light statistics pass (`-mbd rd`, no `-cmp/-subcmp`, `-trellis 0`): with
  RD comparison, `b_strategy 2` trial-encoded every frame several times (5×
  slower, same final PSNR). Quality options kept after timing them:
  `-intra_vlc 1 -last_pred 2 -mpv_flags +mv0`; `qmin 1`, `dia_size` and
  `bidir_refine` were rejected (much slower, no visible gain).
- Interlaced output takes its field order from `setfield=tff/bff` in the
  filter chain; never `-top` (removed in FFmpeg 9).
- DC precision uses `-intra_dc_precision 0/1/2` (8/9/10 bits), not deprecated
  `-dc`.
- Closed GOP is mandatory: `-flags +cgop` and strict `-g 15` (PAL) / `-g 18`
  (NTSC). FFmpeg rejects closed GOP with scene-cut detection, so the code uses
  `-sc_threshold 1000000000`. Do not "restore" `-30000` without revalidating
  FFmpeg.
- Pass 1 writes to `NUL` on Windows and `/dev/null` on Unix; each `EncodeRequest`
  carries its own job-scoped `passlog` path.
- `ENCODER_REV` must be bumped when effective encoder arguments change, and
  `EpisodeRenderInputs`/`inputs_hash` must include every render-affecting field.

Smart Render Cache v2:

- `ArtifactKey` fields: `kind`, `source_fingerprint`, `inputs_hash`,
  `target_profile`, `target_bitrate`, `audio_params_hash`, `encoder_rev`,
  `toolchain_hash`.
- `source_fingerprint` is `fp2:` + full streaming SHA-256. Do not reintroduce
  size/mtime/first-last-8-MiB fingerprints.
- `inputs_hash` is canonical serialized render inputs. Changing trim, fps,
  scale mode, codec args, audio, subtitles, menu composition or encoder choice
  must change the hash.
- Cache failures are non-fatal warnings; they must never fail a render.
- DVD/AVCHD artifacts are published through `cached_artifact_path`; menu cache
  hits must be materialized into the output tree before Spumux.

### 3. `kinodisc-dvd`: DVD-9 layer break and IFO consistency

- The planner computes an ECC-aligned layer-break sector (16 DVD sectors).
- Authoring must call `apply_layer_break` (or equivalent) and set the **SPLIP**
  bit on the containing VTS PGC cell.
- `VTS_01_0.IFO` and `VTS_01_0.BUP` must be byte-identical after patching.
  `VIDEO_TS.IFO`/`VIDEO_TS.BUP` must also remain identical.
- A DVD-9 build without a valid SPLIP cell is structurally broken; the verifier
  must fail it.
- Do not claim in the UI that the layer break is applied unless the authoring
  path actually does it.

### 4. `kinodisc-verify`: independent and read-only

- The verifier is a read-only parser. It must not reuse writer code paths for
  structural validation.
- It runs automatically after render and before burn. `Fail` blocks burning;
  `Warn` is surfaced to the user.
- Checks are stable IDs (`UDF-AVDP`, `UDF-TAG266`, `BDMV-MOBJ`, `DVD-PGC`,
  `DVD-LAYER-BREAK`, `BDMV-PGS-DS`, `BDMV-PGS-TS`, `BDMV-AUDIO-STREAM`,
  `DVD-SPU-SEQ`, `DVD-SPU-PALETTE`, `BDMV-IG-BUTTONS`, `BDMV-IG-NAV`,
  `BDMV-MOTION-THUMB`, `DVD-SPU-BUTTONS`, etc.). Keep IDs stable for UI and tests.
- Do not add writes, temp files or mutation to the verifier.

### 5. Tauri/Svelte IPC

- Every `#[tauri::command]` must be registered in
  `apps/desktop/src-tauri/src/lib.rs` (`collect_commands!`).
- Add any new serialized types to the Specta `.typ::<...>()` list.
- Regenerate `bindings.ts` after every signature or type change.
- Never call `invoke()` directly from components; use the generated
  `commands.*` wrappers and `unwrap()`/`typedError`.
- All command errors are currently `String`; do not silently discard
  `Result`/`Option` in command implementations.
- `project_save` validates with `ensure_valid`; invalid projects must not be
  persisted by the normal save command.

### 6. Native subtitles and audio mastering

- Text subtitle authoring goes through `kinodisc-subtitle`; do not route ASS/VTT
  through the legacy TextSub XML. DVD uses four-color PNGs plus Spumux `<spu>`;
  Blu-ray/AVCHD uses generated PGS `.sup` files in the combined MKV.
- PGS and SPU buffers are bounded; render one 1080p bitmap at a time. Resolved
  font bytes and subtitle style are part of the render cache identity.
- `AudioMastering` (loudnorm/channel mode/codec/passthrough) and per-track gain,
  delay and format must feed one `-af`/`-filter:a:N` chain. Keep mastering
  fields in `audio_params_hash` and bump `ENCODER_REV` when arguments change.
- AVCHD audio is a vector end to end: FFmpeg per-stream args, tsMuxeR meta,
  native PMT/MPLS/CLPI and language registration must all agree. Pass-through
  is only legal for an unfiltered, codec-matching source; otherwise re-encode.
- Subtitle style is persisted on `SubtitleTrack.style`; native DVD/AVCHD
  rendering must use it rather than a global default.

### 7. Domain/project compatibility

- Project schema is version 1. Add new fields with `#[serde(default)]` and
  `specta::Type`; old projects must continue to load.
- `Project::validate` is authoritative: render, plan and save must use it.
- DVD-Video bitmap subtitles are metadata-only and rejected by the text
  pipeline with `W_BITMAP_SUBTITLE_UNSUPPORTED`. Do not feed PGS/VobSub to
  Spumux.
- Blu-ray/AVCHD `hdmv_pgs_subtitle` tracks are stream-copied and registered in
  MPLS/CLPI. Non-PGS bitmap codecs remain unsupported; PGS delay is ignored with
  `W_PGS_DELAY_IGNORED` and mixed episode layouts skip with
  `W_PGS_LAYOUT_MISMATCH`.
- HDMV DTS is kept equal to PTS until menu playback is validated on hardware.
- Menu clip timing fields (`out_ms`, `loop_duration_ms`) must be honored by the
  FFmpeg filter/argument builders.
- Template metadata (`Menu.template`) is additive; applying a template must
  never silently truncate episodes. Page 0 stays the VMGM root and later pages
  are VTSM EpisodeSelect menus with ordinary Prev/Next GotoMenu buttons.
- `Menu.transition` is a pre-rendered clip. It is concatenated before the menu
  loop; subpicture/IG start and menu audio are delayed to the transition end.
  Transition duration/easing and source SHA are part of the menu cache key. The
  current FFmpeg matrix has no fade-curve option, so the boundary fade is linear
  while easing metadata remains authoritative for future encoders.

### 8. HDMV

- IG segments use canonical PDS/WDS/ODS/ICS/END ordering.
- MovieObject commands are 12 bytes. `HdmvCommand::from_bytes`/`decode_command`
  must round-trip canonical commands.
- PGS slot estimation must account for width and height; `dummy_pgs_for_slots`
  measures a real dummy display set, not a worst-case pixel estimate.
- `BdmvIsoBuilder` must validate `index.bdmv`, `MovieObject.bdmv` and the
  playlist tree before publishing; the verifier does the independent deep check.

### 9. Bounded memory and 32-bit I/O

- No production path may `fs::read` an ISO, M2TS, VOB or other multi-gigabyte
  payload. Use the 256 KiB streaming helpers in `kinodisc-verify::stream`,
  `kinodisc-hdmv::inject_ig_into_menu_m2ts_file` or equivalent.
- Buffers are 64 KiB–1 MiB. `vendor/libmkiso` uses a 1 MiB payload/digest
  buffer and a file-backed `FileSource`; never add a full-image allocation.
- `i686` is a first-class target: sector/extent math uses `u64`/`u128`, packet
  counts use checked `u32` conversion, and the i686 check/test jobs must stay
  green. 32-bit Windows carries `/LARGEADDRESSAWARE` in
  `.cargo/config.toml`.

### 10. Native BDMV muxer and portable toolchain

- `kinodisc-hdmv::native` emits canonical INDX0200/MOBJ0200/MPLS0200/HDMV0200
  structures; keep its tests and the `kinodisc-verify` integration test green.
- tsMuxeR is optional. `MuxBackend::Auto` chooses native on 32-bit hosts or
  when tsMuxeR is absent, and tsMuxeR otherwise. A native failure on 64-bit may
  fall back with `W_NATIVE_MUX_FALLBACK`; on 32-bit it must fail structurally.
- Portable discovery checks `KINODISC_TOOLCHAIN_DIR`, `<exe>/toolchain/`,
  `<exe>/bin/` and platform-arch variants before resources and PATH.
- Native menu clips are encoded to **Matroska** (`menu.mkv`), not raw
  `.h264`: FFmpeg ≥ 9 refuses to stream-copy timestamp-less H.264 ES into
  MPEG-TS. tsMuxeR keeps the raw `.h264`. `NativeStream::video_attr` must come
  from `AvchdVideoConfig::bdmv_video_attr()` (format << 4 | rate).

### 12. Frame rates, stream copy and audio reconciliation

- 1080p 25/29.97 is **PsF** (`HdScan::Psf`): x264 `fake-interlaced=1:pic-struct=1`,
  CPU only, signalled as 1080i in BDMV. 1080p 50/59.94 forces Level 4.2.
- `kinodisc-plan::stream_copy::avchd_copy_decision` is the single authority
  for Smart Stream Copy; unknown probe data means re-encode. The render reads
  the decision from the plan (`EpisodeBitrate.stream_copy`) and includes it in
  the cache `profile`.
- `reconcile_episode_audio` and `harmonize_avchd_audio` run before every
  encode; never map an audio stream index the probe does not list.

### 14. Bundles, batch and provisioning

- `kinodisc-project::bundle` is the only bundle reader/writer: manifest
  first, deterministic headers (mtime 0, uid/gid 0, mode 0644), streamed
  hashing, traversal-safe import with full rollback. Never trust an entry
  that is not in the manifest.
- `kinodisc-cli batch` reuses `render::render`; keep concurrency 1.
- `kinodisc-toolchain::provision::DOWNLOADS` entries must carry the exact
  size and SHA-256 published for the asset; never extract or run anything
  before the checksum matches. The per-user toolchain folder is searched
  right after `KINODISC_TOOLCHAIN_DIR`.

### 15. Native DVD ISO, tool builds and accessibility

- `kinodisc-iso::dvd_native` writes the DVD-Video image; keep the
  canonical contiguous file order (IFO sector offsets depend on it) and
  validate with `kinodisc-verify` plus the dual-reader test. Never put
  anything but DVD-Video files in VIDEO_TS.
- `.github/workflows/build-dvdauthor.yml` builds the Windows DVDAuthor
  from the checksummed 0.7.2 tarball; changing it changes the zip SHA-256,
  so update `provision::DOWNLOADS` together with a new pre-release.
- Header dialogs must not live under an element with `backdrop-filter`
  (it becomes their containing block). Secondary screens and non-es/en
  locales are lazy; keep start-up JavaScript under 500 kB.

### 16. Kinodisc 1.0 additions

- **Upgrade compatibility** lives in `kinodisc-toolchain::legacy`: read
  environment variables with `legacy::env_var("KINODISC_X")` (it falls back
  to `DVDFORGE_X`/`DVD_FORGE_X`) and get the per-user folder from
  `legacy::app_data_dir()` (migrates `org.dvdforge.desktop` once). Never read
  `std::env::var("KINODISC_…")` directly.
- **Tool trust**: provisioned tools carry per-file SHA-256 in their
  provenance record; `provision::is_trusted_binary` refuses changed/unknown
  files and planted libraries in the per-user tool folder. Tool search must
  never include the current working directory.
- **Design system**: `apps/desktop/src/theme.css` is generated (light/dark
  remap of the slate/cyan/status palettes); components use the `ui-*` classes
  in `app.css`. `slate-950` is the window background and `slate-50` the
  strongest text in both themes; text over video or black overlays must be
  `text-[#fff]` (`text-white` inverts in light mode). Dialogs need
  `role="dialog" aria-modal="true"` (editor shortcuts rely on it).
- **Menu items** (`menu.items`) are drawn by `editor/menuItems.ts` for the
  disc export and by `layers/ItemsLayer.ts` in the editor; keep them in sync.
  Generated ids starting with `tpl:` belong to templates/splits and may be
  replaced.
- **HDMV per-episode playback**: episode MovieObjects start at
  `base + menus`, play their own range playlist from `EPISODE_MARK_GPR` (1)
  and clear it before jumping. MPLS mark tables are 14 bytes per mark
  (`BDMV-MARKS` verifies it).
- **DVD episode encoding** runs `encode_concurrency()` jobs in parallel;
  everything after encoding (cache publish, subtitle mux) stays in disc order.
- New CLI commands share their modules with the app: `split`
  (`kinodisc-render::split`), `scenes` (`kinodisc-menu::chapters`),
  `slideshow` (`kinodisc-render::slideshow`), `inspect`
  (`kinodisc-verify::inspect`, read-only parsers, bounds-checked) and
  `render --folder` (`kinodisc-verify::extract`).

### 17. Kinodisc 1.1 additions

- **Picture fixes** (`Episode.filters`, serde default) go through
  `kinodisc-render::picture` only: the same chain for DVD, AVCHD/BD, the
  preview (`media_filter_preview`) and the quality sample. It is part of the
  episode cache key only when not neutral, and disables stream copy
  (`COPY_FILTERS`). Order: deinterlace → crop → rotate/mirror → denoise →
  colour → blurred sides.
- **Menu artwork without the editor**: `kinodisc-render::compose` draws the
  menus for CLI/batch renders (mirrors `buttonLayout.ts`/`exportMenu.ts`).
  Keep the two in sync. The menu cache key includes the composed PNG.
- **Templates** paint `tpl:bg` (removed when the user picks a background) and
  `tpl:decor-*` shapes; titles use `title_color`. Template names are shown
  translated; never use third-party names or trademarks for templates.
- **Join** (`kinodisc-render::join`) passes `-reinit_filter 0` per input and
  has a stall timeout; keep both. Join and slideshow intermediates are
  limited-range `yuv420p` (`scale=…:out_range=tv` plus `-color_range tv`):
  JPEG/MJPEG input is full range and would otherwise turn off stream copy.
- **Quality sample** (`kinodisc-render::sample`) must build its requests like
  the real render and pick compare frames by number (raw `.m2v` has no
  B-frame timestamps).
- **Network**: the only requests are tool downloads and the daily update
  check (`kinodisc-toolchain::update`, official repo only, nothing sent);
  `open_release_page` opens only official release URLs.
- **Writes chosen by the user**: `image_save_png` writes absolute `.png`
  paths with PNG data only; `system_shutdown` runs fixed arguments through
  absolute system paths.
- `cargo test` must never touch the user's data folder: tests that call
  commands set `XDG_DATA_HOME` to a temporary folder.
- **Production output** (`render_start`): the render works in
  `<output>/.kinodisc-work/<job>` and removes it when it ends (finished,
  failed or cancelled); only the image, named by the caller (`iso_name`,
  one `.iso` file name), is left in the chosen folder. Stale work folders
  older than 6 h are removed on the next render there.
- **QA walkthroughs** run with their own `XDG_DATA_HOME`,
  `KINODISC_CACHE_DIR` and `WEBVIEW2_USER_DATA_FOLDER`, so they never touch
  the user's projects, caches or browser storage.

### 18. Kinodisc 1.2 additions

- **Auto-play** (`MenuTiming.auto_play`, serde default): root menu only,
  DVD-Video only. The root VMGM PGC `<post>` becomes the Play All command.
- **Even volume** is `AudioMastering.loudness` on every track
  (`projectStore.setEvenVolume`); a levelled track is never passed through.
- **Data disc**: `kinodisc-iso::build_data_iso` writes UDF 1.02 from a host
  tree and refuses an output inside the source. Commands:
  `data_folder_size`, `data_iso_create`, `data_iso_cancel`. Nothing in the
  folder is ever executed.
- **AVCHD/Blu-ray audio layout** comes from the episode with the most tracks;
  soundless episodes get silent tracks (`AvchdAudioConfig.silent`;
  `AudioEncodeConfig.silent` on DVD). `Project::avchd_subtitle_layout` is the
  single rule for HD subtitles and is part of the episode cache profile
  (`"pgs"`/`"nopgs"`).
- **Interlaced and PsF HD** never use a hardware encoder.
- **Production output**: `render_start` works in
  `<output>/.kinodisc-work/<job>` and publishes only the named image (see
  section 17).
- **QA before every release** uses the isolated launch (see section 17).
- `ButtonAction::SetSubtitleTrack { track: Option<u8> }` (`None` = off): DVD
  `subtitle=N+64` / `subtitle=0`, HDMV `SetStream`. `kinodisc-menu::setup`
  generates the "Ajustes" page and its root link; it is recognised on re-run
  by structure (a non-root submenu whose buttons are only SetAudioTrack /
  SetSubtitleTrack / GotoMenu root), like the scene menus.
- With a subtitle menu on a DVD, the per-title default subtitle `<pre>` is
  guarded by a choice register (g2, or g3 when Play All uses g2) that the
  menu sets, so "Sin subtítulos" is not undone at each title.
- `publish_iso` never deletes a finished image: when the named image cannot
  be replaced it publishes "<name> (2).iso" … "(9).iso". Folder mode renders
  to `<name>.kinodisc-tmp.iso`; the export clears only `VIDEO_TS` / `BDMV`.
- A soundless episode makes the whole AVCHD/Blu-ray audio layout AC-3 (no
  pass-through), so the silent tracks can match it.
- Dialogs doing work set `ui.locked`; `ui.open()` does nothing while it is set.
- Deleting asks with the awaited native confirm (`utils/confirm.ts`);
  `window.confirm` does not block in the webview.

### 19. Kinodisc 1.2.1 additions

- **Erase**: `DiscBurner::erase(device, full)` (default: unsupported).
  Windows `IDiscFormat2Erase` after `IsCurrentMediaSupported`; Linux and
  FreeBSD `xorriso -outdev <dev> -blank fast|all`; macOS
  `drutil -drive <sel> erase quick|full`. The `burn_erase` command takes
  the burn slot and refuses media that `probe_media` does not report as
  rewritable.
- **IMAPI errors** keep their HRESULT: `map_com` writes
  `"<reason> (código 0xC0AA….)"` with `windows::imapi_reason`. The frontend
  (`i18n/burnErrors.ts`) matches the code first, then engine phrases, and
  shows reason + advice; keep both tables in sync.
- **Diagnostics are local**: "Copy details" builds the report in the page;
  there is no telemetry upload (see section 17, Network).
- `kinodisc-cli template <project> <name>` uses
  `kinodisc-menu::templates::apply_template` (kebab-case names from
  `MenuTemplateId`, `--list`), validates before saving, never writes an
  invalid project.
- `burn_start` lets an image without VIDEO_TS/BDMV through
  (`PROFILE-UNKNOWN` is not blocking): it is a data disc; any other failed
  check still blocks the burn.

### 13. Localization

- Spanish is the source language; English must stay complete. Run
  `npm run i18n:check` (fails on missing English keys or on Rust diagnostic
  codes without an entry in `src/lib/i18n/diagnosticCodes.ts`).

### 11. FreeBSD and multi-arch release

- `kinodisc-burn` FreeBSD uses `camcontrol` for enumeration/eject,
  `dvd+rw-mediainfo` for media, and `growisofs` with `xorriso` fallback.
- The headless workspace must compile on FreeBSD (CI VM) and the pure crates are
  cross-checked with `x86_64-unknown-freebsd`.
- Release artifacts cover Windows x86_64/i686, Linux x86_64/i686, macOS
  arm64/x86_64 and the FreeBSD x86_64 CLI (built in a VM). Keep the 32-bit and
  FreeBSD archive names stable. CI runs the whole headless test suite on
  FreeBSD.

## Development commands

Run these before committing. They are also the CI gate:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --exclude kinodisc-desktop --target i686-unknown-linux-gnu
cargo test -p kinodisc-plan -p kinodisc-verify -p kinodisc-hdmv --target i686-unknown-linux-gnu
cd apps/desktop && npm run check && npm run build
```

Vendored `libmkiso` tests (not part of the workspace):

```bash
cd vendor/libmkiso
cargo fmt -- --check
cargo test --all-features --locked
cargo test --no-default-features --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

FreeBSD (CI uses a native VM):

```bash
cargo check --workspace --exclude kinodisc-desktop
```

Ignored render smoke tests (require FFmpeg, DVDAuthor, Spumux, xorriso, tsMuxeR
and fixtures):

```bash
cargo test -p kinodisc-render --test render_smoke -- --ignored
```

Bindings:

```bash
cd apps/desktop
npm run bindings
```

## Release checklist

1. Update versions in `Cargo.toml` (workspace), `Cargo.lock`,
   `apps/desktop/package.json`, `apps/desktop/package-lock.json` and
   `apps/desktop/src-tauri/tauri.conf.json`.
2. Update `README.md`, `README.es.md`, `AGENTS.md` and `CHANGELOG.md`.
3. Regenerate bindings only if an IPC-visible type changed.
4. Run the full verification command list above plus the vendored gates.
5. Commit, create annotated tag `vX.Y.Z`, push `main` and the tag.
6. The `Release` workflow builds Windows x86_64/i686, Linux x86_64/i686 and
   macOS arm64/x86_64 artifacts and publishes the GitHub Release; verify the
   32-bit `.zip`/`.tar.gz` names before publishing.

## Common pitfalls

- `cargo test -p libmkiso` fails because the crate is excluded; run its tests
  from `vendor/libmkiso`.
- Tauri command signature changes without regenerating bindings break
  `npm run check`.
- `-dc` is deprecated for `mpeg2video`; use `-intra_dc_precision`.
- `+cgop` + `-sc_threshold -30000` makes FFmpeg fail for MPEG-2.
- The menu cache may return an artifact outside the output directory; always
  materialize it before Spumux/UI tests that expect `menu_base.mpg`.
- Do not use `Store::open_default().ok()` to hide cache errors; emit structured
  diagnostics.
- Do not patch the IFO region mask without also checking the BUP and VTS files.
- Do not add a manual `invoke()` fallback in Svelte.
- Do not commit generated `target/`, `dist/` or cache blobs.
