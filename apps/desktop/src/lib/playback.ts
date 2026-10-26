import { convertFileSrc } from "@tauri-apps/api/core";
import { commands, type PlaybackSupport_Serialize } from "./api/bindings";
import { unwrap } from "./api/client";

let cached: Promise<PlaybackSupport_Serialize> | null = null;

/**
 * WebKitGTK no reproduce `<video>` desde `asset://`; el backend expone el
 * archivo por HTTP local con Range. Si el comando falla se mantiene el asset
 * protocol como fallback para otras plataformas.
 */
export async function previewUrlFor(path: string): Promise<string> {
  try {
    return await unwrap(commands.mediaPreviewUrl(path));
  } catch {
    return convertFileSrc(path);
  }
}

/**
 * Comprueba una sola vez si WebKit puede montar `<video>`. En Linux sin los
 * plugins de GStreamer, montar el elemento congela el proceso web de WebKitGTK,
 * así que la UI espera esta respuesta antes de crearlo.
 */
export function loadPlaybackSupport(force = false): Promise<PlaybackSupport_Serialize> {
  if (!cached || force) {
    cached = commands.mediaPlaybackSupport().catch((error) => fallbackSupport(error));
  }
  return cached;
}

function fallbackSupport(error: unknown): PlaybackSupport_Serialize {
  const linux =
    typeof navigator !== "undefined" && /Linux/i.test(navigator.userAgent);
  const message = error instanceof Error ? error.message : String(error);
  return {
    supported: !linux,
    platform: linux ? "linux" : "unknown",
    missing: [],
    detail: linux
      ? `No se pudo comprobar GStreamer; la reproducción queda desactivada para evitar un cuelgue. ${message}`
      : `No se pudo comprobar el soporte de reproducción. ${message}`,
    installHint: linux
      ? "Arch/CachyOS: sudo pacman -S gst-plugins-good gst-libav. Debian/Ubuntu: sudo apt install gstreamer1.0-plugins-good gstreamer1.0-libav."
      : null,
  };
}
