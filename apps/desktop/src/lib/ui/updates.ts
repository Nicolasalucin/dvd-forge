/**
 * Update notice: at most one check a day (GitHub releases API through the
 * backend; nothing about the user is sent), shown in the status bar with a
 * link to the release page. Never downloads or installs anything. Can be
 * turned off in Help.
 */
import { commands } from "../api/bindings";
import { ui } from "./state.svelte";

const ENABLED_KEY = "kinodisc_update_check";
const LAST_KEY = "kinodisc_update_last";
const FOUND_KEY = "kinodisc_update_found";
const DAY_MS = 24 * 60 * 60 * 1000;

function read(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value === null) window.localStorage.removeItem(key);
    else window.localStorage.setItem(key, value);
  } catch {
    // Private storage unavailable: the check simply runs again next time.
  }
}

export function updateChecksEnabled(): boolean {
  return read(ENABLED_KEY) !== "off";
}

export function setUpdateChecks(enabled: boolean): void {
  write(ENABLED_KEY, enabled ? "on" : "off");
  if (!enabled) {
    ui.update = null;
    write(FOUND_KEY, null);
  } else {
    void checkForUpdates(true);
  }
}

/** On start-up: show a cached result, ask GitHub once a day. */
export async function checkForUpdates(force = false): Promise<void> {
  if (!updateChecksEnabled()) return;
  const cached = read(FOUND_KEY);
  if (cached && !force) {
    try {
      ui.update = JSON.parse(cached);
    } catch {
      write(FOUND_KEY, null);
    }
  }
  const last = Number(read(LAST_KEY) ?? 0);
  if (!force && Date.now() - last < DAY_MS) return;
  const result = await commands.updateCheck();
  if (result.status === "error") return;
  write(LAST_KEY, String(Date.now()));
  ui.update = result.data;
  write(FOUND_KEY, result.data ? JSON.stringify(result.data) : null);
}

export async function openUpdate(): Promise<void> {
  if (!ui.update) return;
  const result = await commands.openReleasePage(ui.update.url);
  if (result.status === "error") ui.notice = result.error;
}
