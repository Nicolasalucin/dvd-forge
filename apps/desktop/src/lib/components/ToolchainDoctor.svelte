<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { onMount } from "svelte";
  import { projectStore } from "../stores/project.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  onMount(() => {
    void projectStore.refreshToolsDoctor();
  });

  const report = $derived(projectStore.toolsReport);
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar diagnóstico")}
    onclick={onClose}
    class="absolute inset-0 cursor-default bg-slate-950/80 backdrop-blur-sm"
  ></button>

  <section class="relative z-10 flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950 shadow-2xl shadow-black/60">
    <header class="flex shrink-0 items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-[0.22em] text-cyan-400">{t("Diagnóstico")}</p>
        <h2 class="mt-1 text-lg font-semibold text-white">{t("Toolchain del sistema")}</h2>
        <p class="mt-0.5 text-[11px] text-slate-500">
          {t("Comprueba las herramientas externas que usan DVD, AVCHD y Blu-ray.")}
        </p>
      </div>
      <button
        type="button"
        onclick={onClose}
        class="rounded-lg border border-slate-800 px-2 py-1 text-xs text-slate-400 transition hover:border-slate-700 hover:text-slate-200"
      >
        ✕
      </button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if projectStore.toolsError}
        <p class="rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-200">
          {projectStore.toolsError}
        </p>
      {:else if projectStore.toolsBusy && !report}
        <p class="text-[11px] text-slate-500">{t("Comprobando herramientas…")}</p>
      {:else if report}
        <div class="mb-4 flex flex-wrap items-center gap-2">
          <span
            class={`rounded-full border px-2 py-0.5 text-[10px] font-semibold ${
              report.ok
                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
                : "border-rose-500/30 bg-rose-500/10 text-rose-200"
            }`}
          >
            {report.ok ? t("Toolchain completa") : t("Faltan herramientas obligatorias")}
          </span>
          <span class="font-mono text-[10px] text-slate-500">{report.platform}</span>
        </div>

        <div class="space-y-1.5">
          {#each report.tools as tool (tool.id)}
            <div class="rounded-xl border border-slate-800 bg-slate-900/50 px-3 py-2">
              <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-2">
                  <span
                    class={`h-2 w-2 shrink-0 rounded-full ${
                      tool.found ? "bg-emerald-400" : tool.required ? "bg-rose-400" : "bg-amber-400"
                    }`}
                  ></span>
                  <span class="truncate text-[11px] font-semibold text-slate-200">{tool.name}</span>
                  {#if tool.required}
                    <span class="rounded border border-slate-700 px-1 py-0.5 text-[8px] uppercase tracking-wider text-slate-500">
                      {t("requerida")}
                    </span>
                  {/if}
                </div>
                <span class="font-mono text-[9px] text-slate-500">
                  {tool.version ?? (tool.found ? t("versión desconocida") : t("no encontrada"))}
                </span>
              </div>
              {#if tool.path}
                <p class="mt-1 truncate font-mono text-[9px] text-slate-600" title={tool.path}>{tool.path}</p>
              {/if}
              {#if tool.error}
                <p class="mt-1 text-[10px] leading-4 text-amber-200/80">{tool.error}</p>
              {/if}
              {#if tool.detail && !tool.error}
                <p class="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">{tool.detail}</p>
              {/if}
            </div>
          {/each}
        </div>

        <div class="mt-4 rounded-xl border border-slate-800 bg-slate-900/50 px-3 py-2">
          <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-400">
            {t("Aceleración por hardware")}
          </p>
          {#if report.hardwareEncoders?.h264?.length}
            <div class="mt-1 flex flex-wrap gap-1">
              {#each report.hardwareEncoders.h264 as encoder}
                <span class="rounded border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[9px] text-emerald-200">
                  {encoder}
                </span>
              {/each}
            </div>
            <p class="mt-1 text-[9px] text-slate-500">
              {t("Los encoders se confirman con un frame de prueba; si fallan, el render cae a CPU.")}
            </p>
          {:else}
            <p class="mt-1 text-[10px] text-slate-500">
              {t("No se detectaron encoders H.264 por hardware; se usará libx264.")}
            </p>
          {/if}
        </div>

        {#if report.missingRequired.length > 0}
          <div class="mt-4 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[10px] leading-5 text-rose-200">
            Instala o añade al PATH: {report.missingRequired.join(", ")}.
          </div>
        {/if}
        {#if report.warnings.length > 0}
          <div class="mt-3 rounded-xl border border-amber-500/20 bg-amber-500/5 px-3 py-2">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-amber-200/80">{t("Avisos")}</p>
            <ul class="mt-1 space-y-0.5 text-[10px] leading-4 text-amber-100/70">
              {#each report.warnings as warning}
                <li>· {warning}</li>
              {/each}
            </ul>
          </div>
        {/if}
      {:else}
        <p class="text-[11px] text-slate-500">{t("No se pudo obtener el informe.")}</p>
      {/if}
    </div>

    <footer class="flex shrink-0 items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <button
        type="button"
        onclick={() => void projectStore.refreshToolsDoctor()}
        disabled={projectStore.toolsBusy}
        class="rounded-lg border border-slate-800 px-3 py-1.5 text-[11px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-50"
      >
        {projectStore.toolsBusy ? "Comprobando…" : "Volver a comprobar"}
      </button>
      <button
        type="button"
        onclick={onClose}
        class="rounded-lg bg-cyan-500 px-3 py-1.5 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400"
      >
        {t("Cerrar")}
      </button>
    </footer>
  </section>
</div>
