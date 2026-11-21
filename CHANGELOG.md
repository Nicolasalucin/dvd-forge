# Changelog

## 1.2.2 — 2026-11-21

### Added

- **`kinodisc-cli template`** applies a menu template from the command line, with the same engine as Menu › Templates in the app: `kinodisc-cli template project.json multi-page-tv-grid`. `kinodisc-cli template --list` lists the 15 templates. A whole disc (template, scene menus, render, verify) can now be made without opening the app, for example by a script or an AI assistant. The project is checked before it is saved; a broken one is never written.

## 1.2.1 — 2026-11-21

### Added

- **Erase rewritable discs.** Disc › Erase rewritable disc… makes a DVD-RW, DVD+RW or BD-RE ready to burn again, quick or full. It checks that the disc is rewritable before touching the drive.
- **Burn errors explain why.** A failed burn or erase says what went wrong in plain words (no disc, disc upside down, not empty, speed, drive locked, USB power…) and what to try. A used rewritable disc offers to erase it. **Copy details** copies a short technical report for asking for help; nothing is sent anywhere.

### Fixed

- **Data discs can be burned**, from the app and with `kinodisc-cli burn`. The check before burning refused every image without VIDEO_TS or BDMV, so the data discs made with Disc › Data disc… could only be saved, not burned. `kinodisc-cli verify` now says when an image is a data disc.
- **A data disc never replaces an existing file**: if the output folder already has an image with that label, Kinodisc asks for another label or folder.
- **Escape closes the dialog on top** (Erase disc over Produce disc), and the Data disc and Erase disc dialogs take the keyboard focus.

## 1.2.0 — 2026-11-19

New disc features, plus the fixes from several complete tests of the app by an independent QA pass. (1.1.3 was never released: its fixes are in this version.)

### Added

- **Auto-play.** A toggle on the main menu makes the DVD start playing everything when nobody chooses anything by the time the menu ends. Before, the menu waited forever.
- **Even volume across the disc.** One checkbox in Audio and subtitles levels the volume of every episode, so you don't reach for the remote between videos.
- **Data disc.** Disc › Data disc… burns any folder (photos, documents, the original files) to a CD, DVD or Blu-ray that opens on a computer. It tells you which disc the folder fits.
- **Several copies in a row.** Choose how many copies to burn; after each one the app asks for the next blank disc.
- **Audio and subtitles menu on the disc.** A generated Settings page lets the viewer pick the audio language and the subtitles with the remote, as on retail discs.
- **Disc label field** in the disc settings.

### Fixed

- **Choosing BD-25/BD-50 keeps your HD format.** A 1080i 25 fps project used to become 1080p 23.976 NTSC, with the menu squeezed to 480 lines. The resolution, frame rate and menus now stay as they are. Chapters and trims keep their times whenever the disc frame rate changes (PAL/NTSC, DVD/HD).
- **Every episode can be chosen on the disc.** Each video you add to a menu you have not designed yet gets its own button; before, only the first one did. The production dialog warns before you start when no button, Play All or "next episode" can start an episode.
- **One disc per episode works.** Discs 2 and later no longer fail when only the first episode had a button, and they no longer carry a scene menu that belongs to another disc.
- **Only the image is left in the output folder.**
  - The work files (encoded episodes, menus, the disc tree) go to a temporary folder. It is removed when the production ends, fails or is cancelled.
  - The ISO is named after the project ("Boda.iso", "Serie - Disc 2.iso") instead of `disc.iso`.
  - The output folder is remembered instead of going back to Videos.
  - Every disc of a multi-disc run is listed.
