# Personas, tasks and missing-feature checklist

## Personas

| Persona | Who | Language | Patience | Typical source files |
|---|---|---|---|---|
| **Abuela Rosa** | First computer DVD; wants the grandchildren's videos on a disc for the TV | es | Low: gives up after 2 dead ends | Phone MP4 (portrait too), WhatsApp clips |
| **Mark, the DVDStyler veteran** | Has made DVD menus for 10 years; expects DVDStyler/DVD Flick muscle memory | en | Medium, but judges every missing feature | MKV/AVI episodes, SRT subtitles |
| **Lucía, TV archivist** | Burns a whole season (24 episodes) to several discs | es/en | High | MKV with 2 audio tracks + subs |
| **Kenji, camcorder owner** | AVCHD camcorder `.MTS` files to Blu-ray without re-encoding | ja/en | Medium | `.MTS`/`.M2TS` 1080i/1080p50 |
| **Dana, accessibility** | Keyboard only, large text, screen reader | en | Medium | Any |

## Core tasks (every pass)

1. First run with no project: understand what the app does and create a project.
2. Import 3 videos, put them in order, trim the start of one.
3. Get a menu with one button per episode and a thumbnail on each.
4. Add a background picture and music to the menu.
5. Add a second menu screen (chapters or extras) reachable from the first.
6. Test the disc with the remote simulator.
7. Produce: ISO only; then ISO + burn (stop before the burn).
8. Close and reopen the project; everything is still there.
9. Find out why something failed (missing tool, disc too full) and fix it
   without reading the docs.

## Persona-specific tasks

- Mark: change a button's action to "play chapter 3"; make Play All; set
  First Play to an intro clip; make a chapter menu automatically.
- Lucía: 24 episodes that do not fit one DVD → split across discs and get
  **a folder with Disc 1.iso, Disc 2.iso…**, each with its own menu.
- Kenji: add `.MTS` files and confirm they are copied without re-encoding.
- Dana: complete tasks 1–7 using only the keyboard.

## Missing-feature checklist (compare with DVDStyler, DVD Flick, ConvertXtoDVD, Encore)

Mark each Have / Partial / Missing with evidence.

- Output **VIDEO_TS / BDMV folder** (not only ISO) for playing from a USB or
  copying to a media player.
- **Multi-disc split** of long projects into a folder of ISOs (Disc 1…N),
  with per-disc menus and a "continued on disc 2" note.
- **One ISO per episode/chapter** option (batch: each title as its own disc).
- **Automatic chapter menus** (thumbnail grid of chapters, paginated).
- **Photo slideshow** titles (images + music + transitions), common in
  DVDStyler and consumer tools.
- **Title per file vs. one long title** choice; per-title post-action
  (return to menu / next / loop).
- Menu **text objects, shapes, frames, drag-to-align guides**, undo/redo,
  copy/paste of buttons, multi-select.
- **Menu preview with real playback** of motion menus before encoding.
- **Fit to disc** quality slider with a clear "fits / does not fit" meter.
- **Drag and drop** files from the file manager onto the app.
- **Recent projects**, autosave and crash recovery.
- **Burn speed / verify / eject** options; burn an existing ISO or folder.
- **Templates gallery** that looks good with real thumbnails.
- **Subtitles**: import, style, burn-in option, forced subtitles.
- **Audio**: multiple languages, normalise volume, replace audio.
- **Help**: tooltips that explain DVD jargon in plain words; first-run guide.
