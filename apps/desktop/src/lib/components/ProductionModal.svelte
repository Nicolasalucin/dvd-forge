<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { Channel } from "@tauri-apps/api/core";
  import { videoDir } from "@tauri-apps/api/path";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import {
    commands,
    type BurnProgress_Deserialize,
    type JobEvent_Deserialize,
  } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { compositeProjectMenus } from "../production";
  import { projectStore } from "../stores/project.svelte";
  import { basename, formatBytes } from "../utils/format";

  type ProductionMode = "iso" | "isoBurn" | "existing";

  let mode = $state<ProductionMode>("iso");
  let outputDir = $state("");
  let existingIso = $state("");
  let selectedDevice = $state("");
  let speed = $state(4);
  let verify = $state(true);
  let eject = $state(true);
  let localError = $state<string | null>(null);
  let refreshing = $state(false);
  let defaultsReady = $state(false);
  let cancelRequested = $state(false);

  const project = $derived(projectStore.project);
  const devices = $derived(projectStore.devices);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const isBluRay = $derived(
    project?.disc.capacity === "bd25" || project?.disc.capacity === "bd50",
  );
  const speedOptions = $derived(isBluRay ? [1, 2, 4, 6, 8] : [2, 4, 8, 16]);
  const busy = $derived(projectStore.rendering || projectStore.burning);
  const renderPercent = $derived(Math.min(100, Math.max(0, projectStore.renderPercent)));
  const burnPercent = $derived(
    Math.min(100, Math.max(0, projectStore.burnProgress?.percent ?? 0)),
  );
  const needsBurn = $derived(mode === "isoBurn" || mode === "existing");
  const canStart = $derived(
    !busy &&
      (mode === "existing"
        ? existingIso.trim().length > 0
        : Boolean(project && project.episodes.length > 0 && outputDir.trim())),
  );
  const startLabel = $derived(
    busy
      ? projectStore.burning
        ? t("Grabando…")
        : t("Generando…")
      : mode === "existing"
        ? t("Grabar ISO")
        : mode === "isoBurn"
          ? t("Generar y grabar")
          : t("Generar ISO"),
  );
  const modes: Array<{
    id: ProductionMode;
    title: string;
    hint: string;
    icon: string;
  }> = [
    { id: "iso", title: "Solo ISO", hint: "Genera la imagen para guardarla o copiarla", icon: "◉" },
    { id: "isoBurn", title: "ISO + grabar", hint: "Genera y envía el disco en un flujo", icon: "▷" },
    { id: "existing", title: "ISO existente", hint: "Graba una imagen ya generada", icon: "▤" },
  ];

  onMount(() => {
    void prepareDefaults();
  });

  $effect(() => {
    if (!devices.some((device) => device.id === selectedDevice) && devices.length > 0) {
      selectedDevice = devices[0].id;
    }
  });

  $effect(() => {
    if (!speedOptions.includes(speed)) speed = speedOptions[0];
  });

  async function prepareDefaults() {
    if (!outputDir) {
      try {
        outputDir = await videoDir();
      } catch {
        outputDir = "";
      }
    }
    defaultsReady = true;
    if (devices.length === 0) await refreshDevices();
  }

  async function refreshDevices() {
    refreshing = true;
    try {
      await projectStore.refreshDevices();
    } finally {
      refreshing = false;
    }
  }

  async function chooseOutput() {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked === "string") outputDir = picked;
  }

  async function chooseExistingIso() {
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Imagen ISO"), extensions: ["iso"] }],
    });
    if (typeof picked === "string") existingIso = picked;
  }

  function close() {
    if (busy) return;
    projectStore.productionOpen = false;
  }

  async function start() {
    localError = null;
    cancelRequested = false;
    if (mode === "existing") {
      await burnIso(existingIso.trim());
      return;
    }
    const current = project;
    if (!current) {
      localError = t("No hay un proyecto abierto.");
      return;
    }
    if (!outputDir.trim()) {
      localError = t("Elige una carpeta de salida.");
      return;
    }
    projectStore.resetRender();
    projectStore.rendering = true;
    projectStore.renderStage = t("Componiendo menús");
    try {
      const warnings = await compositeProjectMenus(current, {
        backgroundColor: projectStore.menuBackgroundColor,
        previewUrls: projectStore.previewUrls,
        framePaths: projectStore.framePaths,
      });
      for (const warning of warnings) {
        projectStore.renderLogs = [...projectStore.renderLogs, `⚠ ${warning}`].slice(-300);
      }
      if (cancelRequested) {
        projectStore.renderStage = t("Cancelado");
        projectStore.renderSummary = t("Generación cancelada.");
        return;
      }
      const channel = new Channel<JobEvent_Deserialize>();
      channel.onmessage = (event) => projectStore.applyJobEvent(event);
      const outcome = await unwrap(
        commands.renderStart(current, outputDir.trim(), channel),
      );
      projectStore.lastIso = outcome.isoPath;
      projectStore.renderPercent = 100;
      if (!projectStore.renderSummary) {
        projectStore.renderSummary = t("ISO generado en {seconds} s", { seconds: (outcome.elapsedMs / 1000).toFixed(1) });
      }
      if (mode === "isoBurn") {
        if (cancelRequested) {
          projectStore.renderSummary = t("Generación cancelada antes de grabar.");
          return;
        }
        if (!selectedDevice) {
          localError = t("El ISO está listo, pero no hay una unidad seleccionada para grabarlo.");
          return;
        }
        projectStore.rendering = false;
        await burnIso(outcome.isoPath);
      }
    } catch (cause) {
      if (cancelRequested) {
        projectStore.renderError = null;
        projectStore.renderStage = t("Cancelado");
        projectStore.renderSummary = t("Generación cancelada.");
      } else {
        projectStore.renderError = messageOf(cause);
      }
    } finally {
      projectStore.rendering = false;
    }
  }

  async function burnIso(isoPath: string) {
    localError = null;
    if (!isoPath) {
      localError = t("Elige un archivo ISO.");
      return;
    }
    if (!selectedDevice) {
      localError = t("No se detectó ninguna unidad grabable. Conecta una unidad y pulsa «Actualizar».");
      return;
    }
    projectStore.burnError = null;
    projectStore.burnReport = null;
    projectStore.burnProgress = null;
    projectStore.burning = true;
    const channel = new Channel<BurnProgress_Deserialize>();
    channel.onmessage = (event) => projectStore.applyBurnProgress(event);
    try {
      const report = await unwrap(
        commands.burnStart(isoPath, selectedDevice, speed, verify, eject, channel),
      );
      projectStore.burnReport = report;
      projectStore.lastIso = isoPath;
    } catch (cause) {
      if (cancelRequested) {
        projectStore.burnError = null;
        projectStore.renderSummary = t("Grabación cancelada.");
      } else {
        projectStore.burnError = messageOf(cause);
      }
    } finally {
      projectStore.burning = false;
    }
  }

  async function cancel() {
    localError = null;
    cancelRequested = true;
    projectStore.renderStage = t("Cancelando…");
    // A render/burn task may not exist yet (for example while the editor is
    // compositing layers). The flag above stops the flow before it starts.
    try {
      if (projectStore.rendering) await unwrap(commands.renderCancel());
    } catch {
      // "No hay una generación activa" is expected during composition.
    }
    try {
      if (projectStore.burning) await unwrap(commands.burnCancel());
    } catch {
      // The burn may have finished between the click and the command.
    }
  }

  async function openOutput() {
    const target =
      mode === "existing" && existingIso
        ? existingIso
        : (projectStore.lastIso ?? outputDir);
    if (!target) {
      localError = t("Todavía no hay una carpeta de salida.");
      return;
    }
    try {
      await unwrap(commands.openFolder(target));
    } catch (cause) {
      localError = messageOf(cause);
    }
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar producción")}
    onclick={close}
    class="absolute inset-0 cursor-default bg-slate-950/80 backdrop-blur-sm"
  ></button>

  <div class="relative z-10 flex max-h-[92vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950 shadow-2xl shadow-black/60">
    <header class="flex shrink-0 items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-[0.22em] text-cyan-400">{t("Producción")}</p>
        <h2 class="mt-1 text-lg font-semibold text-white">{t("Generar y grabar disco")}</h2>
        <p class="mt-0.5 text-[11px] text-slate-500">
          {isBluRay ? "Blu-ray HDMV" : isAvchd ? "AVCHD HD" : t("DVD-Video interactivo")} ·
          {project?.episodes.length ?? 0}
          {t((project?.episodes.length ?? 0) === 1 ? "episodio" : "episodios")}
        </p>
      </div>
      <button
        type="button"
        onclick={close}
        disabled={busy}
        class="rounded-lg border border-slate-800 px-2 py-1 text-xs text-slate-400 transition hover:border-slate-700 hover:text-slate-200 disabled:opacity-40"
      >
        ✕
      </button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if localError || projectStore.renderError || projectStore.burnError}
        <div class="mb-4 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] leading-5 text-rose-200">
          {localError ?? projectStore.renderError ?? projectStore.burnError}
        </div>
      {/if}

      <div class="grid gap-2 sm:grid-cols-3">
        {#each modes as item}
          <button
            type="button"
            onclick={() => (mode = item.id)}
            disabled={busy}
            class={`rounded-xl border p-3 text-left transition disabled:opacity-50 ${
              mode === item.id
                ? "border-cyan-500/60 bg-cyan-500/10"
                : "border-slate-800 bg-slate-900/40 hover:border-slate-700"
            }`}
          >
            <span class="text-base text-cyan-300">{item.icon}</span>
            <p class="mt-1 text-[11px] font-semibold text-slate-100">{t(item.title)}</p>
            <p class="mt-0.5 text-[10px] leading-4 text-slate-500">{t(item.hint)}</p>
          </button>
        {/each}
      </div>

      <div class="mt-4 grid gap-4 lg:grid-cols-[1fr_320px]">
        <div class="space-y-3">
          {#if mode === "existing"}
            <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
              <div class="flex items-center justify-between">
                <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Imagen ISO")}</span>
                <button
                  type="button"
                  onclick={chooseExistingIso}
                  disabled={busy}
                  class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 disabled:opacity-50"
                >
                  {t("Elegir…")}
                </button>
              </div>
              <input
                class="mt-2 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-1.5 font-mono text-[11px] text-slate-200 outline-none focus:border-cyan-500/60"
                bind:value={existingIso}
                placeholder={t("/ruta/al/disco.iso")}
                disabled={busy}
              />
              {#if existingIso}
                <p class="mt-1 truncate text-[10px] text-slate-500">{basename(existingIso)}</p>
              {/if}
            </div>
          {:else}
            <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
              <div class="flex items-center justify-between">
                <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Carpeta de salida")}</span>
                <button
                  type="button"
                  onclick={chooseOutput}
                  disabled={busy}
                  class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 disabled:opacity-50"
                >
                  {t("Elegir…")}
                </button>
              </div>
              <input
                class="mt-2 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-1.5 font-mono text-[11px] text-slate-200 outline-none focus:border-cyan-500/60"
                bind:value={outputDir}
                disabled={busy}
              />
              <p class="mt-1 text-[10px] text-slate-500">
                {defaultsReady
                  ? t("El ISO se escribirá aquí con el nombre del proyecto.")
                  : t("Preparando carpeta predeterminada…")}
              </p>
            </div>
          {/if}

          <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
            <div class="flex items-center justify-between">
              <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">
                {t("Progreso")}
              </span>
              <span class="font-mono text-[11px] text-cyan-300">
                {projectStore.burning
                  ? `${burnPercent.toFixed(1)}%`
                  : `${renderPercent.toFixed(1)}%`}
              </span>
            </div>
            <div class="mt-2 h-2 overflow-hidden rounded-full border border-slate-800 bg-slate-950">
              <div
                class={`h-full rounded-full transition-all duration-300 ${
                  projectStore.renderError || projectStore.burnError
                    ? "bg-gradient-to-r from-rose-500 to-orange-400"
                    : "bg-gradient-to-r from-cyan-500 via-sky-400 to-emerald-400"
                }`}
                style={`width: ${projectStore.burning ? burnPercent : renderPercent}%`}
              ></div>
            </div>
            <p class="mt-1.5 truncate text-[10px] text-slate-500">
              {#if projectStore.rendering}
                {projectStore.renderStage || t("Generando…")}
              {:else if projectStore.burning}
                {t("Grabando en")} {selectedDevice || t("la unidad")} · {formatBytes(projectStore.burnProgress?.writtenBytes ?? 0)} {t("de")} {formatBytes(projectStore.burnProgress?.totalBytes ?? 0)}
              {:else if projectStore.renderSummary}
                {projectStore.renderSummary}
              {:else}
                {t("Listo para comenzar.")}
              {/if}
            </p>
            {#if projectStore.burnReport}
              <p class="mt-1 text-[10px] text-emerald-300">
                {t("Grabación completada")} · {projectStore.burnReport.backend} · {(projectStore.burnReport.elapsedMs / 1000).toFixed(1)} s
                {#if projectStore.burnReport.verified}· {t("verificada")}{/if}
              </p>
            {/if}
            {#if projectStore.lastIso}
              <div class="mt-2 flex items-center justify-between gap-2 rounded-lg border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                <span class="min-w-0 truncate font-mono text-[10px] text-slate-400" title={projectStore.lastIso}>
                  {basename(projectStore.lastIso)}
                </span>
                <button
                  type="button"
                  onclick={openOutput}
                  class="shrink-0 rounded-md border border-slate-700 px-2 py-0.5 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
                >
                  {t("Abrir carpeta")}
                </button>
              </div>
            {/if}
          </div>

          {#if projectStore.renderLogs.length > 0}
            <details class="rounded-xl border border-slate-800 bg-slate-900/30 p-3">
              <summary class="cursor-pointer select-none text-[10px] font-semibold uppercase tracking-wider text-slate-500">
                {t("Registro")} · {projectStore.renderLogs.length} {t("líneas")}
              </summary>
              <div class="mt-2 max-h-44 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950 p-2 font-mono text-[10px] leading-4 text-slate-500">
                {#each projectStore.renderLogs as line}
                  <p class="whitespace-pre-wrap">{line}</p>
                {/each}
              </div>
            </details>
          {/if}
        </div>

        <aside class="space-y-3">
          {#if needsBurn}
            <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
              <div class="flex items-center justify-between">
                <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Unidad")}</span>
                <button
                  type="button"
                  onclick={refreshDevices}
                  disabled={refreshing || busy}
                  class="rounded-md border border-slate-700 px-2 py-0.5 text-[10px] text-slate-300 transition hover:border-cyan-500/60 disabled:opacity-50"
                >
                  {refreshing ? "…" : "Actualizar"}
                </button>
              </div>
              {#if devices.length === 0}
                <p class="mt-2 rounded-lg border border-amber-500/20 bg-amber-500/5 px-2 py-1.5 text-[10px] leading-4 text-amber-200/80">
                  {t("No se detectaron unidades ópticas. Puedes generar el ISO y grabarlo después.")}
                </p>
              {:else}
                <select
                  class="mt-2 w-full rounded-lg border border-slate-800 bg-slate-950 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  bind:value={selectedDevice}
                  disabled={busy}
                >
                  {#each devices as device}
                    <option value={device.id}>
                      {device.id} · {device.vendor} {device.model}
                    </option>
                  {/each}
                </select>
              {/if}
            </div>

            <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
              <label class="block">
                <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Velocidad")}</span>
                <select
                  class="w-full rounded-lg border border-slate-800 bg-slate-950 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  bind:value={speed}
                  disabled={busy}
                >
                  {#each speedOptions as option}
                    <option value={option}>
                      {option}x{isBluRay ? ` · ${(option * 4.495).toFixed(1)} MB/s` : ""}
                    </option>
                  {/each}
                </select>
              </label>
              <label class="mt-2 flex items-center gap-1.5 text-[10px] text-slate-400">
                <input type="checkbox" bind:checked={verify} disabled={busy} class="accent-cyan-500" />
                {t("Verificar por lectura SHA-256")}
              </label>
              <label class="mt-1 flex items-center gap-1.5 text-[10px] text-slate-400">
                <input type="checkbox" bind:checked={eject} disabled={busy} class="accent-cyan-500" />
                {t("Expulsar al terminar")}
              </label>
            </div>
          {:else}
            <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
              <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Salida")}</p>
              <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
                {t("Se generará un archivo")} <span class="font-mono text-slate-300">{t(".iso")}</span> {t("listo para grabarlo con cualquier herramienta del sistema.")}
              </p>
            </div>
          {/if}

          <div class="rounded-xl border border-slate-800 bg-slate-900/40 p-3">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Plan")}</p>
            {#if projectStore.plan}
              <div class="mt-1.5 flex items-center justify-between text-[10px]">
                <span class="text-slate-500">{t("Ocupación")}</span>
                <span class={projectStore.plan.feasible ? "font-mono text-emerald-300" : "font-mono text-rose-300"}>
                  {projectStore.planPercent.toFixed(1)}%
                </span>
              </div>
              <div class="mt-1 flex items-center justify-between text-[10px]">
                <span class="text-slate-500">{t("Tasa de bits")}</span>
                <span class="font-mono text-slate-300">{projectStore.plan.videoKbps} kbps</span>
              </div>
              <div class="mt-1 flex items-center justify-between text-[10px]">
                <span class="text-slate-500">{t("Calidad")}</span>
                <span class="text-slate-300">{projectStore.qualityLabel}</span>
              </div>
            {:else}
              <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
                {t("El plan se calculará automáticamente al generar el disco.")}
              </p>
            {/if}
          </div>
        </aside>
      </div>
    </div>

    <footer class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <div class="flex items-center gap-2">
        {#if busy}
          <button
            type="button"
            onclick={cancel}
            class="rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 text-[11px] font-semibold text-rose-200 transition hover:bg-rose-500/20"
          >
            {t("Cancelar")}
          </button>
          <span class="text-[10px] text-slate-500">
            {projectStore.rendering ? t("Generación en curso") : t("Grabación en curso")}
          </span>
        {:else}
          <button
            type="button"
            onclick={openOutput}
            class="rounded-lg border border-slate-800 px-3 py-1.5 text-[11px] text-slate-400 transition hover:border-slate-700 hover:text-slate-200"
          >
            {t("Abrir carpeta")}
          </button>
        {/if}
      </div>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={close}
          disabled={busy}
          class="rounded-lg border border-slate-800 px-3 py-1.5 text-[11px] text-slate-300 transition hover:border-slate-700 disabled:opacity-40"
        >
          {t("Cerrar")}
        </button>
        <button
          type="button"
          onclick={start}
          disabled={!canStart}
          class="rounded-lg bg-cyan-500 px-4 py-1.5 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
        >
          {startLabel}
        </button>
      </div>
    </footer>
  </div>
</div>
