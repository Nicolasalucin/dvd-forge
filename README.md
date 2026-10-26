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
  <img alt="macOS" src="https://img.shields.io/badge/macOS-source%20build-0f172a?style=flat-square&logo=apple" />
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

### High definition

- **AVCHD on DVD-5/DVD-9** and **native Blu-ray BD-25/BD-50**.
- H.264 High with `bluray-compat`, closed GOP, 30,000–40,000 kbit VBV and up to
  38 Mbps on Blu-ray.
- Native **HDMV Interactive Graphics** menus: PDS, WDS, ODS with RLE, ICS and END,
  buttons with normal/selected/activated states and directional navigation.
- Nested menus: each menu is published as its own IG/Playlist clip and linked
  with `JumpObject`; episode and chapter buttons start at the exact mark through
  GPR and `PlayPL_PM`.
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
- SHA-256 artifact cache to avoid re-encoding unchanged sources, trims, audio or
  bitrates.

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
| Linux | x86_64 | AppImage, `.deb`, CLI |
| Linux | i686 | CLI and portable desktop |
| Windows | x86_64 | MSI, NSIS, portable CLI |
| Windows | i686 | 32-bit MSI, 32-bit NSIS, portable CLI |
| macOS | Apple Silicon / Intel | Build from source |

> On Linux i686 the CLI and the entire authoring stack are natively supported.
> The desktop app is published as a portable binary when the distribution
> provides 32-bit WebKitGTK/GTK; some distributions only ship those libraries
> for 64-bit.

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

`tsMuxeR` is distributed as a standalone binary: download it and place it on
`PATH`. `libbluray`/`bd_info` are optional and are used to validate the ISO.

### Windows

- Windows 10 64-bit or 32-bit and Windows 11 64-bit, with WebView2.
- FFmpeg/FFprobe, tsMuxeR and DVDAuthor/Spumux available on `PATH`.
- Burning uses the native IMAPI2 API; no external tools are required.

### macOS

The application can be built from source. Burning uses `drutil`/`hdiutil`;
FFmpeg, tsMuxeR and the authoring tools must be installed and accessible.

## Installation

### Packages

Download the appropriate installer from the releases page:

- Linux x86_64: `.AppImage` or `.deb`.
- Linux i686: portable desktop binary and CLI.
- Windows x86_64/i686: `.msi`, NSIS installer and portable CLI.
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

# List drives and burn
dvdforge-cli devices
dvdforge-cli burn output/disc.iso --device /dev/sr0 --speed 4
```

The CLI shares the renderer, HDMV menus, DVD subtitles, regions, BD-25/BD-50
budget and artifact cache with the desktop application.

## Technology

- **Rust** for the domain, planning, encoding, authoring, ISO/UDF, HDMV menus,
  SQLite persistence and optical burning.
- **Tauri v2** as the secure native bridge between the interface and the system.
- **Svelte 5 + Vite + Tailwind CSS 4 + Konva** for the visual editor.
- **FFmpeg**, **tsMuxeR**, **DVDAuthor/Spumux**, **xorriso**, **growisofs** and
  **libbluray** as external engines.

## License

DVD Forge is distributed under the **GNU General Public License v3.0 or later**
(`GPL-3.0-or-later`). See [LICENSE](LICENSE) for the full text.