- **Default audio and subtitles.** The Default controls always show the real state, and a subtitle default can be turned off. The default subtitle is now switched on when the DVD title starts.
- **Preview.** Portrait videos are framed as on the disc: whole, with blurred sides if chosen, instead of zoomed. The remote works right after "Root menu", "Title" or "Back".
- **Closing the window while a disc is being made or burned asks first**, and stops the production cleanly. The unsaved-changes question offers Save.
- **Job history** shows past jobs when it opens (it always looked empty). Disc-folder jobs point to the folder.
- **Old results, errors and notices no longer stay on screen** after you open another project or try again.
- **The progress bar is honest.** It no longer reads 100% for minutes while episodes are still encoding, and it never goes backwards.
- **The structural verifier checks Blu-ray menu buttons.** It used to read the menu one byte too late and report "0 IG buttons".
- **The interface language everywhere.** The production log, the structural check list, errors, scene menus on the disc ("Scenes", "Start") and number formats ("4.7 GB", "1.8%" in English) are now translated. Spanish uses "computadora" throughout, and English uses US spelling.
- **Menus.**
  - Long button names shrink a little, then end in "…", instead of being squeezed until unreadable.
  - Emoji are drawn.
  - Long menu text shrinks to fit its box.
  - Scene buttons show their own chapter frame in the editor.
  - New buttons never land on the title.
  - The Scenes link is a full-size button.
  - With the arrows, Left from the second button goes back to the first.
- **Import.**
  - Imported videos become episodes, as the guide says.
  - Subtitle files saved next to a video ("name.es.srt") are added with their language.
  - A file that cannot be read is named in a plain sentence.
  - Picking a file again does not add a duplicate.
- **A video without sound gets silent audio tracks**, on DVD and on Blu-ray/AVCHD, matching the rest of the disc. A Blu-ray that mixed a soundless video with one that has sound used to fail.
- **Blu-ray/AVCHD audio and subtitles are kept.**
  - The disc's audio layout comes from the episode with the most tracks, so a one-track first video no longer drops the other languages.
  - Episodes with different audio or subtitles are reported before you produce.
  - Subtitles are no longer lost when an episode was reused from the cache.
- **1080i stays interlaced.** Hardware encoders write progressive video only, so 1080i and PsF now always use x264; with AMF a 1080i disc came out as 1080p25.
- **The disc label can be typed** in the disc settings, and it follows the project name when you rename the project.
- **Smaller fixes.**
  - "Every 5 min" on a short video keeps its chapters, and scene detection says when it finds nothing.
  - Burning without a drive, producing with a tool missing, and a menu without buttons are explained before you start.
  - Blu-ray no longer shows a "computed bitrate" alarm on short content, or DVD-9 hints. The average bitrate is the one the episodes really get.
  - The menu lead-in transition is marked as DVD-only.
  - Join, Cover and Inspect are in the Disc menu, where the guide points.
  - Volume labels keep the base letters of accented names ("SERIE_NANDU").
  - Relinked files keep their name.
  - Notices float instead of pushing the page and fade on their own. Opening a dialog closes the one already open, and closing a dialog returns focus to what opened it.
  - Labels wrap instead of being cut off, and the chart labels are readable.
  - The Japanese interface uses a proper font.
  - `kinodisc-cli verify` explains that folders go to `inspect`.
  - Project errors (overlapping buttons, for example) are explained before you produce.
  - Steps that report no progress of their own show a pulsing bar and the elapsed time.
  - The text Size of thumbnail buttons follows its control.
- **From a review of the code before this release:**
  - Split discs no longer keep scene pages of episodes that are on another disc.
  - A finished image is never lost when the earlier one with the same name is open: it is saved as "name (2).iso".
  - Disc folder mode no longer replaces an ISO you made earlier, and clears the old disc files in the folder first.
  - Two queued projects with the same name no longer overwrite each other.
  - Closing the window during a burn stops the burner first.
  - A dialog that is working (queue, join, slideshow, data disc) is not closed by opening another from the menu.
  - A Blu-ray that mixes a soundless video with pass-through DTS or LPCM audio no longer fails.
  - The preview shows a rotated and mirrored video the way the disc does.
- **From the QA of this release:**
  - Deleting an episode or a menu screen waits for your answer: before, it was already gone while the question was on screen, and Cancel did not bring it back.
  - Deleting an episode also removes its menu buttons, instead of leaving a button that played another episode.
  - The Settings link on the main menu takes the current language when the audio and subtitles menu is made again.

## 1.1.2 — 2026-11-18

