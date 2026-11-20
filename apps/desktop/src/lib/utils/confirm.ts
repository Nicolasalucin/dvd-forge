import { confirm } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n/index.svelte";

/**
 * Asks before a destructive action and waits for the answer.
 * (`window.confirm` does not block in the webview: the action ran while the
 * question was still on screen, QA 1.2.)
 */
export async function confirmDelete(question: string): Promise<boolean> {
  try {
    return await confirm(question, {
      kind: "warning",
      okLabel: t("Eliminar"),
      cancelLabel: t("Cancelar"),
    });
  } catch {
    return false;
  }
}
