<p align="center">
  <img src="docs/screenshots/01-menu-studio.png" alt="DVD Forge Studio" width="100%" />
</p>

<h1 align="center">DVD Forge</h1>

<p align="center">
  Optical authoring studio for DVD-Video, AVCHD and Blu-ray, with interactive menus,
  non-destructive editing, navigation simulation and native burning.
</p>

<p align="center">
  <a href="README.es.md">🇪🇸 Español</a> · <strong>🇺🇳 English</strong>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License GPL-3.0-or-later" src="https://img.shields.io/badge/license-GPL--3.0--or--later-22d3ee?style=flat-square" /></a>
  <img alt="Linux x86_64 and i686" src="https://img.shields.io/badge/Linux-x86__64%20%7C%20i686-0f172a?style=flat-square&logo=linux" />
  <img alt="Windows x86_64 and i686" src="https://img.shields.io/badge/Windows-x86__64%20%7C%20i686-0f172a?style=flat-square&logo=windows" />
  <img alt="macOS" src="https://img.shields.io/badge/macOS-arm64%20%7C%20x86__64-0f172a?style=flat-square&logo=apple" />
  <img alt="Rust + Tauri v2 + Svelte 5" src="https://img.shields.io/badge/Rust%20%2B%20Tauri%20v2%20%2B%20Svelte%205-0f172a?style=flat-square&logo=rust" />
</p>

---

## One workshop for the whole disc

DVD Forge brings the media library, visual menu editor, trimmer, chapters, audio
and subtitle tracks, remote-control simulation, navigation map and final
production into a single window. The same project can be delivered as a classic
DVD-Video or as high-definition AVCHD/Blu-ray without duplicating the creative
work.

## Gallery

| WYSIWYG menu | Trimmer and chapters |
|---|---|
| ![Menu studio](docs/screenshots/01-menu-studio.png) | ![Trimmer](docs/screenshots/02-video-trimmer.png) |

| Simulator with remote | Virtual machine map |
|---|---|
| ![Simulator](docs/screenshots/03-dvd-simulator.png) | ![VM map](docs/screenshots/04-vm-flowchart.png) |

| Audio and subtitles | VBR optimizer |
|---|---|
| ![Tracks](docs/screenshots/05-audio-subtitles.png) | ![VBR](docs/screenshots/06-vbr-optimizer.png) |

## v0.4.0 highlights

### 32-bit memory and streaming

- Every multi-gigabyte path now uses bounded streaming buffers (64 KiB–1 MiB):
  source fingerprints, subtitle/label hashes, UDF payloads, burn verification
  and menu M2TS processing. No production path reads a DVD-9, BD-25/BD-50 or
  M2TS file into memory.
- The UDF writer shares node paths through `Arc<str>`, pre-reserves its object
  and extent storage, and keeps a 1 MiB payload/digest buffer. The >4 GiB and
  50 GiB sparse-image tests read through a file-backed source instead of
  `std::fs::read`.
- 32-bit Windows builds request `/LARGEADDRESSAWARE`, and the full authoring
  stack is checked on `i686-unknown-linux-gnu` with core tests running on the
  32-bit target.

### Portable, 32-bit-independent toolchain

- `DVDFORGE_TOOLCHAIN_DIR` and directories beside the executable
  (`toolchain/`, `toolchain/<platform>-<arch>/`, `bin/`) are searched before
  packaged resources and PATH, so a release can run as a zero-setup portable
  bundle.
- tsMuxeR is now optional: a native Rust + FFmpeg BDMV muxer builds the movie
  M2TS, `index.bdmv`, `MovieObject.bdmv`, MPLS, CLPI, chapter marks and HDMV
  IG menu clips when tsMuxeR is absent or the host is 32-bit. The existing
  tsMuxeR path remains the default on 64-bit hosts.

### Blu-ray PGS pass-through

- Active `hdmv_pgs_subtitle` tracks are stream-copied into AVCHD/Blu-ray
  M2TS and registered in the MPLS STN and CLPI program/sequence info.
- DVD-Video keeps `W_BITMAP_SUBTITLE_UNSUPPORTED` because Spumux cannot
  consume bitmap subtitles without OCR. Non-PGS bitmap codecs are still
  rejected for Blu-ray.
- PGS delay is ignored with `W_PGS_DELAY_IGNORED`; mixed per-episode PGS
  layouts are skipped with `W_PGS_LAYOUT_MISMATCH`.

### FreeBSD and multi-arch releases