Fixes reported while recording the promo videos.

### Fixed

- **One default audio track**: a video with two audio tracks flagged as
  default (eng + spa, for example) no longer makes both tracks "Default". The
  first track is the default audio, and only one subtitle track can be the
  default. Older projects are repaired when opened.
- **Photo slideshows keep stream copy**: the slideshow video was full-range
  `yuvj420p` (straight from the JPEGs), so the Videos tab warned "The colour
  format is not yuv420p" and the disc re-encoded it. Slideshows and joined
  videos are now limited-range `yuv420p`.
- **The animated menu preview shows the shape mask**: Rectangle, Rounded
  corners, Circle and Star now change the preview (and the button list
  thumbnails) the way the exported menu draws them.
- **Trimmer messages no longer pile up or move the layout**: an error such as
  "The trim end must be at least 0.5 s after the start" is shown once, in the
  trimmer, and clears with the next action that works. Messages float over
  the page and success messages fade after a few seconds.
- **No alarming bitrate figures on short projects**: when the content is too
  short to fill the disc, the VBR panel shows only that (`W_UNDERFILL`), not
  "411998 kbps exceeds the maximum". The per-episode DVD mux limit is now a
  note (`I_EPISODE_BITRATE_CAP`) instead of a warning, since the episode
  table already shows each episode's bitrate.
- **Disc settings always show Format**: on BD-25/BD-50 the Format field used
  to disappear, so you could not go back to DVD-Video from there. Choosing
  DVD-Video on a Blu-ray capacity now switches to a DVD-5. Changing the
  format no longer jumps to the Menu tab.
- **Paths use one kind of separator**: an output folder typed as `C:/Discs`
  no longer gives "C:/Discs\disc.iso".
- **Decimals follow the app, not Windows**: trim start/end, the thumbnail
  time and the slideshow fields showed "6,0" in English on a Spanish system.
  They show "6.0" and accept either "6.0" or "6,0".

## 1.1.1 — 2026-11-18

A real app icon and a round of polish found while recording the promo video.

### Fixed

- **App icon**: Kinodisc now has its own icon (a disc with a play button)
  instead of a plain square, in every size for Windows, macOS and Linux, and
  in the toolbar, the welcome screen and About. Source:
  `apps/desktop/src-tauri/icons/source/kinodisc.svg`.
- **Template pages follow the interface language** ("Elegant · 2" instead of
  "Elegante · 2").
- **No false navigation notices on templates**: uneven grids (3 + 2) no
  longer show "A manual navigation link prevents perfectly symmetric
  navigation" when there is no manual link. The automatic adjustment is now
  informative (`I_NAV_ROUTE_ADJUSTED`); the notice remains for real manual
  links.
- **Renaming an episode renames its menu buttons** that still had the old
  title; buttons you renamed yourself keep their text.
- **The production result is translated** ("ISO created: … (61.2 MB)" with a
  readable size) in the log and in Job history.
- **Output folder must be a full path**: "C:Discs" or "Discs" used to write
  the disc into whatever folder the app was started from.

## 1.1.0 — 2026-11-17

Fix the video, join clips, chapters at scene changes, a quality test, 15 menu
templates, printable covers and labels, a FreeBSD CLI, and a full walkthrough
of the app with its bugs fixed.

### New

- **Fix the video** (Videos › Picture): rotate/mirror, crop (with black
  border detection), brightness/contrast/saturation, noise reduction,
  deinterlacing and a blurred background for vertical videos. One FFmpeg
  chain is shared by the DVD and Blu-ray encoders and by the preview, so the
  preview matches the disc. Older projects open unchanged.
- **Join videos** into one episode with a chapter per clip (`kinodisc-cli
  join`); different sizes, frame rates and silent clips are fine.
- **Chapters at scene changes** (*By scenes*, `kinodisc-cli chapters
  --scenes`).
- **Test quality**: 10 s encoded exactly as on the disc at the planned
  bitrate, side by side with the original, with PSNR and a plain verdict.
