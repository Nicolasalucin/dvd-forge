# Kinodisc user guide

Kinodisc turns your videos into a DVD, AVCHD or Blu-ray with menus that work
on a living-room player. This guide follows the steps in order. Press **F1**
at any time for the guided tour.

## 1. Before you start

- **Windows:** on the welcome screen press **Download and set up**. Kinodisc
  downloads FFmpeg and DVDAuthor once (about 120 MB), checks their SHA-256 and
  keeps them in your user folder.
- **Linux, macOS, FreeBSD:** install `ffmpeg` and `dvdauthor` with your package
  manager (see the README).
- The status bar (bottom right) says **Tools ready** or **Tools missing**;
  click it to see each program and install what is missing.

The theme follows your system; **View › Theme** forces light or dark.
**View › Language** changes the interface language. Once a day Kinodisc asks
GitHub whether a newer version exists and says so in the status bar; turn it
off in **Help › Check for updates at start-up**.

## 2. Create a project

**File › New project** (Ctrl+N) or the **New project** card:

- **DVD-Video universal** plays almost everywhere. TV system (PAL/NTSC) is
  chosen from your region; the screen is 16:9 unless you pick 4:3.
- **Blu-ray / AVCHD (high definition)** for Blu-ray players and PlayStation:
  - 1080p 23.976/24 for films, 25/29.97 for home video (recorded as PsF, plays
    on every Blu-ray player), 50/59.94 for very smooth motion (AVCHD 2.0; some
    old players do not support it).

After creating it, the **disc button** at the right of the tabs (for example
`DVD-5 · PAL · 16:9`) opens the disc settings: format, capacity (DVD-5/9,
BD-25/50), TV system, screen, region, target player and **Disc label** (the
name a computer shows when the disc is inserted: letters without accents,
digits and `_`). The label follows the project name when you rename the
project.

## 3. Add videos

**Import videos…** in the empty project, **Import** in the toolbar (Ctrl+I),
or drag the files onto the window. Each video becomes an episode
automatically in the **Episodes** strip at the bottom; drag the frames to
change the order. Subtitle files saved next to a video (for example
`name.es.srt`) are added to it with their language. Picking the same file
again does not add a duplicate.

In **Media** (left) each file has **Add**; the **⋯** button offers the other
uses (button thumbnail, menu background, animated background, menu music).

### Trim and chapters

In **Videos**, the strip under the player shows frames of the whole video:
drag the blue handles or use **Mark start / Mark end**. Chapters appear as
amber marks; add them at the playhead or with *Every 5 min*, *Every 10 min*,
*4 parts* or **By scenes** (a chapter at every scene change), or import them
from the file.

### Fix the picture

The **Picture** card under the player fixes a video before it goes on the
disc; the preview shows the frame at the playhead as the disc will:

- **Orientation**: rotate left/right and mirror (phone videos filmed sideways).
- **Crop edges**: type the pixels or press **Detect black borders**.
- **Brightness, contrast, saturation** (double-click a slider to reset it).
- **Reduce noise** (light/strong) and **Deinterlace** (automatic when an
  interlaced camcorder video goes to a progressive Blu-ray).
- **Sides of a vertical video**: black bars or a **blurred background** made
  from the same video.
- **Test quality** encodes 10 seconds exactly as on the disc and shows them
  next to the original, with a verdict.

### Join videos

**Disc › Join videos…** (or the button in an empty project) turns several
clips into one episode, in the order you choose, with a chapter at the start
of each clip. Different sizes, frame rates and clips without sound are fine.

### Videos that do not need re-encoding (Blu-ray/AVCHD)

