import { formatDecimal } from "../i18n/index.svelte";

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let index = 0;
  while (value >= 1000 && index < units.length - 1) {
    value /= 1000;
    index += 1;
  }
  return `${formatDecimal(value, index === 0 ? 0 : 1)} ${units[index]}`;
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "—";
  const totalSeconds = Math.round(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
  }
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

export function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/** A full path: `C:\…`, `C:/…`, a `\\server\share` UNC path or a Unix `/…`. */
export function isAbsolutePath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path) || /^[\\/]{2}[^\\/]/.test(path) || path.startsWith("/");
}

/**
 * Windows paths typed with `/` (`C:/Discs`) use `\` throughout, so messages
 * never show `C:/Discs\disc.iso`. Unix paths are returned unchanged.
 */
/**
 * A project name as a file or folder name: characters Windows forbids become
 * "_" (accents, emoji and other scripts are kept), trailing dots go.
 */
export function safeFileName(name: string, fallback = "Disc"): string {
  const cleaned = name
    .replace(/[\u0000-\u001f<>:"\/\\|?*]/g, "_")
    .trim()
    .replace(/[. ]+$/, "");
  return cleaned || fallback;
}

export function nativePath(path: string): string {
  return /^([A-Za-z]:|[\\/]{2}[^\\/])/.test(path) ? path.replace(/\//g, "\\") : path;
}

/**
 * Reads a decimal typed with either `.` or `,`. Number fields are plain text
 * because the WebView formats `type="number"` with the system's separator,
 * not the app's language. Returns NaN when it is not a number.
 */
export function parseDecimal(value: string): number {
  const text = value.trim().replace(",", ".");
  return /^-?(\d+\.?\d*|\.\d+)$/.test(text) ? Number(text) : Number.NaN;
}