- **15 menu templates**: Wedding, Birthday, Travel, Sports, Holidays, Old
  film, Nature, Documentary, Kids and Elegant join the five from 1.0; each has
  its own background colour and decoration.
- **Case cover and disc label** at 300 dpi (DVD 273×183 mm or Blu-ray
  262×148 mm with spine; 118 mm label), saved as PNG or printed at real size.
- **Notice when done** (sound, taskbar) and **shut down when done** with a
  60-second cancel.
- **Relink moved videos**: missing files are reported on open, marked in
  Media and found again in their new folder.
- **Update check**: at most once a day, one request to GitHub, nothing sent;
  shown in the status bar, can be turned off in Help.
- **CLI menus**: discs rendered from the command line now have the same menu
  artwork as the app (titles, labels, thumbnails, items).
- **FreeBSD**: a prebuilt `kinodisc-cli` in every release; CI runs the whole
  headless test suite in a FreeBSD VM.

### Fixes (walkthrough)

- Interlaced sources failed with FFmpeg 9 (removed `-top`); fixed in 1.0.1
  and now also deinterlaced automatically for progressive Blu-ray discs.
- New projects could not be produced (fixed in 1.0.1).
- The menu cache ignored the composed artwork; a changed thumbnail or font
  could reuse an old menu.
- The space meter emptied after applying a template, undo or disc changes.
- Joining clips whose parameters change midway stalled FFmpeg.
- Minimalist template: dark text on the dark canvas.
- One-episode discs and last pages centre their buttons.
- Production dialog layout, readable overlay labels in the light theme,
  developer wording removed from the preview, "font" vs "source" in
  translations, duplicated error messages.
- "Classic Criterion" is now "Classic Editorial" (trademark).

### Performance

- DVD encoding about **5× faster** at the same quality (light statistics
  pass): 4 videos of 1:50 went from 365 s to 66 s.

## 1.0.1 — 2026-11-17

Fixes found by a full walkthrough of 1.0.0. Upgrading is recommended.

### Fixes

- **New projects can be produced again.** A project created in the app
  failed with "Could not load «background.png»" because its empty starting
  background was read as an image; the menu is now composed on its
  background colour (and the empty background is never packaged).
- **Interlaced videos** (camcorders, TV recordings) failed to encode with
  FFmpeg 9, the version Kinodisc downloads: the removed `-top` option is
  replaced by the `setfield` filter on DVD, AVCHD/Blu-ray and HD menus.
- Templates no longer write their own name ("Retro CRT") as the menu title:
  they use the project name, or the title you typed.
- Vertical phone videos keep their side bars in the trimmer, the episode
  strip and thumbnails, as they appear on the disc.
- A recent project whose file was moved or deleted is marked "Not found"
  with a remove button, instead of a raw system error shown twice.
- Translated errors no longer append the Spanish original.
- The episode thumbnail panel no longer shows an internal file name.

### Performance

- DVD encoding is about **5x faster** for the same quality: the first of
  the two passes only gathers statistics now (it trial-encoded every frame
  several times). A 1:50 project with four videos: 365 s → 66 s.
- Cheap quality options for MPEG-2 (intra VLC table, more motion
  predictors): about +0.35 dB on long discs. Previously encoded episodes are
  re-encoded once.

## 1.0.0 — 2026-11-17

DVD Forge is now **Kinodisc**: a redesigned, professional interface, many
discs at once, scene menus, photo slideshows, a disc inspector, real text on
menus, faster DVD encoding and security hardening.

### New name

- The app, CLI (`kinodisc-cli`), crates (`kinodisc-*`) and repository
  (`github.com/Nicolasalucin/kinodisc`; old links redirect) are now Kinodisc.
- Upgrades keep working: the per-user folder `org.dvdforge.desktop`
  (recent projects, history, render cache, tools) moves once to
  `org.kinodisc.app`, `DVDFORGE_*`/`DVD_FORGE_*` environment variables are
  still read, and `.dvdforge-bundle` packages still open (new ones are
  `.kinodisc`).

### Interface

- Light and dark themes that follow the system (View › Theme to force one),
  system font, no text under 11 px, own icon set, one design system for
  every screen.
