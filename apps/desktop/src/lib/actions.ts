/**
 * User actions shared by the toolbar, the native menu bar and keyboard
 * shortcuts, so each one behaves the same wherever it is started.
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import { commands } from "./api/bindings";
import { messageOf, unwrap } from "./api/client";
import { localizeDiagnostic, localizeError } from "./i18n/diagnostics";
import { t } from "./i18n/index.svelte";
import { projectStore } from "./stores/project.svelte";
import { ui } from "./ui/state.svelte";
import { nativePath } from "./utils/format";

function fail(cause: unknown): void {
  projectStore.error = localizeError(messageOf(cause));
}

function fileStem(name: string): string {
  return name.trim().replaceAll(" ", "-").toLowerCase() || "proyecto";
}

export async function newProject(): Promise<void> {
  if (!(await projectStore.confirmLoseChanges())) return;
  ui.newProject = true;
}

export async function openProject(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [
      { name: t("Proyecto o paquete de Kinodisc"), extensions: ["json", "kinodisc", "dvdforge-bundle"] },
    ],
  });
  if (typeof picked !== "string") return;
  await openPath(picked);
}

export async function openPath(picked: string): Promise<void> {
  try {
    if (/\.(kinodisc|dvdforge-bundle)$/i.test(picked)) {
      if (!(await projectStore.confirmLoseChanges())) return;
      // A bundle is unpacked into a folder chosen by the user, then opened.
      const folder = await open({
        directory: true,
        title: t("Elige una carpeta vacía donde descomprimir el paquete"),
      });
      if (typeof folder !== "string") return;
      const base = picked.replace(/^.*[\\/]/, "").replace(/\.(kinodisc|dvdforge-bundle)$/i, "");
      // One separator style for the whole path (it is shown and kept in the
      // recent projects).
      const destination = nativePath(`${folder.replace(/[\\/]+$/, "")}/${base}`);
      const projectPath = await unwrap(commands.bundleImport(picked, destination));
      await projectStore.openProject(projectPath);
      ui.notice = t("Paquete descomprimido en {folder}.", { folder: destination });
      return;
    }
    await projectStore.openProject(picked);
  } catch (cause) {
    fail(cause);
  }
}

/** Saves to the current file; asks for a name the first time. */
export async function saveProject(): Promise<void> {
  if (!projectStore.project) return;
  if (!projectStore.projectPath) return saveProjectAs();
  try {
    await projectStore.saveProject(projectStore.projectPath);
    ui.notice = t("Proyecto guardado.");
  } catch (cause) {
    fail(cause);
  }
}

export async function saveProjectAs(): Promise<void> {
  const current = projectStore.project;
  if (!current) return;
  const path = await save({
    defaultPath: `${fileStem(current.name)}.json`,
    filters: [{ name: t("Proyecto de Kinodisc"), extensions: ["json"] }],
  });
  if (!path) return;
  try {
    await projectStore.saveProject(path);
    ui.notice = t("Proyecto guardado.");
  } catch (cause) {
    fail(cause);
  }
}

export async function exportBundle(): Promise<void> {
  const current = projectStore.project;
  if (!current) return;
  const path = await save({
    defaultPath: `${fileStem(current.name)}.kinodisc`,
    filters: [{ name: t("Paquete de Kinodisc"), extensions: ["kinodisc"] }],
  });
  if (!path) return;
  ui.notice = t("Empaquetando el proyecto y sus vídeos…");
  try {
    const report = await unwrap(commands.bundleExport(current, path));
    const megabytes = (report.manifest.files.reduce((sum, file) => sum + file.bytes, 0) / 1_000_000).toFixed(1);
    ui.notice = t("Paquete creado: {count} archivos, {size} MB.", {
      count: report.manifest.files.length,
      size: megabytes,
    });
    if (report.warnings.length > 0) {
      projectStore.error = report.warnings.map((warning) => localizeDiagnostic(warning)).join(" ");
    }
  } catch (cause) {
    ui.notice = null;
    fail(cause);
  }
}

export const MEDIA_EXTENSIONS = [
  "mp4", "m4v", "mkv", "mov", "avi", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "vob",
  "mp3", "wav", "flac", "ogg", "ac3", "m4a", "aac",
  "png", "jpg", "jpeg", "webp",
  "srt", "ass", "ssa", "vtt", "sup",
];

export async function importMedia(paths?: string[]): Promise<void> {
  if (!projectStore.project) return;
  let list = paths;
  if (!list) {
    const selected = await open({
      multiple: true,
      filters: [
        { name: t("Vídeo, audio e imagen"), extensions: MEDIA_EXTENSIONS },
        { name: t("Todos los archivos"), extensions: ["*"] },
      ],
    });
    if (!selected) return;
    list = Array.isArray(selected) ? selected : [selected];
  }
  const before = new Set(projectStore.project.assets.map((asset) => asset.id));
  const count = await projectStore.importMedia(list);
  // Each imported video becomes an episode, as the guide and the empty
  // project promise; images and audio stay in the library for menus.
  const project = projectStore.project;
  let episodes = 0;
  for (const asset of project?.assets ?? []) {
    if (before.has(asset.id) || asset.kind !== "video") continue;
    if (project?.episodes.some((episode) => episode.video === asset.id)) continue;
    if (projectStore.addEpisodeFromAsset(asset.id)) episodes += 1;
  }
  if (episodes > 0) {
    ui.notice = t(episodes === 1 ? "{count} vídeo añadido como episodio." : "{count} vídeo(s) añadidos como episodios.", { count: episodes });
  } else if (count > 0) {
    ui.notice = t("{count} archivo(s) importados.", { count });
    projectStore.mediaTab = "media";
  }
}

export async function toggleFullscreen(): Promise<void> {
  const window = getCurrentWindow();
  try {
    const isFull = await window.isFullscreen();
    await window.setFullscreen(!isFull);
    ui.fullscreen = !isFull;
  } catch (cause) {
    fail(cause);
  }
}

export function produce(): void {
  if (projectStore.project) projectStore.productionOpen = true;
}

export function undo(): void {
  projectStore.undo();
}

export function redo(): void {
  projectStore.redo();
}