- Native FreeBSD CLI and burning: drive discovery through `camcontrol`,
  media probing through `dvd+rw-mediainfo`, burning through `growisofs` with
  `xorriso` fallback, and eject through `camcontrol eject`.
- Release builds cover Windows x86_64/i686 (including 32-bit WebView2), Linux
  x86_64/i686, and macOS Apple Silicon/Intel. The 32-bit Windows and Linux CLI
  archives are published alongside the Tauri installers.
- Release binaries use thin LTO, one codegen unit and stripped symbols.

## v0.3.0 highlights

### Pre-burn structural verifier

- New independent, read-only crate **`dvdforge-verify`**. It parses the image
  itself instead of trusting the writer that produced it.
- **UDF 1.02/2.50:** AVDP, VRS, PVD, partition map, FSD, root ICB, Tag 266
  Extended File Entries (`information_length`, checksum and protected length),
  extent bounds, metadata mirror and optional full SHA-256 digest.
- **BDMV:** `index.bdmv`, `MovieObject.bdmv` (12-byte navigation commands and
  offsets), playlists and CLPI references, IN/OUT times, M2TS sync, PAT/PMT,
  video/audio/IG/PGS PIDs.
- **DVD-Video:** byte-for-byte `.IFO`/`.BUP` identity for VMG and every VTS,
  PGC/cell sector bounds, non-overlapping cells, DVD-9 SPLIP placement, region
  mask and VOB presence.
- CLI:

  ```bash
  dvdforge-cli verify disc.iso
  dvdforge-cli verify disc.iso --deep --json
  dvdforge-cli verify disc.iso --strict
  ```

- The desktop renderer runs the verifier automatically after building the ISO;
  `burn_start` runs it again before touching the drive. A `Fail` verdict blocks
  burning; `Warn` is shown in the production panel.

### Smart Render Cache v2

- `ArtifactKey` v2 covers **episodes, menu videos, menu overlays and intro
  clips** with an explicit `ArtifactKind`.
- Source identity uses `fp2:` full streaming SHA-256, not a size/mtime/edge
  sample. `inputs_hash` is a canonical hash of trim, fps, scale mode, encoder
  parameters, audio, subtitle contents and menu composition.
- SQLite `artifact_cache_v2` stores `kind`, `inputs_hash`, `encoder_rev`,
  `toolchain_hash`, `size_bytes`, `created_at` and `last_used_at`; `CacheLookup`
  distinguishes `Hit`, `Miss`, `Stale` and `Error`.
- Cache failures are diagnostics (`W_CACHE_DISABLED`, `W_CACHE_READ_FAILED`,
  `W_CACHE_WRITE_FAILED`, `W_CACHE_STALE`), never silent render failures.
- LRU pruning and cache statistics are available through the store API.
- Editing one menu, subtitle or intro no longer re-encodes unchanged video
  streams.

### Cross-platform hardware H.264 encoding

- Detects and smoke-tests **NVIDIA NVENC**, **Intel Quick Sync**, **VA-API**,
  **AMD AMF** and **Apple VideoToolbox** with a one-frame `lavfi` encode.
- Hardware encoding is used for AVCHD/Blu-ray H.264; classic DVD-Video remains
  on the tuned CPU MPEG-2 path.
- Output is checked with `ffprobe` for H.264, High profile and `yuv420p`; any
  driver/encode/compliance failure emits `W_HW_ENCODER_FALLBACK` and re-encodes
  with CPU `libx264`.
- The production dialog exposes `Hardware if available` and `CPU only`;
  `ToolchainDoctor` lists the confirmed encoders.

### MKV/MP4 chapter and stream metadata import

- FFprobe now reads `-show_chapters`, stream titles, audio disposition
  (`default`, `comment`), subtitle disposition (`default`, `forced`) and bitmap
  codecs.
- Chapter markers are imported into the active trim range, converted to
  disc-rate frames, deduplicated and anchored at 00:00.
- Audio tracks inherit title/language/default/commentary metadata; subtitle
  tracks inherit title, language, default and forced flags.
- Bitmap subtitles (`hdmv_pgs_subtitle`, `dvd_subtitle`, `dvb_subtitle`, `xsub`)
  are detected and reported with `W_BITMAP_SUBTITLE_UNSUPPORTED`; they are never
  passed to the SRT/Spumux pipeline.

### Reference-grade 2-pass MPEG-2

- Lanczos scaling and DVD-compliant 720×480/720×576 output.
- `-mbd rd`, `-cmp rd`, `-subcmp rd`, `-trellis 1` in pass 1 and `-trellis 2`
  in pass 2.
