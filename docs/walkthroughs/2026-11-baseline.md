# Baseline walkthrough — 0.9.0 (before the Kinodisc 1.0 rework)

Method: `app-walkthrough` skill, built app driven through WebView2 DevTools,
screenshots read after each step (kept locally in `img/`, not committed:
they show local paths). Personas: Abuela Rosa (es, first run), Mark (DVDStyler
veteran, en), Lucía (TV archivist). Items marked *code* were confirmed by
reading the code instead of clicking (native dialogs cannot be scripted).

**Verdict: Completed with friction** for Mark, **Blocked** for Rosa (cannot
tell what to do first), **Blocked** for Lucía (no multi-disc).

## Findings

| # | Sev | Finding | Evidence | Where |
|---|---|---|---|---|
| 1 | Blocker | First run shows the new-project dialog **and** the guide at the same time; after creating the project the app opens on the menu editor with no videos — the first thing to do (add videos) is a small "+ Importar" in a side panel. | b01, b02 | `App.svelte`, `NewProjectModal.svelte`, `GuideTour.svelte` |
| 2 | Major | Overload: 50 visible buttons and 49 text labels ≤ 10 px on the empty studio; duplicated controls (NTSC/4:3 badges + selects; "+ Botón" and "Añadir botón"). | b02, DOM count | `StudioHeader.svelte`, `MenuWorkspace.svelte`, `Inspector.svelte` |
| 3 | Major | Jargon for normal users: "VMGM · Raíz", "Guías nav", "Área segura", "Resaltado/Selección", "Región libre", "Modo experto". | b02 | several |
| 4 | Major | A fake `background.png` placeholder appears in the media library of a new project. | b02 | `commands/project.rs` (placeholder asset) |
| 5 | Major | Defaults: NTSC for a Spanish speaker (Spain/Argentina are PAL); 4:3 although phone video is 16:9. | b01 | `NewProjectModal.svelte` |
| 6 | Major | Templates gallery is shown (and clickable) before any episode exists; applying fails. | b02 | `TemplateGallery.svelte` |
| 7 | Major | "Producir disco" is enabled with 0 episodes; the dialog then disables "Generar ISO" without saying why. | b03 | `StudioHeader.svelte`, `ProductionModal.svelte` |
| 8 | Major | No undo/redo (Ctrl+Z does nothing). | DOM test | store |
| 9 | Major | Menu text/image/shape items are never drawn nor exported: template titles vanish. | *code* | `editor/*`, `exportMenu.ts` |
| 10 | Major | No drag & drop of files onto the window. | *code* | — |
| 11 | Major | No autosave / crash recovery (README claims autosave). | *code* | — |
| 12 | Minor | Visual style: glow, cyan gradients, emoji icons, dark only — reads as generic. | b01–b03 | `app.css`, components |
| 13 | Minor | Canvas is small inside a crowded centre column. | b02 | `MenuWorkspace.svelte` |

## Missing-feature checklist

| Feature | Status |
|---|---|
| VIDEO_TS / BDMV folder output | Partial (CLI only) |
| Multi-disc split into a folder of ISOs | Missing (planner only suggests) |
| One ISO per episode | Missing |
| Automatic chapter menus | Missing (`MenuRole::ChapterSelect` unused by templates) |
| Photo slideshow | Missing |
| Title per file / post-action | Have |
| Text objects, shapes, align guides | Missing (data model only) |
| Undo/redo, copy/paste, multi-select | Missing |
| Real motion preview | Partial |
| Fit-to-disc meter | Have |
| Drag & drop files | Missing |
| Recent projects / autosave / recovery | Partial (recent only) |
| Burn speed / verify / eject / burn existing ISO | Have |
| Templates with real thumbnails | Partial (colour swatches only) |
| Subtitles / multiple audio | Have |
| Plain-language help and first-run guide | Partial (guide exists, jargon remains) |
| About / credits | Missing |

## Keep

Production dialog flow (ISO only / ISO + burn / existing ISO), capacity bar,
simulator with remote, one-click tool install, guide content.
