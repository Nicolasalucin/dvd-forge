/**
 * What happens when a long job ends (production, burning, the disc queue):
 * a short chime, the taskbar entry flashes if the window is in the
 * background, and, when the user asked for it, a 60 s shutdown countdown.
 * No notification plugin: built-in window attention and Web Audio only.
 */
import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { ui } from "./state.svelte";

const SHUTDOWN_SECONDS = 60;
let timer: ReturnType<typeof setInterval> | null = null;

function chime(success: boolean): void {
  try {
    const AudioContextClass =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextClass) return;
    const context = new AudioContextClass();
    // Two rising notes for success, one low note for a failure.
    const notes = success ? [660, 880] : [330];
    notes.forEach((frequency, index) => {
      const start = context.currentTime + index * 0.18;
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.type = "sine";
      oscillator.frequency.value = frequency;
      gain.gain.setValueAtTime(0.0001, start);
      gain.gain.exponentialRampToValueAtTime(0.18, start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.35);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(start);
      oscillator.stop(start + 0.4);
    });
    setTimeout(() => void context.close(), 1200);
  } catch {
    // Sound is a courtesy; never fail the job because of it.
  }
}

/** Call once when a long job ends. `success` false = failed or cancelled. */
export async function announceFinished(success: boolean): Promise<void> {
  chime(success);
  try {
    if (!document.hasFocus()) {
      await getCurrentWindow().requestUserAttention(
        success ? UserAttentionType.Informational : UserAttentionType.Critical,
      );
    }
  } catch {
    // Not every platform supports attention requests.
  }
  if (success && ui.shutdownWhenDone) startShutdownCountdown();
  ui.shutdownWhenDone = false;
}

export function startShutdownCountdown(): void {
  cancelShutdown();
  ui.shutdownCountdown = SHUTDOWN_SECONDS;
  timer = setInterval(() => {
    const left = (ui.shutdownCountdown ?? 0) - 1;
    if (left > 0) {
      ui.shutdownCountdown = left;
      return;
    }
    cancelShutdown();
    void import("../api/bindings").then(({ commands }) =>
      commands.systemShutdown().then((result) => {
        if (result.status === "error") ui.notice = result.error;
      }),
    );
  }, 1000);
}

export function cancelShutdown(): void {
  if (timer) clearInterval(timer);
  timer = null;
  ui.shutdownCountdown = null;
}