- Native menu bar (File, Edit, View, Disc, Help) with recent files, theme,
  language and shortcuts; toolbar; status bar with disc space and tools.
- Undo/redo, unsaved-changes marker and prompt, crash-recovery autosave every
  30 s; Save writes to the current file (Save as asks); drag and drop of
  projects and media onto the window.
- Welcome screen with one-click verified tool setup; a new project opens on
  Videos with "Start by adding your videos"; templates explain they need
  episodes; 16:9 and PAL/NTSC from the system region by default.
- Menu editor: text objects, smart guides, right-click menu (duplicate,
  delete, centre, front/back, align, distribute), templates in a drop-down.
- Properties: font picker with live previews, size stepper, weight toggle,
  colour fields with palette, recents and hex; button preview; clearer
  action, thumbnail and D-pad navigation sections.
- Media as a compact list with type filter and a row menu; sequence frames
  reorder by drag and drop; trimmer filmstrip with dimmed cuts and chapter
  marks; disc settings in one panel; About dialog with credits.

### New features

- **Disc folder** output (VIDEO_TS/BDMV), byte-identical to the ISO, in the
  app and with `kinodisc-cli render --folder`.
- **Several discs**: fill each disc or one disc per episode; template menus
  are regenerated per disc and each root menu shows `N / M`
  (`kinodisc-cli split`).
- **Scene menus** generated from chapters (`kinodisc-cli scenes`).
- **Photo slideshows** with cross-fades and music (`kinodisc-cli slideshow`).
- **Disc inspector** for ISO and VIDEO_TS/BDMV folders, with import of
  unencrypted titles as episodes (`kinodisc-cli inspect`).
- **Blu-ray/AVCHD**: each episode has its own playlist and returns to the
  menu (or continues) when it ends; chapter buttons play inside the episode.

### Fixes

- Menu items (template titles, text, images, shapes) were never drawn in the
  editor nor burned on the disc.
- Native Blu-ray muxer wrote a malformed chapter-mark table (wrong chapters
  on every disc built without tsMuxeR); verifier check `BDMV-MARKS` added.
- Preview: the menu was not visible inside the virtual TV.
- Dialogs without `role="dialog"` let editor shortcuts act while typing.
- Inspector parsers hardened against 32-bit offset overflow (seeded tests).

### Performance

- DVD episodes are encoded in parallel (a quarter of the cores, 1–3 jobs;
  `KINODISC_ENCODE_JOBS`): 4 cold episodes 121 s → 73 s on a 12-core PC.
- Start-up JavaScript 378 kB → 259 kB; first paint ≈ 130 ms.

### Security

- No tool lookup in the current folder; per-file SHA-256 of downloaded tools
  checked before use, planted DLLs refused; absolute System32 `curl`/`tar`;
  image-only IPC reads, folder-only "open folder"; CI actions pinned to
  commit SHAs with least privilege; `SHA256SUMS.txt` in releases. Details in
  `docs/security/2026-11-review.md`.

### Credits

Thanks to OhItsStefan, Infinite_Silver483, Narrow-Chard4152,
Mirandopacuenca67 and Carllito7 for finding bugs and suggesting improvements.

## 0.9.0 — 2026-11-17

DVD-Video now works on Windows with one-click tools, a disc queue in the
app, player presets, a much faster start-up, keyboard accessibility and
robustness tests.

### DVD-Video without external ISO tools

- New native **UDF 1.02 + ISO 9660 bridge** writer for DVD-Video
  (`dvdforge-iso::write_dvd_video_iso`): canonical contiguous layout the
  IFO sector offsets rely on, streamed payload, cancellable, deterministic
  with `SOURCE_DATE_EPOCH`. `genisoimage`/`mkisofs`/xorriso are no longer
  needed (`DVDFORGE_DVD_ISO_BUILDER=external` keeps the old path).
  Verified with `dvdforge-verify --deep`, Windows mounting (byte-identical
  VOBs) and libmkiso's independent UDF and ISO 9660 readers.
