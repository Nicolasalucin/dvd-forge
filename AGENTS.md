# AGENTS.md — DVD Forge engineering guide

This file is the high-signal orientation for any AI coding agent working in this
repository. Read it before changing code. The project ships optical disc images;
several invariants are hardware- or format-critical and are not obvious from a
single crate.

## What DVD Forge is

DVD Forge is a Rust + Tauri v2 + Svelte 5 optical authoring studio for:

- **DVD-Video** (DVD-5/DVD-9, MPEG-2, AC-3, DVDAuthor/Spumux, DVD-9 layer break).
- **AVCHD** (H.264 on DVD media, HDMV Interactive Graphics menus).
- **Blu-ray** (BD-25/BD-50, UDF 2.50 metadata partition, HDMV menus).

The desktop app and CLI share the same render pipeline in `dvdforge-render`.

## Repository map

```text
crates/
  dvdforge-domain/       Pure models, validation, geometry, menus, diagnostics
  dvdforge-project/      Project JSON load/save/validate (schema v1)
  dvdforge-media/        FFprobe adapter: streams, tags, chapters, dispositions
  dvdforge-menu/         SPU/menu layer rasterization, Spumux/TextSub XML
  dvdforge-plan/         Bitrate planning, DVD-9 ECC layer-break sector math
  dvdforge-dvd/          MPEG-2/AC-3 encoding, DVDAuthor XML, IFO patching
  dvdforge-avchd/        H.264/AC-3 encoding, tsMuxeR metadata, hardware args
  dvdforge-hdmv/         HDMV IG segments, opcodes, M2TS/MPLS/CLPI/MovieObject
  dvdforge-iso/          DVD-Video ISO builder and BDMV UDF 2.50 builder
  dvdforge-burn/         Platform burners: growisofs/xorriso, IMAPI2, drutil
  dvdforge-jobs/         Job events, stages, diagnostics-facing types
  dvdforge-toolchain/    Binary discovery, tool doctor, hardware encoder probes
  dvdforge-store/        SQLite recent projects, job history, artifact cache v2
  dvdforge-testkit/      Shared test helpers
  dvdforge-cli/          CLI: doctor, probe, plan, render, verify, devices, burn
  dvdforge-render/       End-to-end DVD/AVCHD render orchestration and cache
  dvdforge-verify/       Independent read-only UDF/BDMV/DVD structural verifier

vendor/libmkiso/         Vendored UDF/ISO writer+reader (upstream 0.1.5 + patches)
vendor/libmkiso/DVDFORGE-PATCH.md
                         Local patch rationale; read before touching vendor code

apps/desktop/            Svelte 5 + Vite + Tailwind frontend
apps/desktop/src-tauri/  Tauri v2 commands, Specta bindings, platform setup
```

Generated IPC bindings live at:

```text
apps/desktop/src/lib/api/bindings.ts
```

Do not edit that file manually. Regenerate it with `npm run bindings` (or
`cargo run -p dvdforge-desktop --bin export-bindings`).

## Critical invariants — do not break

### 1. `vendor/libmkiso`: 64-bit sectors, Tag 266, UDF layout

- All ISO/UDF sector and extent arithmetic that can exceed 4 GiB must use
  `u64`/`u128`; never `usize` or `u32`. The CLI and authoring stack are compiled
  for **i686** and must handle BD-25/BD-50 and >4 GiB M2TS files.
- UDF 2.50 metadata files must use **Extended File Entries (Tag 266)** with long
  allocation descriptors. The local patch is documented in
  `vendor/libmkiso/DVDFORGE-PATCH.md`; the rest is upstream 0.1.5.
- UDF images must keep the trailing AVDPs within the final 32-sector/64 KiB
  boundary, and metadata mirrors must remain readable.
- Do not replace the reader with the writer for validation. The verifier needs
  an independent parse.
- Run `vendor/libmkiso` tests from inside `vendor/libmkiso`; it is intentionally
  excluded from the workspace.

### 2. `dvdforge-render`: MPEG-2 flags and cache determinism

The DVD-Video software encode path is tuned deliberately:

- Scaling uses `flags=lanczos`.
- RD/Trellis: `-mbd rd`, `-cmp rd`, `-subcmp rd`; `-trellis 1` on pass 1 and
  `-trellis 2` on pass 2.
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

### 3. `dvdforge-dvd`: DVD-9 layer break and IFO consistency

- The planner computes an ECC-aligned layer-break sector (16 DVD sectors).
- Authoring must call `apply_layer_break` (or equivalent) and set the **SPLIP**
  bit on the containing VTS PGC cell.
- `VTS_01_0.IFO` and `VTS_01_0.BUP` must be byte-identical after patching.
  `VIDEO_TS.IFO`/`VIDEO_TS.BUP` must also remain identical.
- A DVD-9 build without a valid SPLIP cell is structurally broken; the verifier
  must fail it.
- Do not claim in the UI that the layer break is applied unless the authoring
  path actually does it.

### 4. `dvdforge-verify`: independent and read-only

- The verifier is a read-only parser. It must not reuse writer code paths for
  structural validation.
- It runs automatically after render and before burn. `Fail` blocks burning;
  `Warn` is surfaced to the user.
- Checks are stable IDs (`UDF-AVDP`, `UDF-TAG266`, `BDMV-MOBJ`, `DVD-PGC`,
  `DVD-LAYER-BREAK`, etc.). Keep IDs stable for UI and tests.
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

### 6. Domain/project compatibility

- Project schema is version 1. Add new fields with `#[serde(default)]` and
  `specta::Type`; old projects must continue to load.
- `Project::validate` is authoritative: render, plan and save must use it.
- Bitmap subtitles are metadata-only in v0.3.0; they are rejected by the text
  pipeline with `W_BITMAP_SUBTITLE_UNSUPPORTED`. Do not feed PGS/VobSub to
  Spumux.
- HDMV DTS is kept equal to PTS until menu playback is validated on hardware.
- Menu clip timing fields (`out_ms`, `loop_duration_ms`) must be honored by the
  FFmpeg filter/argument builders.

### 7. HDMV

- IG segments use canonical PDS/WDS/ODS/ICS/END ordering.
- MovieObject commands are 12 bytes. `HdmvCommand::from_bytes`/`decode_command`
  must round-trip canonical commands.
- PGS slot estimation must account for width and height; `dummy_pgs_for_slots`
  measures a real dummy display set, not a worst-case pixel estimate.
- `BdmvIsoBuilder` must validate `index.bdmv`, `MovieObject.bdmv` and the
  playlist tree before publishing; the verifier does the independent deep check.

## Development commands

Run these before committing. They are also the CI gate:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --exclude dvdforge-desktop --target i686-unknown-linux-gnu
cd apps/desktop && npm run check && npm run build
```

Vendored `libmkiso` tests (not part of the workspace):

```bash
cd vendor/libmkiso
cargo test
```

Ignored render smoke tests (require FFmpeg, DVDAuthor, Spumux, xorriso, tsMuxeR
and fixtures):

```bash
cargo test -p dvdforge-render --test render_smoke -- --ignored
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
2. Update `README.md`, `README.es.md` and `CHANGELOG.md`.
3. Regenerate bindings.
4. Run the full verification command list above.
5. Commit, create annotated tag `vX.Y.Z`, push `main` and the tag.
6. The `Release` workflow builds Linux/Windows artifacts and publishes the
   GitHub Release; use `gh release edit` for final release notes if needed.

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
