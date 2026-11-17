<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { onMount } from "svelte";
  import { projectStore } from "../stores/project.svelte";
  import { commands, type ToolInstallOption } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { localizeError } from "../i18n/diagnostics";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  onMount(() => {
    void projectStore.refreshToolsDoctor();
  });

  const report = $derived(projectStore.toolsReport);

  /** The Rust probe reports in Spanish; show it in the interface language. */
  function localizeToolMessage(message: string): string {
    const run = /^no se pudo ejecutar: (.*)$/s.exec(message);
    if (run) return t("no se pudo ejecutar: {error}", { error: run[1] });
    const code = /^el comando terminó con código (.*) y no mostró un banner reconocible$/.exec(message);
    if (code) return t("el comando terminó con código {code} y no mostró un banner reconocible", { code: code[1] });
    return t(message);
  }

  /** Report warnings are "<tool>: <message>" or the "no version" sentence. */
  function localizeToolWarning(warning: string): string {
    const version = /^(.+?) se encontró en (.+) pero no se pudo extraer una versión\.$/.exec(warning);
    if (version) {
      return t("{tool} se encontró en {path} pero no se pudo extraer una versión.", {
        tool: version[1],
        path: version[2],
      });
    }
    const separator = warning.indexOf(": ");
    if (separator > 0) {
      return `${warning.slice(0, separator)}: ${localizeToolMessage(warning.slice(separator + 2))}`;
    }
    return t(warning);
  }

  let options = $state<ToolInstallOption[]>([]);
  let installing = $state<string | null>(null);
  let installNotice = $state<string | null>(null);
  let installError = $state<string | null>(null);

  $effect(() => {
    const missing = report?.missingRequired ?? [];
    if (missing.length === 0) {
      options = [];
      return;
    }
    void commands
      .toolchainInstallOptions([...missing])
      .then((result) => (options = result))
      .catch(() => (options = []));
  });

  async function install(option: ToolInstallOption) {
    installing = option.tool;
    installError = null;
    installNotice = null;
    try {
      const result = await unwrap(commands.toolchainInstall(option.tool));
      installNotice = t("{tool} {version} instalado y verificado (SHA-256).", {
        tool: result.tool,
        version: result.version,
      });
      await projectStore.refreshToolsDoctor();
    } catch (cause) {
      installError = localizeError(messageOf(cause));
    } finally {
      installing = null;
    }
  }

  /** One archive can provide several tools (ffmpeg + ffprobe). */
  const automatic = $derived(
    options.filter(
      (option, index) =>
        option.automatic &&
        options.findIndex((other) => other.automatic && other.version === option.version) === index,
    ),
  );
  const manual = $derived(options.filter((option) => !option.automatic));
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape") onClose(); }} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar diagnóstico")}
    onclick={onClose}
    class="ui-backdrop"
  ></button>

  <section class="relative z-10 flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-slate-700/80 bg-slate-950 shadow-xl">
    <header class="flex shrink-0 items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
      <div>
        <h2 class="text-[16px] font-semibold text-slate-50">{t("Herramientas de disco")}</h2>
        <p class="mt-0.5 text-[12px] text-slate-500">
          {t("Programas que Kinodisc usa para convertir los vídeos y crear el disco. Los opcionales no hacen falta.")}
        </p>
      </div>
      <button
        type="button"
        onclick={onClose}
        class="ui-btn ui-btn-sm"
      >
        ✕
      </button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if projectStore.toolsError}
        <p class="rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[12px] text-rose-200">
          {projectStore.toolsError}
        </p>
      {:else if projectStore.toolsBusy && !report}
        <p class="text-[12px] text-slate-500">{t("Comprobando herramientas…")}</p>
      {:else if report}
        <div class="mb-4 flex flex-wrap items-center gap-2">
          <span
            class={`rounded-full border px-2 py-0.5 text-[11px] font-semibold ${
              report.ok
                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
                : "border-rose-500/30 bg-rose-500/10 text-rose-200"
            }`}
          >
            {report.ok ? t("Toolchain completa") : t("Faltan herramientas obligatorias")}
          </span>
          <span class="font-mono text-[11px] text-slate-500">{report.platform}</span>
        </div>

        <div class="space-y-1.5">
          {#each report.tools as tool (tool.id)}
            <div class="rounded-lg border border-slate-800 px-3 py-2">
              <div class="flex items-center justify-between gap-2">
                <div class="flex min-w-0 items-center gap-2">
                  <span
                    class={`h-2 w-2 shrink-0 rounded-full ${
                      tool.found ? "bg-emerald-500" : tool.required ? "bg-rose-500" : "bg-slate-500"
                    }`}
                  ></span>
                  <span class="truncate text-[12px] font-semibold text-slate-200">{tool.name}</span>
                  {#if tool.required}
                    <span class="ui-section-title px-1 py-0.5">
                      {t("requerida")}
                    </span>
                  {/if}
                </div>
                <span class="font-mono text-[11px] text-slate-500">
                  {tool.version ?? (tool.found ? t("versión desconocida") : tool.required ? t("no encontrada") : t("opcional, no instalada"))}
                </span>
              </div>
              {#if tool.path}
                <p class="mt-1 truncate font-mono text-[11px] text-slate-600" title={tool.path}>{tool.path}</p>
              {/if}
              {#if tool.error}
                {#if tool.required}<p class="mt-1 text-[12px] leading-5 text-amber-200">{localizeToolMessage(tool.error)}</p>{/if}
              {/if}
              {#if tool.detail && !tool.error}
                <p class="mt-1 line-clamp-2 text-[12px] leading-5 text-slate-500">{tool.detail}</p>
              {/if}
            </div>
          {/each}
        </div>

        <div class="mt-4 rounded-lg border border-slate-800 px-3 py-2">
          <p class="ui-section-title">
            {t("Aceleración por hardware")}
          </p>
          {#if report.hardwareEncoders?.h264?.length}
            <div class="mt-1 flex flex-wrap gap-1">
              {#each report.hardwareEncoders.h264 as encoder}
                <span class="rounded border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[11px] text-emerald-200">
                  {encoder}
                </span>
              {/each}
            </div>
            <p class="mt-1 text-[11px] text-slate-500">
              {t("Los encoders se confirman con un frame de prueba; si fallan, el render cae a CPU.")}
            </p>
          {:else}
            <p class="mt-1 text-[11px] text-slate-500">
              {t("No se detectaron encoders H.264 por hardware; se usará libx264.")}
            </p>
          {/if}
        </div>

        {#if report.missingRequired.length > 0}
          <div class="mt-4 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] leading-5 text-rose-200">
            {t("Instala o añade al PATH: {tools}.", { tools: report.missingRequired.join(", ") })}
          </div>
        {/if}
        {#if automatic.length > 0 || manual.length > 0 || installNotice}
          <div class="mt-3 space-y-2 rounded-xl border border-cyan-500/25 bg-cyan-500/5 px-3 py-2.5">
            {#each automatic as option (option.tool)}
              <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="min-w-0 text-[12px] text-slate-200">
                  <p class="font-semibold">
                    {t("Instalar {tool} automáticamente", { tool: option.tool === "ffprobe" ? "FFmpeg" : option.tool })}
                  </p>
                  <p class="text-[11px] text-slate-400">
                    {t("Versión {version} · {size} MB · se comprueba su SHA-256 · {license}", {
                      version: option.version ?? "",
                      size: Math.round((option.sizeBytes ?? 0) / 1_000_000),
                      license: option.license ?? "",
                    })}
                  </p>
                </div>
                <button
                  type="button"
                  onclick={() => install(option)}
                  disabled={installing !== null}
                  class="rounded-lg bg-cyan-500 px-3 py-1.5 text-[12px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-wait disabled:opacity-60"
                >
                  {installing === option.tool ? t("Descargando…") : t("Instalar")}
                </button>
              </div>
            {/each}
            {#each manual as option (option.tool)}
              {#if option.hint}
                <p class="text-[12px] leading-5 text-slate-400">
                  {option.tool}: <code class="rounded bg-slate-900 px-1 py-0.5 text-cyan-200">{option.hint}</code>
                </p>
              {/if}
            {/each}
            {#if installNotice}
              <p class="text-[11px] text-emerald-300">✓ {installNotice}</p>
            {/if}
            {#if installError}
              <p class="text-[11px] text-rose-300">{installError}</p>
            {/if}
          </div>
        {/if}
        {#if report.warnings.length > 0}
          <div class="mt-3 rounded-xl border border-amber-500/20 bg-amber-500/5 px-3 py-2">
            <p class="ui-section-title">{t("Avisos")}</p>
            <ul class="mt-1 space-y-0.5 text-[12px] leading-5 text-amber-100/70">
              {#each report.warnings as warning}
                <li>· {localizeToolWarning(warning)}</li>
              {/each}
            </ul>
          </div>
        {/if}
      {:else}
        <p class="text-[12px] text-slate-500">{t("No se pudo obtener el informe.")}</p>
      {/if}
    </div>

    <footer class="flex shrink-0 items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <button
        type="button"
        onclick={() => void projectStore.refreshToolsDoctor()}
        disabled={projectStore.toolsBusy}
        class="rounded-lg border border-slate-800 px-3 py-1.5 text-[12px] text-slate-300 transition hover:border-slate-500 hover:text-white disabled:opacity-50"
      >
        {projectStore.toolsBusy ? t("Comprobando…") : t("Volver a comprobar")}
      </button>
      <button
        type="button"
        onclick={onClose}
        class="ui-btn ui-btn-primary"
      >
        {t("Cerrar")}
      </button>
    </footer>
  </section>
</div>