- **DVDAuthor/Spumux for Windows with one click**: built from the official
  0.7.2 sources by a new CI workflow and published as the
  `tools-dvdauthor-0.7.2` pre-release; **Health → Install** downloads it,
  checks size and SHA-256 and installs it with its DLLs. A full DVD-Video
  renders on Windows with only one-click-installed tools.

### Disc queue in the app

- **📋 Queue**: add the current or any saved projects, reorder, retry and
  **Produce all**; each project is rendered next to its file. The queue
  survives restarts and can be stopped.

### Player presets

- **Player** selector: PlayStation 2, PlayStation 3/4/5, living-room
  Blu-ray player, older DVD player. Tightens bitrate/level limits and adds
  `W_PLAYER_PRESET_STRICT` / `E_PLAYER_PRESET_CONFLICT` notes to the plan.

### Faster start-up and accessibility

- Languages other than Spanish/English and every secondary screen (menu
  canvas, editing, tracks, simulator, VM map, motion, VBR, production) load
  on demand: start-up JavaScript **1072 kB → 376 kB**.
- Visible keyboard focus, `prefers-reduced-motion`, Escape closes dialogs,
  and the menu canvas can be edited with the keyboard (arrows, Shift,
  `[ ]`, Delete, Esc).
- Fix: dialogs opened from the header (Health, Queue, History) were clipped
  and drawn under the canvas.

### Robustness

- Seeded fuzz-style tests in the normal test suite: corrupted DVD images
  for the verifier, HDMV PMT/MovieObject/command parsers, malformed
  SRT/ASS/VTT and corrupted bundles. They found and fixed a verifier crash
  on truncated images.
- CI now fails if any of the 6 languages or any diagnostic code is missing
  a translation (`npm run i18n:check`).

## 0.8.0 — 2026-11-17

Move projects between computers, render many discs in one go, resume
interrupted renders and install FFmpeg with one click.

### Project bundles

- New **📦 Bundle** button and `dvdforge-cli bundle export|import|info`: the
  project plus every video, external audio, subtitle and font file in one
  `.dvdforge-bundle` (deterministic tar + zstd, manifest with size and
  SHA-256 of every file, all paths rewritten to `assets/…`).
- **Open** accepts bundles and unpacks them into a chosen folder. Import
  verifies every file while streaming, rejects path traversal, refuses
  non-empty folders and removes everything if a check fails.
- New codes `E_BUNDLE_CORRUPT`, `E_BUNDLE_SCHEMA`, `E_BUNDLE_DESTINATION`,
  `E_BUNDLE_IO`, `W_BUNDLE_EXTERNAL_ASSET`.

### Batch render queue

- `dvdforge-cli batch <queue.json> [--json] [--resume] [--fail-fast]` renders
  several projects one after another, saves progress to
  `<queue>.state.json` after every item and resumes skipping finished
  items. New codes `E_BATCH_QUEUE`, `E_BATCH_ITEM`, `W_BATCH_PARTIAL`.

### Resume interrupted renders

- Episodes finished by an earlier (interrupted or failed) render are reused
  from the Smart Cache; the log now says which ones and how many, the CLI
  prints these notices, and the production dialog explains that producing
  again does not start over. Verified by killing a render in the middle of
  an episode.

### One-click FFmpeg

- **Health → Install automatically** and `dvdforge-cli toolchain install
  ffmpeg` download the pinned FFmpeg 9.0.2 essentials build (Windows x86_64),
  check its size and SHA-256 before extracting, extract only `ffmpeg.exe` and
  `ffprobe.exe` into the per-user folder (searched before PATH) and record
  their provenance. Other systems get the package-manager command.
- New codes `E_TOOLCHAIN_UNAVAILABLE`, `E_TOOLCHAIN_DOWNLOAD`,
  `E_TOOLCHAIN_CHECKSUM`, `E_TOOLCHAIN_ARCHIVE`, `E_TOOLCHAIN_IO`.

### Engineering

- All new texts and codes are translated in the 6 interface languages
  (`npm run i18n:check` clean). 302 workspace tests pass; clippy clean.
