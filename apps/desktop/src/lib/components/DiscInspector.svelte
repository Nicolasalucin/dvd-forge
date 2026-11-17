<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { commands, type DiscInspection } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import { formatBytes } from "../utils/format";

  let { onclose }: { onclose: () => void } = $props();

  let source = $state<string | null>(null);
  let report = $state<DiscInspection | null>(null);
  let working = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const importable = $derived(
    Boolean(report && !report.encrypted && report.kind !== "data" && report.titles.length > 0),
  );

  function clock(ms: number | null): string {
    const seconds = Math.round((ms ?? 0) / 1000);
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = seconds % 60;
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }

  async function inspect(path: string) {
    working = true;
    error = null;
    notice = null;
    report = null;
    try {
      report = await unwrap(commands.discInspect(path));
      source = path;
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      working = false;
    }
  }

  async function pickImage() {
    const picked = await open({ multiple: false, filters: [{ name: t("Imagen de disco"), extensions: ["iso"] }] });
    if (typeof picked === "string") await inspect(picked);
  }

  async function pickFolder() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") await inspect(picked);
  }

  async function importTitles() {
    if (!source || !report) return;
    const destination = await open({ directory: true, multiple: false, title: t("Carpeta donde copiar los títulos") });
    if (typeof destination !== "string") return;
    working = true;
    error = null;
    try {
      const titles = await unwrap(commands.discImport(source, destination));
      const added = await projectStore.addImportedTitles(titles);
      notice = t("{count} títulos añadidos como episodios.", { count: added });
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      working = false;
    }
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !working) onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" aria-label={t("Cerrar")} class="absolute inset-0 cursor-default bg-slate-950/80" onclick={() => !working && onclose()}></button>
  <div
    class="relative z-10 flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-slate-700 bg-slate-950"
    role="dialog"
    aria-modal="true"
    aria-labelledby="inspector-title"
  >
    <header class="border-b border-slate-800 px-5 py-4">
      <h2 id="inspector-title" class="text-base font-semibold text-white">{t("Inspeccionar disco")}</h2>
      <p class="mt-1 text-xs text-slate-400">
        {t("Mira qué hay en una imagen ISO o en una carpeta VIDEO_TS/BDMV y, si no está cifrado, impórtalo como episodios.")}
      </p>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-5 py-4 text-sm text-slate-200">
      <div class="flex flex-wrap gap-2">
        <button type="button" class="rounded-md border border-slate-600 px-3 py-1.5 text-sm hover:border-cyan-500" onclick={pickImage} disabled={working}>{t("Abrir ISO…")}</button>
        <button type="button" class="rounded-md border border-slate-600 px-3 py-1.5 text-sm hover:border-cyan-500" onclick={pickFolder} disabled={working}>{t("Abrir carpeta…")}</button>
        {#if working}<span class="self-center text-xs text-slate-400">{t("Leyendo…")}</span>{/if}
      </div>

      {#if report}
        <div class="rounded-md border border-slate-800 px-3 py-2 text-xs text-slate-300">
          <p class="font-medium text-slate-100">
            {report.kind === "dvdVideo" ? "DVD-Video" : report.kind === "bdmv" ? "Blu-ray / AVCHD" : t("Disco de datos")}
            · {report.volumeLabel}
          </p>
          <p class="mt-1 text-slate-400">
            {t("{count} archivos", { count: report.fileCount })} · {formatBytes(report.totalBytes)}{report.video ? ` · ${report.video}` : ""}
          </p>
          {#if report.encrypted}
            <p class="mt-1 text-amber-300">{t("Cifrado: se puede ver, pero no importar.")}</p>
          {/if}
        </div>

        {#if report.titles.length > 0}
          <table class="w-full text-left text-xs">
            <thead class="text-slate-400">
              <tr>
                <th class="py-1 font-normal">{t("Título")}</th>
                <th class="py-1 font-normal">{t("Duración")}</th>
                <th class="py-1 font-normal">{t("Capítulos")}</th>
                <th class="py-1 font-normal">{t("Audio")}</th>
                <th class="py-1 font-normal">{t("Subtítulos")}</th>
                <th class="py-1 text-right font-normal">{t("Tamaño")}</th>
              </tr>
            </thead>
            <tbody>
              {#each report.titles as title (title.number)}
                <tr class={`border-t border-slate-800 ${title.duplicate ? "text-slate-500" : ""}`}>
                  <td class="py-1 tabular-nums">{title.number}{title.duplicate ? ` (${t("repetido")})` : ""}</td>
                  <td class="py-1 tabular-nums">{clock(title.durationMs)}</td>
                  <td class="py-1 tabular-nums">{title.chapters}</td>
                  <td class="py-1">{title.audio.map((stream) => [stream.language, stream.codec].filter(Boolean).join(" ")).join(", ") || "—"}</td>
                  <td class="py-1">{title.subtitles.length || "—"}</td>
                  <td class="py-1 text-right tabular-nums">{formatBytes(title.bytes)}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}

        {#each report.notes as note}
          <p class="text-xs text-slate-400">{note}</p>
        {/each}
      {/if}

      {#if notice}<p class="rounded-md bg-emerald-500/10 px-3 py-2 text-xs text-emerald-200" role="status">{notice}</p>{/if}
      {#if error}<p class="rounded-md bg-rose-500/10 px-3 py-2 text-xs text-rose-200" role="alert">{error}</p>{/if}
    </div>

    <footer class="flex justify-end gap-2 border-t border-slate-800 px-5 py-3">
      <button type="button" class="rounded-md px-3 py-1.5 text-sm text-slate-300 hover:text-white" onclick={onclose} disabled={working}>{t("Cerrar")}</button>
      <button
        type="button"
        class="rounded-md bg-cyan-500 px-3 py-1.5 text-sm font-semibold text-slate-950 hover:bg-cyan-400 disabled:opacity-50"
        onclick={importTitles}
        disabled={working || !importable || !projectStore.project}
      >{t("Importar como episodios…")}</button>
    </footer>
  </div>
</div>
