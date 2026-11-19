/**
 * Native menu bar (File, Edit, View, Disc, Help), built from the frontend so
 * its labels come from the same translations as the rest of the interface.
 * It is rebuilt when the language, theme, expert mode or recent list change.
 *
 * Shortcuts: on Windows/Linux the shortcut is shown in the label (the Win32
 * "\t" convention) and handled by the app's own keydown listener, so it
 * never fires twice; on macOS the menu registers real accelerators.
 */
import { setUpdateChecks, updateChecksEnabled } from "./updates";
import { CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import * as actions from "../actions";
import { i18n, LOCALES, t, type Locale } from "../i18n/index.svelte";
import { projectStore } from "../stores/project.svelte";
import { ui, type ThemePreference } from "./state.svelte";

export const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

const LANGUAGE_NAMES: Record<string, string> = {
  es: "Español",
  en: "English",
  pt: "Português",
  fr: "Français",
  de: "Deutsch",
  it: "Italiano",
  ja: "日本語",
};

type Shortcut = { key: string; label: string };

function shortcut(key: string): Shortcut {
  // key like "Ctrl+Shift+S"; macOS shows ⌘.
  return { key, label: isMac ? key.replace("Ctrl", "Cmd") : key };
}

async function item(id: string, text: string, run: () => unknown, keys?: Shortcut, enabled = true) {
  return MenuItem.new({
    id,
    text: keys && !isMac ? `${text}\t${keys.label}` : text,
    accelerator: keys && isMac ? keys.key.replace("Ctrl", "CmdOrCtrl") : undefined,
    enabled,
    action: () => void run(),
  });
}

async function separator() {
  return PredefinedMenuItem.new({ item: "Separator" });
}

let building: Promise<void> | null = null;
let again = false;

/** Builds and installs the menu bar; concurrent calls collapse into one. */
export function refreshAppMenu(): void {
  if (building) {
    again = true;
    return;
  }
  building = build()
    .catch(() => undefined)
    .finally(() => {
      building = null;
      if (again) {
        again = false;
        refreshAppMenu();
      }
    });
}

async function build(): Promise<void> {
  const hasProject = Boolean(projectStore.project);
  const recent = projectStore.recentProjects.slice(0, 8);
  const recentItems = recent.length
    ? await Promise.all(
        recent.map((entry, index) =>
          item(`recent-${index}`, entry.name || entry.filePath, () => actions.openPath(entry.filePath)),
        ),
      )
    : [await item("recent-none", t("Sin proyectos recientes"), () => undefined, undefined, false)];

  const file = await Submenu.new({
    text: t("Archivo"),
    items: [
      await item("new", t("Nuevo proyecto…"), actions.newProject, shortcut("Ctrl+N")),
      await item("open", t("Abrir…"), actions.openProject, shortcut("Ctrl+O")),
      await Submenu.new({ text: t("Abrir reciente"), items: recentItems }),
      await separator(),
      await item("save", t("Guardar"), actions.saveProject, shortcut("Ctrl+S"), hasProject),
      await item("save-as", t("Guardar como…"), actions.saveProjectAs, shortcut("Ctrl+Shift+S"), hasProject),
      await item("bundle", t("Empaquetar con los vídeos…"), actions.exportBundle, undefined, hasProject),
      await separator(),
      await item("import", t("Importar medios…"), () => actions.importMedia(), shortcut("Ctrl+I"), hasProject),
      await item("slideshow", t("Presentación de fotos…"), () => (ui.slideshow = true), undefined, hasProject),
      await separator(),
      await item("quit", t("Salir"), () => getCurrentWindow().close(), shortcut("Ctrl+Q")),
    ],
  });

  const edit = await Submenu.new({
    text: t("Edición"),
    items: [
      await item("undo", t("Deshacer"), actions.undo, shortcut("Ctrl+Z"), projectStore.canUndo),
      await item("redo", t("Rehacer"), actions.redo, shortcut("Ctrl+Y"), projectStore.canRedo),
      await separator(),
      await item("duplicate", t("Duplicar botón"), () => projectStore.duplicateSelectedButton(), shortcut("Ctrl+D"), hasProject),
      await item("delete", t("Eliminar botón"), () => projectStore.deleteSelectedButton(), undefined, hasProject),
    ],
  });

  const themeItem = async (id: ThemePreference, text: string) =>
    CheckMenuItem.new({ id: `theme-${id}`, text, checked: ui.theme === id, action: () => ui.setTheme(id) });
  const view = await Submenu.new({
    text: t("Ver"),
    items: [
      await Submenu.new({
        text: t("Tema"),
        items: [
          await themeItem("system", t("Como el sistema")),
          await themeItem("light", t("Claro")),
          await themeItem("dark", t("Oscuro")),
        ],
      }),
      await Submenu.new({
        text: t("Idioma"),
        items: await Promise.all(
          LOCALES.map((locale) =>
            CheckMenuItem.new({
              id: `lang-${locale.id}`,
              text: LANGUAGE_NAMES[locale.id] ?? locale.label,
              checked: i18n.locale === locale.id,
              action: () => void i18n.setLocale(locale.id as Locale),
            }),
          ),
        ),
      }),
      await CheckMenuItem.new({
        id: "expert",
        text: t("Herramientas avanzadas"),
        checked: projectStore.expertMode,
        action: () => projectStore.setExpertMode(!projectStore.expertMode),
      }),
      await separator(),
      await item("fullscreen", t("Pantalla completa"), actions.toggleFullscreen, shortcut("F11")),
    ],
  });

  const disc = await Submenu.new({
    text: t("Disco"),
    items: [
      await item("produce", t("Producir disco…"), actions.produce, shortcut("Ctrl+R"), hasProject),
      await item("queue", t("Cola de discos…"), () => (ui.queue = true)),
      await item("history", t("Historial de trabajos…"), () => (ui.history = true)),
      await separator(),
      // Where the guide and the tour point people to.
      await item("join", t("Unir vídeos…"), () => (ui.join = true), undefined, hasProject),
      await item("cover", t("Carátula y etiqueta…"), () => (ui.cover = true), undefined, hasProject),
      await item("inspect", t("Inspeccionar disco…"), () => (ui.inspector = true), undefined, hasProject),
      await separator(),
      await item("tools", t("Herramientas de disco…"), () => (ui.doctor = true)),
    ],
  });

  const help = await Submenu.new({
    text: t("Ayuda"),
    items: [
      await item("guide", t("Guía paso a paso"), () => (projectStore.guideOpen = true), shortcut("F1")),
      await separator(),
      await CheckMenuItem.new({
        id: "update-check",
        text: t("Buscar actualizaciones al iniciar"),
        checked: updateChecksEnabled(),
        action: () => {
          setUpdateChecks(!updateChecksEnabled());
          void refreshAppMenu();
        },
      }),
      await item("about", t("Acerca de Kinodisc"), () => (ui.about = true)),
    ],
  });

  const menu = await Menu.new({ items: [file, edit, view, disc, help] });
  await menu.setAsAppMenu();
}

/**
 * App-level keyboard shortcuts (Windows/Linux; macOS uses the menu's
 * accelerators). Text fields keep their own undo and clipboard.
 */
export function handleShortcut(event: KeyboardEvent): boolean {
  if (event.key === "F11") {
    void actions.toggleFullscreen();
    return true;
  }
  if (event.key === "F1") {
    projectStore.guideOpen = true;
    return true;
  }
  if (isMac || !event.ctrlKey || event.altKey) return false;
  const target = event.target as HTMLElement | null;
  const typing =
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target?.isContentEditable === true;
  const key = event.key.toLowerCase();
  const run: Record<string, () => unknown> = event.shiftKey
    ? { s: actions.saveProjectAs, z: actions.redo }
    : {
        n: actions.newProject,
        o: actions.openProject,
        s: actions.saveProject,
        i: () => actions.importMedia(),
        r: actions.produce,
        q: () => getCurrentWindow().close(),
        z: actions.undo,
        y: actions.redo,
        d: () => projectStore.duplicateSelectedButton(),
      };
  const action = run[key];
  if (!action) return false;
  // Inside a text field, undo/redo belong to the field.
  if (typing && (key === "z" || key === "y" || key === "d")) return false;
  void action();
  return true;
}