- Known limitation: DVD-Video ISOs still need `genisoimage`/`mkisofs`
  (xorriso) installed; Blu-ray/AVCHD use the built-in UDF writer.

## 0.7.0 — 2026-11-16

Community feedback release: frame rates, menus, localization, audio, direct
import and a guided, family-friendly interface.

### Blu-ray/AVCHD frame rates

- 1080p now accepts **25 and 29.97 fps**, encoded as **PsF** (x264
  fake-interlaced, signalled as 1080i) so every Blu-ray player accepts it, and
  **50/59.94 fps** (AVCHD 2.0 Progressive, H.264 Level 4.2) with the new
  `W_AVCHD_1080P_HFR` compatibility warning.
- The encoder honours the configured H.264 level instead of a fixed 4.1.
- **Fix:** the native BDMV muxer wrote a hardcoded 480p/23.976 video
  attribute in every MPLS/CLPI; it now writes the project's real format/rate.

### Menus

- **Fix:** with FFmpeg 9 and no tsMuxeR (the usual Windows setup) every HDMV
  menu failed to attach (`first pts and dts value must be set`) and the disc
  was silently produced without menus. The native path now muxes the menu
  video from Matroska with real timestamps.
- HD menus are edited on a **1920×1080 / 1280×720 canvas** that matches the
  disc, instead of a 720×480 DVD canvas that squeezed HD backgrounds. Older
  projects are upgraded on open; layouts, fonts and templates scale.
- New **+ Menu screen** action (submenu with Back button, linked from the
  current menu); rename by double click, delete from the menu list. The
  misleading "+ Add" button next to the menus is now "+ Episode".
- Button fonts: any font installed on the computer (searchable picker).
- Thumbnails: menu tools (thumbnails, layouts, button styles) are no longer
  disabled on Blu-ray/AVCHD; one-click "use the episode image" and "pick the
  exact frame" from the Inspector; labelled 📸 Thumbnail button in the trimmer.

### Smart Stream Copy (roadmap v0.7.0)

- `Episode.videoMode` (auto/copy/reencode, default auto). Compliant H.264
  sources (High/Main, allowed level, yuv420p, exact resolution, frame rate —
  including field-rate camcorder 1080i —, scan and bitrate, no start trim) are
  put on AVCHD/Blu-ray discs **without re-encoding**.
- The planner budgets copied episodes at their real bitrate and shares the
  remaining space among re-encoded ones; cache keys include the decision.
- FFprobe import records video profile, level and bitrate. New
  `W_STREAM_COPY_FALLBACK`; the Inspector shows ⚡ copied / ↻ re-encoded and why.
- DVD-Video stream copy is not included yet.

### Audio

- Tracks pointing at a stream the source file does not have are remapped
  (`W_AUDIO_STREAM_REMAPPED`) or replaced by silence (`W_AUDIO_SOURCE_SILENT`)
  instead of failing FFmpeg.
- AVCHD/Blu-ray episodes with different audio languages/track counts are
  adapted to the first episode (`W_AUDIO_LAYOUT_ADAPTED`) instead of aborting.

### Localization

- The English interface is complete; remaining hardcoded Spanish strings are
  translated. Every Rust diagnostic code (139) has an English meaning, so
  warnings, logs and simulator notices follow the interface language.
- Default disc content names ("Main menu", "Next ›") follow the interface
  language. `npm run i18n:check` reports missing keys and codes.

### Family-friendly interface

- Step-by-step **❓ Guide** that opens on first run and jumps to each screen.
- **Simple mode** by default; Animation, VM map and VBR are behind
  **Expert mode**. Plain-language welcome screen and new-project dialog.
- New user guides: `docs/GUIA-DE-USO.md` (Spanish) and `docs/USER_GUIDE.md`.

### Engineering

- `ENCODER_REV` 7. Stream-copy and PsF/HFR unit tests, audio reconciliation
  tests, a stream-copy smoke render, and backend-agnostic BD smoke tests.
- Clippy (Rust 1.100) clean; `cargo test --workspace` green.

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
