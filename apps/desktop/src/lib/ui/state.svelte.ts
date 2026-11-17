/** Interface state shared by the toolbar, the native menu and shortcuts. */
export type ThemePreference = "system" | "light" | "dark";

const THEME_KEY = "kinodisc_theme";

function readTheme(): ThemePreference {
  try {
    const value = window.localStorage.getItem(THEME_KEY);
    if (value === "light" || value === "dark") return value;
  } catch {
    // Storage may be unavailable.
  }
  return "system";
}

class UiState {
  newProject = $state(false);
  about = $state(false);
  queue = $state(false);
  doctor = $state(false);
  inspector = $state(false);
  slideshow = $state(false);
  join = $state(false);
  cover = $state(false);
  history = $state(false);
  recent = $state(false);
  notice = $state<string | null>(null);
  /** "Shut down when done" for the current production or queue (never kept). */
  shutdownWhenDone = $state(false);
  /** Seconds left before shutting down; null when no countdown is running. */
  shutdownCountdown = $state<number | null>(null);
  /** A newer Kinodisc release (status bar notice), or null. */
  update = $state<{ version: string; url: string } | null>(null);
  theme = $state<ThemePreference>(readTheme());
  fullscreen = $state(false);

  setTheme(theme: ThemePreference): void {
    this.theme = theme;
    applyTheme(theme);
    try {
      window.localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Best effort.
    }
  }
}

export function applyTheme(theme: ThemePreference): void {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
  // The title bar and native menu bar follow the window theme on Windows.
  void import("@tauri-apps/api/window")
    .then(({ getCurrentWindow }) => getCurrentWindow().setTheme(theme === "system" ? null : theme))
    .catch(() => undefined);
}

export const ui = new UiState();