- `-intra_dc_precision` 0/1/2 for 8/9/10-bit DC precision based on target
  bitrate.
- Closed GOPs (`+cgop`) with strict PAL (15) / NTSC (18) GOP lengths. Because
  FFmpeg rejects closed GOPs together with scene-cut detection, the encoder uses
  `-sc_threshold 1000000000` instead of `-30000`.
- Job-scoped passlog files; pass 1 writes to `NUL` on Windows and `/dev/null`
  on Linux/macOS.

### Hardware-accurate DVD-9 layer break

- The planner computes an ECC-aligned layer-break sector.
- Authoring patches the containing VTS PGC cell with the SPLIP/seamless flag and
  keeps `VTS_01_0.IFO` and `VTS_01_0.BUP` byte-identical.
- The verifier checks the SPLIP cell on DVD-9 images and reports unaligned or
  missing layer-break metadata as a structural failure.

## Key features

### Unified studio

- Single-window workbench with media library, Konva canvas, inspector, episode
  sequence and an always-visible capacity bar.
- Portable JSON projects with structural validation, disc budget and diagnostics
  before encoding.
- Recent-project autosave and job history in SQLite.
- Integrated toolchain doctor for FFmpeg, tsMuxeR, DVDAuthor, xorriso,
  growisofs, `dvd+rw-mediainfo` and libbluray.

### Classic DVD-Video

- DVD-5 and DVD-9 authoring with **OTP/PTP layer-break** calculation aligned to
  16-sector ECC blocks.
- Compatibility with living-room players, PlayStation 2/3/4/5, Xbox and software
  players.
- VMGM/VTSM menus, First Play, chapters, Play All and automatic return.
- Real DVD region mask in `VIDEO_TS.IFO` and `VIDEO_TS.BUP`, plus RCE checking.
- Two-pass MPEG-2, AC-3 2.0/5.1 and `.srt`/`.ass`/`.ssa`/`.vtt` subtitles with
  sync offset. In HD projects, tracks are preserved in the project and shown in
  the editor.
- Reference-grade MPEG-2 two-pass tuning (Lanczos, RD, Trellis, dynamic DC
  precision and strict PAL/NTSC GOPs).
- DVD-9 builds mark the layer-break cell in the VTS IFO and the verifier checks
  its ECC alignment and PGC placement.

### High definition

- **AVCHD on DVD-5/DVD-9** and **native Blu-ray BD-25/BD-50**.
- H.264 High with `bluray-compat`, closed GOP, 30,000–40,000 kbit VBV and up to
  38 Mbps on Blu-ray.
- Native **HDMV Interactive Graphics** menus: PDS, WDS, ODS with RLE, ICS and END,
  buttons with normal/selected/activated states and directional navigation.
- Nested menus: each menu is published as its own IG/Playlist clip and linked
  with `JumpObject`; episode and chapter buttons start at the exact mark through
  GPR and `PlayPL_PM`.
- Optional hardware H.264 encoding (NVENC, QSV, VAAPI, AMF, VideoToolbox) with
  automatic CPU fallback.
- Smart artifact cache for episodes, menus and intros, keyed by full source
  SHA-256 and canonical render inputs.
- Looping video backgrounds, moving-thumbnail buttons and AC-3 menu audio with
  volume, fades and a silent backup track.
- Final **UDF 2.50 image with Metadata Partition** (Metadata File + Metadata
  Mirror File), readable by libbluray, living-room players and consoles.

### Visual editor

- WYSIWYG canvas with even coordinates, grid, safe areas and action guides.
- Six button styles: 16:9 thumbnail, classic, glass, Play badge, text only and an
  invisible **hotspot** over custom artwork.
- Automatic geometry-based navigation with manual overrides per direction.
- Thumbnails and frames extracted from the original video.
- Static and animated menu previews before producing the disc.

### Media and accessibility

- Direct import of `.mp4`, `.mkv`, `.avi`, `.mov` and other common containers.
  The editor builds a cached MP4 proxy for preview but always keeps the original
  file as the highest-quality source.
- Non-destructive **In/Out** trimming and chapter management by time or frame.
- Multiple AC-3 2.0/5.1 audio tracks, default track, gain and delay.
- External subtitles with language, label, forced flag and offset.
- Interactive simulator with virtual remote, focus, activation and menu buttons.
- Virtual-machine node map with the project's real transitions.

### Internationalization