When a video already matches the disc (camcorder `.m2ts`, or H.264 with the
project's resolution and frame rate), Kinodisc **copies it without
re-encoding**: much faster and lossless. The **Video** field of the episode
properties says whether it is copied or re-encoded, and why. The episode must
not be trimmed at the start to be copied.

### Photo slideshows

**Photos** in the toolbar: add pictures, order them, choose seconds per photo,
the cross-fade and optional music. The slideshow is created as a video and
added as an episode.

### Import an existing disc

**Disc** in the toolbar (or **Disc › Inspect disc…**) opens the inspector:
open an ISO or a VIDEO_TS/BDMV
folder to see its titles, chapters and tracks, and **Import as episodes** if it
is not encrypted. Encrypted (commercial) discs can be viewed but not imported.

## 4. Design the menu

In **Menu**:

- **Templates** applies one of 15 designs (wedding, birthday, travel,
  sports, holidays, old film, nature, documentary, kids, elegant…) with one
  button per episode; more pages are added automatically. Each template has
  its own background colour and decoration; choosing your own background
  picture replaces the colour. **Scene menus** inside the templates panel creates a
  menu with one button per chapter.
- **Button** adds a button, **Text** adds a title or any text.
- Drag to move: pink guides appear when a button lines up with another or with
  the centre. Right-click a button to duplicate, delete, centre, bring to front,
  align or distribute all buttons.
- Properties (right): text with **font picker**, size, weight and colours;
  **When pressed** (play episode, play all, go to a menu, a chapter, an audio
  track…); thumbnail; and **Remote navigation** (which button each arrow
  reaches).
- **Background** and **Music** in the menu toolbar; *Normal / Highlight /
  Selection* show how buttons look when the remote moves over them.
- **Auto-play** in the menu toolbar (main menu of a DVD-Video): when nobody
  chooses anything by the time the menu ends, the disc starts playing
  everything. Without it the menu waits. On Blu-ray and AVCHD the menu always
  waits until you choose.

## 5. Audio and subtitles

In **Audio and subtitles** choose each track's language, format and bitrate,
mark the default track and fix delays (±10 s). Add subtitles from `.srt`,
`.ass`, `.vtt` or `.sup` files; their style is set per track.

**Even volume across the disc** (one checkbox) levels the volume of every
episode, so you don't reach for the remote between one video and the next.

The disc can also carry an **audio and subtitles menu**: a generated Settings
page where the viewer picks the audio language and the subtitles with the
remote. Create it in the **Menu** tab with **Templates › Audio and
subtitles**; it needs a video with more than one audio language, or with
subtitles. If you change the tracks, press it again to refresh the page.

## 6. Try the disc

**Preview** shows the menu on a TV with a virtual remote: arrows, OK, Menu,
Title and Back behave as on a real player, including Play All and chapters.

## 7. Produce and burn

**Produce disc** (Ctrl+R) offers:

- **ISO only**, **Disc folder** (VIDEO_TS/BDMV for players, USB drives or
  Kodi), **ISO + burn**, or **Existing ISO**.
- **Discs:** a single disc, several discs (each filled up) or one disc per
  episode. Split discs show `1 / 3`, `2 / 3`… on their menu.
- **Copies** (when burning): choose how many copies you want. After each one
  Kinodisc asks for the next blank disc; insert it and press **Continue**.

The output folder ends up with only the image, named after the project
(`Boda.iso`, `Serie - Disc 2.iso`). The work files go to a temporary folder
that is removed when the production ends, fails or is cancelled. Kinodisc
remembers the output folder for the next time.

Every image is checked by the structural verifier before it can be burned.

**Shut down the computer when done** turns the computer off after the job
(a 60-second countdown lets you cancel). Either way, Kinodisc plays a sound
and flashes in the taskbar when it finishes.

### Cover and label

**Disc › Cover and label…** makes the case cover (DVD 273×183 mm, Blu-ray
262×148 mm, with spine) and the round disc label (118 mm, blank centre) with
your menu's colours, font and thumbnails. Edit the texts, then **Save PNG**
(300 dpi; print at 100 %) or **Print**.

### Data disc

**Disc › Data disc…** burns any folder (photos, documents, the original
files) to a CD, DVD or Blu-ray that opens on a computer. It is not a video
disc for the TV. Choose the folder: Kinodisc shows how many files it has, its
size and which disc it fits. Type a disc label, choose the output folder and
press **Create disc image**; then **Burn to a disc…** burns the image.

### Erase a rewritable disc

**Disc › Erase rewritable disc…** makes a DVD-RW, DVD+RW or BD-RE ready to
burn again. **Quick** takes a few minutes and is enough to burn the disc
again; **Full** rewrites the whole disc and is for discs that give errors.
DVD-R and DVD+R discs cannot be erased. Do not remove the disc while it is
being erased.

### When burning fails

The dialog says why in plain words (no disc, the disc is upside down, it is
not empty, the speed is too high, the cable…) and what to try. When a
rewritable disc is not empty it offers **Erase disc…**. **Copy details**
copies a short technical report you can paste when asking for help; Kinodisc
never sends it anywhere.

### If production stops halfway

Press **Produce disc** again: episodes that were already encoded are reused.

### Several projects in a row

**Disc › Disc queue…**: add saved projects and press **Produce all**; waiting
projects stay in the queue if the app is closed.

### Move or share a project

**File › Package with the videos…** saves the project and everything it uses
in one `.kinodisc` file. Open it on the other computer and choose an empty
folder; every file is checked (SHA-256).

### Unsaved changes

Undo/redo with Ctrl+Z / Ctrl+Y. A dot next to the project name means unsaved
changes; Kinodisc asks before closing them, and keeps a recovery copy every
30 seconds in case the computer shuts down.

## 8. Keyboard

| Keys | Action |
|---|---|
| Ctrl+N / Ctrl+O / Ctrl+S | New, open, save (Ctrl+Shift+S: save as) |
| Ctrl+Z / Ctrl+Y | Undo / redo |
| Ctrl+I | Import media |
| Ctrl+D | Duplicate the selected button |
| Ctrl+R | Produce disc |
| Arrows (Shift = faster) | Move the selected button or text |
| `[` `]` | Previous / next button |
| Delete / Esc | Delete / deselect |
| F1 / F11 | Guide / full screen |

## 9. Common problems

| Problem | Solution |
|---|---|
| “Tools missing” | Click it in the status bar and install; on Linux/macOS run the command shown. |
| After updating, tools are reinstalled | Kinodisc 1.0 checks every tool file; copies from older versions are downloaded again once. |
| “Does not fit” | Lower the quality, choose a bigger disc or produce several discs. |
| “N file(s) of this project are missing” | In Media, press **Find…** on a file marked *Not found*; other missing files in the same new folder are found too. |
| A button cannot be moved there | Buttons need 4 px between them (a DVD/Blu-ray rule). |
| Audio warning | The production log says which track was adjusted and why. |

## 10. Advanced tools

**View › Advanced tools** shows Animation (moving menus and thumbnails), VM map
(every jump the player can make) and VBR quality (bitrate per episode).