- Reactive Svelte 5 i18n engine with no heavy dependencies.
- Seven interface locales: **Spanish, English, Portuguese, French, German,
  Italian and Japanese**.
- Automatic first-run detection from `navigator.language`, persisted in
  `localStorage` (`dvdforge_locale`) and switchable live from the header.
- Official ISO 639 language catalog for audio and subtitle tracks: 2-letter
  ISO 639-1 codes for DVD-Video and 3-letter ISO 639-2/T codes for
  Blu-ray/AVCHD, including bibliographic aliases (`fre`, `ger`, `chi`, `baq`).
- FFprobe `tags.language` values are normalized into the catalog automatically
  when importing media.
- Bilingual product documentation: this English README and
  [README.es.md](README.es.md).

### Production and burning

- Shared pipeline between the desktop app and the CLI: the same menus,
  subtitles, regions and budgets apply in both environments.
- Native optical burning on Linux (`growisofs` with `xorriso` fallback), Windows
  (`IMAPI2`) and macOS (`drutil`/`hdiutil`).
- DVD-R/RW, DVD+R/RW, BD-R, BD-RE and BD-R DL support, with MMC profile, capacity
  and real media-speed detection.
- Optional SHA-256 verification and clean cancellation of render or burn jobs.
- Structural verification of the finished ISO after render and again before
  burning; failed checks prevent the burn.
- Smart Render Cache v2 with `fp2:` full-source SHA-256 and deterministic
  `inputs_hash`, covering episodes, menus, overlays and intros.

## Optical format architecture

| | DVD-Video | AVCHD | Blu-ray |
|---|---|---|---|
| Media | DVD-5 / DVD-9 | DVD-5 / DVD-9 | BD-25 / BD-50 |
| Video | MPEG-2, two-pass | H.264 High | H.264 High, up to 38 Mbps |
| Audio | AC-3 2.0/5.1 | AC-3 2.0/5.1 | AC-3 2.0/5.1 |
| Menus | VMGM/VTSM, SPU buttons | HDMV Interactive Graphics | HDMV Interactive Graphics |
| Navigation | DVD virtual machine | IG + Playlist + GPR | IG + Playlist + GPR |
| File system | UDF 1.02 / ISO 9660 | UDF 2.50 | UDF 2.50 + Metadata Partition |
| Layer break | OTP/PTP, 16-sector ECC | n/a | n/a |
| Typical targets | Players, PS2–PS5, Xbox | PS4/PS5, Blu-ray players | Blu-ray players, PS4/PS5 |

## Language catalog

| Language | ISO 639-1 (DVD) | ISO 639-2/T (Blu-ray/AVCHD) | Bibliographic alias |
|---|---|---|---|
| Spanish | `es` | `spa` | — |
| English | `en` | `eng` | — |
| Japanese | `ja` | `jpn` | — |
| Portuguese | `pt` | `por` | — |
| French | `fr` | `fra` | `fre` |
| German | `de` | `deu` | `ger` |
| Italian | `it` | `ita` | — |
| Russian | `ru` | `rus` | — |
| Chinese | `zh` | `zho` | `chi` |
| Korean | `ko` | `kor` | — |
| Catalan | `ca` | `cat` | — |
| Basque | `eu` | `eus` | `baq` |
| Galician | `gl` | `glg` | — |
| Arabic | `ar` | `ara` | — |
| Hindi | `hi` | `hin` | — |
| Undetermined / Original | `und` | `und` | — |

## Platforms

| Platform | Architecture | Distribution |
|---|---|---|
| Linux | x86_64 | AppImage, `.deb`, CLI tarball |
| Linux | i686 | CLI tarball and 32-bit core test job |
| Windows | x86_64 | MSI, NSIS, portable CLI |
| Windows | i686 | 32-bit MSI/NSIS with x86 WebView2, portable CLI zip |
| macOS | Apple Silicon / Intel | `.dmg`, CLI tarball |
| FreeBSD | x86_64 | Source build, CLI and native burning |

> On Linux i686 the CLI and the entire authoring stack are natively supported.
> The 32-bit Linux desktop is not published because WebKitGTK/GTK 32-bit is not
> available in every distribution.

## Requirements

### Linux

System tools:

```bash
# Arch / CachyOS
sudo pacman -S --needed ffmpeg dvdauthor dvd+rw-tools libisoburn udftools \
  gstreamer gst-plugins-base gst-plugins-good gst-libav

# Debian / Ubuntu
sudo apt install ffmpeg dvdauthor dvd+rw-tools xorriso udftools \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-libav \
  libwebkit2gtk-4.1-0 libgtk-3-0

# Fedora
sudo dnf install ffmpeg dvdauthor dvd+rw-tools xorriso udftools \
  gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free \
  webkit2gtk4.1 gtk3
```

`tsMuxeR` is optional on 64-bit hosts: AVCHD/Blu-ray can be authored by the
native FFmpeg path when it is absent. Place tools in `toolchain/` next to the
binary, set `DVDFORGE_TOOLCHAIN_DIR`, or put them on `PATH`.
`libbluray`/`bd_info` are optional and are used to validate the ISO.

### Windows

- Windows 10 64-bit or 32-bit and Windows 11 64-bit, with WebView2.
- FFmpeg/FFprobe and DVDAuthor/Spumux available on `PATH`; tsMuxeR is optional.
- Burning uses the native IMAPI2 API; no external tools are required.

### macOS

Run the `.dmg` installer from the releases page or build from source. Burning
uses `drutil`/`hdiutil`; FFmpeg is required and tsMuxeR is optional.

### FreeBSD

```bash
pkg install ffmpeg dvdauthor dvd+rw-tools xorriso
```

Burning uses `camcontrol`, `growisofs` and `xorriso`; tsMuxeR is optional.

## Installation

### Packages

Download the appropriate installer from the releases page:

- Linux x86_64: `.AppImage`, `.deb` and CLI tarball.
- Linux i686: `dvdforge-cli-i686-unknown-linux.tar.gz`.
- Windows x86_64/i686: `.msi`, NSIS installer and portable CLI.
- Windows i686 CLI: `dvdforge-cli-i686-pc-windows-msvc.zip`.
- macOS Apple Silicon/Intel: `.dmg` and CLI tarball.
- Universal CLI: `dvdforge-cli` to automate plans, renders and burning.

### Build from source

```bash
# Frontend dependencies
cd apps/desktop
npm ci

# Desktop application
npm run tauri build

# CLI
cd ../..
cargo build --release -p dvdforge-cli
```

To build the 32-bit CLI:

```bash
rustup target add i686-unknown-linux-gnu
cargo build --release -p dvdforge-cli --target i686-unknown-linux-gnu
```

## Quick start

1. Create a DVD-Video, AVCHD or Blu-ray project.
2. Import the videos and drag them into the sequence.
3. Trim In/Out, add chapters and adjust audio/subtitles.
4. Design the menu, assign actions and check navigation in the simulator.
5. Open **Produce disc**, choose ISO or ISO + burning and select the drive.
6. Verify the result with a player or burn the final disc.

## CLI

```bash
# Check system tools
dvdforge-cli doctor

# Inspect a file
dvdforge-cli probe movie.mkv --json

# Calculate a project budget
dvdforge-cli plan project.json -o plan.json --probe

# Build the ISO with the same pipeline as the desktop app
dvdforge-cli render project.json -o output

# Structural verification (UDF/BDMV/DVD) before burning
dvdforge-cli verify output/disc.iso --deep --strict

# List drives and burn
dvdforge-cli devices
dvdforge-cli burn output/disc.iso --device /dev/sr0 --speed 4
```

The CLI shares the renderer, HDMV menus, DVD subtitles, regions, BD-25/BD-50
budget and artifact cache with the desktop application.

## Technology

- **Rust** for the domain, planning, encoding, authoring, ISO/UDF, HDMV menus,
  SQLite persistence, structural verification and optical burning.
- **Tauri v2** as the secure native bridge between the interface and the system.
- **Svelte 5 + Vite + Tailwind CSS 4 + Konva** for the visual editor.
- **FFmpeg**, **tsMuxeR**, **DVDAuthor/Spumux**, **xorriso**, **growisofs** and
  **libbluray** as external engines.
- Optional **NVENC / QSV / VAAPI / AMF / VideoToolbox** hardware H.264 paths with
  automatic CPU fallback.

## Roadmap

The v0.5.0–v1.0.0 engineering plan covers native subtitle rendering and audio
mastering, the Studio menu designer 2.0, smart stream copy, checkpointing,
fuzzing, packaging and the v1.0.0 LTS schema freeze. See
[ROADMAP.md](ROADMAP.md) for target crates, deliverables, check IDs and
verification criteria.

## License

DVD Forge is distributed under the **GNU General Public License v3.0 or later**
(`GPL-3.0-or-later`). See [LICENSE](LICENSE) for the full text.
