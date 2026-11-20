<script lang="ts">
  import { isAbsolutePath, nativePath } from "../utils/format";
  import { announceFinished } from "../ui/finish";
  import { ui } from "../ui/state.svelte";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../ui/icons";
  const modeIcons: Record<string, IconName> = { iso: "disc", folder: "folder", isoBurn: "burn", existing: "open" };
  import { i18n, t, formatDecimal, formatPercent } from "../i18n/index.svelte";
  import { localizeCheck, localizeDiagnostic, localizeProjectErrors, localizeRenderError } from "../i18n/diagnostics";
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
  import { basename, formatBytes, safeFileName } from "../utils/format";

  // The output folder is remembered between productions (on this computer).
  const OUTPUT_KEY = "kinodisc_output_dir";
  function rememberedOutput(): string {
    try {
      return localStorage.getItem(OUTPUT_KEY) ?? "";
    } catch {
      return "";
    }
  }
  function rememberOutput(path: string) {
    try {
      localStorage.setItem(OUTPUT_KEY, path);
    } catch {
      // Storage can be unavailable; the default folder is used next time.
    }
  }

  // Episodes no menu button can start: shown before producing the disc.
  // and Blu-ray subtitles that would be left out.
  const PREFLIGHT = new Set(["W_EPISODE_NO_BUTTON", "W_PGS_LAYOUT_MISMATCH", "W_AVCHD_AUDIO_LAYOUT"]);
  const unreachable = $derived(
    projectStore.plan?.warnings.filter((warning) => PREFLIGHT.has(warning.code)) ?? [],
  );

  type ProductionMode = "iso" | "folder" | "isoBurn" | "existing";

  let mode = $state<ProductionMode>("iso");
  let outputDir = $state("");
  let existingIso = $state("");
  let selectedDevice = $state("");
  let speed = $state(4);
  let verify = $state(true);
  let eject = $state(true);
  let localError = $state<string | null>(null);
  let producedList = $state<string[]>([]);
  let refreshing = $state(false);
  let defaultsReady = $state(false);
  let cancelRequested = $state(false);
  // Several copies in a row: the same image is burned again once the user
  // has swapped the disc. `waitingIso` is set while the dialog waits for that.
  const MAX_COPIES = 20;
  let copies = $state(1);
  let seriesTotal = $state(1);
  let seriesDone = $state(0);
  let waitingIso = $state<string | null>(null);
  function clampCopies(value: number): number {
    return Number.isFinite(value) ? Math.min(MAX_COPIES, Math.max(1, Math.round(value))) : 1;
  }
  let hardwareEncoding = $state<"auto" | "cpuOnly">("auto");
  // "One disc per episode" and the like are remembered per project.
  type DiscSplit = "single" | "fitToDisc" | "perEpisode";
  const splitKey = `kinodisc_split_${projectStore.project?.id ?? ""}`;
  function rememberedSplit(): DiscSplit {
    try {
      const value = localStorage.getItem(splitKey);
      return value === "fitToDisc" || value === "perEpisode" ? value : "single";
    } catch {
      return "single";
    }
  }
  let discSplit = $state<DiscSplit>(rememberedSplit());
  $effect(() => {
    try {
      // Only non-default choices are kept (no key per project forever).
      if (discSplit === "single") localStorage.removeItem(splitKey);
      else localStorage.setItem(splitKey, discSplit);
    } catch {
      // Not remembered this time.
    }
  });

  const project = $derived(projectStore.project);
  const devices = $derived(projectStore.devices);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const hardwareEncoder = $derived(
    isAvchd ? projectStore.toolsReport?.hardwareEncoders?.h264?.[0] ?? null : null,
  );
  const isBluRay = $derived(
    project?.disc.capacity === "bd25" || project?.disc.capacity === "bd50",
  );
  const speedOptions = $derived(isBluRay ? [1, 2, 4, 6, 8] : [2, 4, 8, 16]);
  const effectiveDevice = $derived(
    devices.some((device) => device.id === selectedDevice)
      ? selectedDevice
      : (devices[0]?.id ?? ""),
  );
  const effectiveSpeed = $derived(
    speedOptions.includes(speed) ? speed : (speedOptions[0] ?? 4),
  );
  const busy = $derived(projectStore.rendering || projectStore.burning);
  // Several discs share one bar: disc 2 of 3 runs from 33 % to 67 %.
  let partIndex = $state(1);
  let partCount = $state(1);
  const renderPercent = $derived(
    Math.min(100, Math.max(0, ((partIndex - 1) * 100 + projectStore.renderPercent) / Math.max(1, partCount))),
  );
  const burnPercent = $derived(
    Math.min(100, Math.max(0, projectStore.burnProgress?.percent ?? 0)),
  );
  const needsBurn = $derived(mode === "isoBurn" || mode === "existing");
  // What stops this mode from working at all, said before the user starts.
  const missingTools = $derived(projectStore.toolsReport?.missingRequired ?? []);
  const emptyMenu = $derived(
    project?.menus.find((menu) => !menu.buttons.some((button) => button.enabled)) ?? null,
  );
  const blocker = $derived(
    busy
      ? null
      : needsBurn && !effectiveDevice
        ? t("No hay ninguna grabadora conectada. Conecta una y pulsa «Actualizar», o elige «Solo ISO».")
        : mode !== "existing" && projectStore.planError && localizeProjectErrors(projectStore.planError)
          ? localizeProjectErrors(projectStore.planError)
        : mode !== "existing" && emptyMenu
          ? emptyMenu.id === project?.rootMenu
            ? t("El menú principal «{name}» no tiene botones: añade al menos uno (o aplica una plantilla) antes de generar el disco.", {
                name: emptyMenu.name,
              })
            : t("El menú «{name}» no tiene botones: añade al menos uno o bórralo antes de generar el disco.", {
                name: emptyMenu.name,
              })
        : mode !== "existing" && missingTools.length > 0
          ? t("Faltan herramientas para generar el disco ({tools}). Instálalas desde «Disco › Herramientas de disco…».", {
              tools: missingTools.join(", "),
            })
          : null,
  );
  const canStart = $derived(
    !busy &&
      // Between copies only «Continuar» / «Detener» move things on.
      !waitingIso &&
      !blocker &&
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
          : mode === "folder"
            ? t("Generar carpeta")
            : t("Generar ISO"),
  );
  const modes: Array<{
    id: ProductionMode;
    title: string;
    hint: string;
    icon: string;
  }> = [
    { id: "iso", title: "Solo ISO", hint: "Genera la imagen para guardarla o copiarla", icon: "◉" },
    { id: "folder", title: "Carpeta del disco", hint: "VIDEO_TS o BDMV para reproductores, USB o Kodi", icon: "▦" },
    { id: "isoBurn", title: "ISO + grabar", hint: "Genera y envía el disco en un flujo", icon: "▷" },
    { id: "existing", title: "ISO existente", hint: "Graba una imagen ya generada", icon: "▤" },
  ];

  function verdictLabel(verdict: string): string {
    if (verdict === "pass") return t("correcta");
    if (verdict === "warn") return t("con avisos");
    if (verdict === "fail") return t("con errores");
    return verdict;
  }

  // Another mode starts clean: an earlier result ("Disc folder ready") does
  // not stay under "ISO only".
  let shownMode: ProductionMode | null = null;
  $effect(() => {
    const current = mode;
    if (shownMode !== null && shownMode !== current && !busy) {
      // Bar, log, result and errors all start over.
      projectStore.resetRender();
      projectStore.lastIso = null;
      producedList = [];
      partIndex = 1;
      partCount = 1;
      seriesTotal = 1;
      seriesDone = 0;
      waitingIso = null;
    }
    shownMode = current;
  });

  // Some steps (menus, subtitles) report no progress of their own: the
  // bar pulses and the elapsed time keeps moving, so it never looks frozen.
  let now = $state(Date.now());
  let startedAt = 0;
  let lastPercent = -1;
  let lastChange = 0;
  $effect(() => {
    if (!busy) {
      startedAt = 0;
      return;
    }
    startedAt ||= Date.now();
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  $effect(() => {
    const percent = projectStore.burning ? burnPercent : renderPercent;
    if (percent !== lastPercent) {
      lastPercent = percent;
      lastChange = Date.now();
    }
  });
  const stalled = $derived(busy && now - lastChange > 4000);
  const elapsedLabel = $derived.by(() => {
    // Read the clock first: returning before it would drop the dependency
    // and the label would never update (QA 1.1.3).
    const current = now;
    if (!busy || !startedAt) return "";
    const seconds = Math.max(0, Math.floor((current - startedAt) / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  });

  // The same check run on several files (one per playlist, say) is one
  // line with a count; the technical details stay in the tooltip.
  const checkRows = $derived.by(() => {
    const rows: Array<{ key: string; check: { id: string; status: string; message: string }; count: number; details: string }> = [];
    for (const check of projectStore.renderVerification?.checks ?? []) {
      const key = `${check.id}:${check.status}`;
      const row = rows.find((item) => item.key === key);
      if (row) {
        row.count += 1;
        row.details += ` · ${check.message}`;
      } else {
        rows.push({ key, check, count: 1, details: `${check.id}: ${check.message}` });
      }
    }
    return rows;
  });

  // A folder typed or chosen is remembered even if nothing is produced.
  $effect(() => {
    const folder = outputDir.trim();
    if (defaultsReady && folder && isAbsolutePath(folder)) rememberOutput(nativePath(folder));
  });

  onMount(() => {
    // The plan (and its warnings, such as episodes without a button) is
    // ready before anything is produced.
    // Always recomputed: menus may have changed since the last plan (a
    // deleted button, say).
    if (projectStore.project?.episodes.length && !projectStore.rendering) void projectStore.calculatePlan();
    // A ready image handed over (data disc): straight to "Existing ISO".
    if (ui.burnIso) {
      mode = "existing";
      existingIso = ui.burnIso;
      ui.burnIso = null;
    }
    // The dialog opens clean: an earlier result or error is in the job
    // history (Disc › Job history…), not shown again here.
    if (!projectStore.rendering && !projectStore.burning) {
      projectStore.resetRender();
      projectStore.lastIso = null;
    }
    void prepareDefaults();
  });

  async function prepareDefaults() {
    if (!outputDir) outputDir = rememberedOutput();
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
    if (typeof picked === "string") {
      outputDir = picked;
      rememberOutput(nativePath(picked));
    }
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

  /** Runs the production, then announces the end (sound, taskbar, shutdown). */
  async function start() {
    if (busy || waitingIso) return;
    await run();
    // Between copies nothing has ended yet: no chime, no shutdown.
    if (waitingIso) return;
    await announceEnd();
  }

  async function announceEnd() {
    const failed =
      cancelRequested || Boolean(projectStore.renderError || projectStore.burnError || localError);
    await announceFinished(!failed);
  }

  async function run() {
    localError = null;
    cancelRequested = false;
    seriesTotal = 1;
    seriesDone = 0;
    waitingIso = null;
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
    // "C:Discos" or "Discos" would land in whatever folder the app started in.
    if (!isAbsolutePath(outputDir.trim())) {
      localError = t("Escribe la ruta completa de la carpeta de salida o elígela con «Elegir…».");
      return;
    }
    projectStore.resetRender();
    partIndex = 1;
    partCount = 1;
    projectStore.lastIso = null;
    producedList = [];
    projectStore.rendering = true;
    const root = nativePath(outputDir.trim()).replace(/[\\/]+$/, "");
    rememberOutput(root);
    try {
      // One project, or one per disc when splitting.
      let parts: Array<{ project: typeof current; fileName: string; index: number; count: number }> = [
        { project: current, fileName: "", index: 1, count: 1 },
      ];
      if (discSplit !== "single" && mode !== "isoBurn") {
        const split = await unwrap(commands.projectSplit(current, discSplit));
        parts = split.parts.map((part) => ({
          // The engine names parts "… - Disco N"; use the interface language.
          project: {
            ...(part.project as typeof current),
            name:
              part.count > 1
                ? `${current.name} - ${t("Disco {index}", { index: part.index })}`
                : part.project.name,
          },
          fileName: part.fileName,
          index: part.index,
          count: part.count,
        }));
        for (const diagnostic of split.diagnostics) {
          projectStore.renderLogs = [
            ...projectStore.renderLogs,
            `${diagnostic.severity === "info" ? "ℹ" : "⚠"} ${localizeDiagnostic(diagnostic)}`,
          ].slice(-300);
        }
      }
      const outputs: string[] = [];
      let elapsedMs = 0;
      for (const part of parts) {
        if (cancelRequested) break;
        const multi = part.count > 1;
        const prefix = multi ? t("Disco {index} de {count}", { index: part.index, count: part.count }) + " · " : "";
        projectStore.renderPercent = 0;
        partIndex = part.index;
        partCount = part.count;
        projectStore.renderStage = prefix + t("Componiendo menús");
        const warnings = await compositeProjectMenus(part.project, {
          backgroundColor: projectStore.menuBackgroundColor,
          previewUrls: projectStore.previewUrls,
          framePaths: projectStore.framePaths,
        });
        for (const warning of warnings) {
          projectStore.renderLogs = [...projectStore.renderLogs, `⚠ ${warning}`].slice(-300);
        }
        if (cancelRequested) break;
        // The image is named after the project ("Boda.iso", "Serie - Disc 2.iso");
        // the render keeps its work files in a folder it removes at the end.
        const baseName = safeFileName(current.name, t("Disco"));
        const partName = multi
          ? `${baseName} - ${t("Disco {index}", { index: part.index })}`
          : baseName;
        const channel = new Channel<JobEvent_Deserialize>();
        channel.onmessage = (event) => {
          projectStore.applyJobEvent(event);
          if (multi && projectStore.renderStage && !projectStore.renderStage.startsWith(prefix)) {
            projectStore.renderStage = prefix + projectStore.renderStage;
          }
        };
        // A disc folder only needs the image for a moment: it gets a temporary
        // name, so an "<name>.iso" made earlier is neither replaced nor removed.
        const isoName = mode === "folder" ? `${partName}.kinodisc-tmp.iso` : `${partName}.iso`;
        const outcome = await unwrap(
          commands.renderStart(part.project, root, hardwareEncoding, isoName, channel),
        );
        elapsedMs += outcome.elapsedMs;
        projectStore.renderVerification = outcome.verification ?? null;
        let produced = outcome.isoPath;
        if (mode === "folder") {
          projectStore.renderStage = prefix + t("Creando la carpeta del disco");
          const exported = await unwrap(
            commands.renderExportFolder(outcome.isoPath, root, partName, true, outcome.jobId),
          );
          produced = exported.folder;
        }
        outputs.push(produced);
        producedList = [...outputs];
        projectStore.lastIso = produced;
        // The engine's own summary names its (already removed) work folder.
        projectStore.renderSummary = t("Listo: {path}", { path: produced });
      }
      if (cancelRequested) {
        projectStore.renderStage = t("Cancelado");
        projectStore.renderSummary = t("Generación cancelada.");
        return;
      }
      projectStore.renderPercent = 100;
      projectStore.renderStage = t("Listo");
      const seconds = formatDecimal(elapsedMs / 1000, 1);
      if (outputs.length > 1) {
        projectStore.renderSummary = t("{count} discos generados en {seconds} s", { count: outputs.length, seconds });
      } else if (mode === "folder") {
        projectStore.renderSummary = t("Carpeta del disco lista en {seconds} s", { seconds });
      } else if (!projectStore.renderSummary) {
        projectStore.renderSummary = t("ISO generado en {seconds} s", { seconds });
      }
      if (mode === "isoBurn" && outputs[0]) {
        if (!effectiveDevice) {
          localError = t("El ISO está listo, pero no hay una unidad seleccionada para grabarlo.");
          return;
        }
        projectStore.rendering = false;
        await burnIso(outputs[0]);
      }
    } catch (cause) {
      if (cancelRequested) {
        projectStore.renderError = null;
        projectStore.renderStage = t("Cancelado");
        projectStore.renderSummary = t("Generación cancelada.");
      } else {
        // The error is the result: no "ISO created" or verification left
        // over from an earlier disc of the same run.
        projectStore.renderSummary = null;
        projectStore.renderVerification = null;
        projectStore.renderStage = t("Error");
        projectStore.renderError = localizeRenderError(messageOf(cause));
      }
    } finally {
      projectStore.rendering = false;
    }
  }

  /** Burns the first copy; the others wait for the user to swap the disc. */
  async function burnIso(isoPath: string) {
    seriesTotal = clampCopies(copies);
    seriesDone = 0;
    waitingIso = null;
    await burnCopy(isoPath);
  }

  /** Burns the next copy of the series and, if more remain, waits. */
  async function burnCopy(isoPath: string) {
    const last = seriesDone + 1 >= seriesTotal;
    // The disc comes out between copies even with "eject" off: the next
    // blank one has to go in.
    if (!(await burnOnce(isoPath, eject || !last))) return;
    seriesDone += 1;
    if (seriesDone < seriesTotal) waitingIso = isoPath;
  }

  /** «Continuar»: the new disc is in; burn the same image again. */
  async function continueSeries() {
    const isoPath = waitingIso;
    if (!isoPath || busy || refreshing) return;
    localError = null;
    cancelRequested = false;
    // The drive is asked again so the new disc is seen.
    await refreshDevices();
    if (waitingIso !== isoPath) return;
    if (!effectiveDevice) {
      // Still waiting: the user can reconnect the drive and try again.
      localError = t("No se detectó ninguna unidad grabable. Conecta una unidad y pulsa «Actualizar».");
      return;
    }
    waitingIso = null;
    await burnCopy(isoPath);
    if (!waitingIso) await announceEnd();
  }

  /** «Detener»: the copies already burned are the result. */
  function stopSeries() {
    if (busy) return;
    waitingIso = null;
    localError = null;
  }

  /** Burns one disc; true when it was written. */
  async function burnOnce(isoPath: string, ejectAfter: boolean): Promise<boolean> {
    localError = null;
    if (!isoPath) {
      localError = t("Elige un archivo ISO.");
      return false;
    }
    if (!effectiveDevice) {
      localError = t("No se detectó ninguna unidad grabable. Conecta una unidad y pulsa «Actualizar».");
      return false;
    }
    projectStore.burnError = null;
    projectStore.burnReport = null;
    projectStore.burnProgress = null;
    projectStore.burning = true;
    const channel = new Channel<BurnProgress_Deserialize>();
    channel.onmessage = (event) => projectStore.applyBurnProgress(event);
    try {
      const report = await unwrap(
        commands.burnStart(isoPath, effectiveDevice, effectiveSpeed, verify, ejectAfter, channel),
      );
      projectStore.burnReport = report;
      projectStore.lastIso = isoPath;
      return true;
    } catch (cause) {
      if (cancelRequested) {
        projectStore.burnError = null;
        projectStore.renderSummary = t("Grabación cancelada.");
      } else {
        projectStore.burnError = messageOf(cause);
      }
      return false;
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

<svelte:window onkeydown={(event) => { if (event.key === "Escape") close(); }} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar producción")}
    onclick={close}
    class="ui-backdrop"
  ></button>

  <div class="ui-dialog max-h-[92vh] w-full max-w-4xl" role="dialog" aria-modal="true" aria-labelledby="production-title">
    <header class="ui-dialog-header">
      <div>
        <h2 id="production-title" class="text-[16px] font-semibold text-slate-50">{t("Generar y grabar disco")}</h2>
        <p class="ui-hint mt-0.5">
          {isBluRay ? "Blu-ray HDMV" : isAvchd ? "AVCHD HD" : t("DVD-Video interactivo")} ·
          {project?.episodes.length ?? 0}
          {t((project?.episodes.length ?? 0) === 1 ? "episodio" : "episodios")}
        </p>
        {#if hardwareEncoder}
          <p class="mt-1 text-[11px] text-emerald-300/80">
            {t("Aceleración: {encoder} (fallback automático a CPU)", { encoder: hardwareEncoder })}
          </p>
        {:else if isAvchd && projectStore.toolsReport}
          <p class="mt-1 text-[11px] text-slate-500">
            {t("Codificación H.264 en CPU (libx264).")}
          </p>
        {/if}
      </div>
      <button
        type="button"
        onclick={close}
        disabled={busy}
        class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon"
        aria-label={t("Cerrar")}
      ><Icon name="close" size={15} /></button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if localError || projectStore.renderError || projectStore.burnError}
        <div class="mb-4 rounded-md bg-rose-500/10 px-3 py-2 text-[12px] leading-5 text-rose-200" role="alert">
          {localError ?? projectStore.renderError ?? projectStore.burnError}
          {#if projectStore.renderError}
            <p class="mt-1 text-[11px] text-rose-100/70">
              {t("Si vuelves a generar el disco, los episodios que ya estaban codificados se reutilizan: no se empieza de cero.")}
            </p>
          {/if}
        </div>
      {/if}

      <div class="grid gap-2 sm:grid-cols-4">
        {#each modes as item}
          <button
            type="button"
            onclick={() => (mode = item.id)}
            disabled={busy || Boolean(waitingIso)}
            aria-pressed={mode === item.id}
            class={`rounded-md border p-3 text-left disabled:opacity-50 ${
              mode === item.id ? "border-cyan-500 bg-cyan-500/10" : "border-slate-700 hover:border-slate-500"
            }`}
          >
            <Icon name={modeIcons[item.id]} size={20} class={mode === item.id ? "text-cyan-400" : "text-slate-400"} />
            <p class="mt-1.5 text-[13px] font-semibold text-slate-50">{t(item.title)}</p>
            <p class="mt-0.5 text-[12px] leading-5 text-slate-400">{t(item.hint)}</p>
          </button>
        {/each}
      </div>

      {#if mode === "iso" || mode === "folder"}
        <label class="mt-3 flex items-center justify-between gap-3 rounded-md border border-slate-700 p-3 text-[13px] text-slate-300">
          <span>
            <span class="font-semibold text-slate-100">{t("Discos")}</span>
            <span class="ui-label">{t("Si no entra todo, repártelo en varios discos o crea uno por episodio")}</span>
          </span>
          <select bind:value={discSplit} disabled={busy} class="ui-input ui-input-sm">
            <option value="single">{t("Un solo disco")}</option>
            <option value="fitToDisc">{t("Varios discos (llenar cada uno)")}</option>
            <option value="perEpisode">{t("Un disco por episodio")}</option>
          </select>
        </label>
      {/if}

      <label class="mt-3 flex items-center gap-2 text-[13px] text-slate-300">
        <input type="checkbox" class="ui-check" bind:checked={ui.shutdownWhenDone} disabled={busy && !ui.shutdownWhenDone} />
        {t("Apagar la computadora al terminar")}
      </label>

      {#if isAvchd && mode !== "existing"}
        <label class="mt-3 flex items-center justify-between gap-3 rounded-md border border-slate-700 px-3 py-2">
          <span class="text-[13px] text-slate-300">{t("Codificación H.264")}</span>
          <select
            class="ui-input ui-input-sm"
            bind:value={hardwareEncoding}
            disabled={busy}
          >
            <option value="auto">{t("Hardware si está disponible")}</option>
            <option value="cpuOnly">{t("Solo CPU (libx264)")}</option>
          </select>
        </label>
      {/if}

      <div class="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div class="min-w-0 space-y-3">
          {#if mode === "existing"}
            <div class="rounded-md border border-slate-700 p-3">
              <div class="flex items-center justify-between">
                <span class="ui-section-title">{t("Imagen ISO")}</span>
                <button
                  type="button"
                  onclick={chooseExistingIso}
                  disabled={busy}
                  class="ui-btn ui-btn-sm"
                >
                  {t("Elegir…")}
                </button>
              </div>
              <input
                class="ui-input mt-2 w-full font-mono"
                bind:value={existingIso}
                placeholder={t("/ruta/al/disco.iso")}
                disabled={busy}
              />
              {#if existingIso}
                <p class="mt-1 truncate text-[11px] text-slate-500">{basename(existingIso)}</p>
              {/if}
            </div>
          {:else}
            <div class="rounded-md border border-slate-700 p-3">
              <div class="flex items-center justify-between">
                <span class="ui-section-title">{t("Carpeta de salida")}</span>
                <button
                  type="button"
                  onclick={chooseOutput}
                  disabled={busy}
                  class="ui-btn ui-btn-sm"
                >
                  {t("Elegir…")}
                </button>
              </div>
              <input
                class="ui-input mt-2 w-full font-mono"
                bind:value={outputDir}
                disabled={busy}
              />
              <p class="mt-1 text-[11px] text-slate-500">
                {!defaultsReady
                  ? t("Preparando carpeta predeterminada…")
                  : mode === "folder"
                    ? t("La carpeta del disco se creará aquí con el nombre del proyecto.")
                    : discSplit !== "single" && mode === "iso"
                      ? t("Los ISO se escribirán aquí con el nombre del proyecto y el número de disco.")
                      : t("El ISO se escribirá aquí con el nombre del proyecto.")}
              </p>
            </div>
          {/if}

          <div class="rounded-md border border-slate-700 p-3">
            <div class="flex items-center justify-between">
              <span class="ui-section-title">
                {t("Progreso")}
              </span>
              <span class="font-mono text-[12px] text-slate-300">
                {projectStore.burning
                  ? formatPercent(burnPercent)
                  : formatPercent(renderPercent)}
                {#if busy && elapsedLabel}<span class="ml-1 text-slate-500">· {elapsedLabel}</span>{/if}
              </span>
            </div>
            <div class="ui-meter mt-2">
              <div
                class={`h-full rounded-full transition-all duration-300 ${
                  projectStore.renderError || projectStore.burnError
                    ? "bg-rose-500"
                    : "bg-cyan-500"
                } ${stalled ? "animate-pulse" : ""}`}
                style={`width: ${projectStore.burning ? burnPercent : renderPercent}%`}
              ></div>
            </div>
            <p class="mt-1.5 truncate text-[11px] text-slate-500">
              {#if projectStore.rendering}
                {projectStore.renderStage || t("Generando…")}
              {:else if projectStore.burning}
                {#if seriesTotal > 1}{t("Copia {index} de {count}", { index: seriesDone + 1, count: seriesTotal })} · {/if}{t("Grabando en")} {effectiveDevice || t("la unidad")} · {formatBytes(projectStore.burnProgress?.writtenBytes ?? 0)} {t("de")} {formatBytes(projectStore.burnProgress?.totalBytes ?? 0)}
              {:else if projectStore.renderSummary}
                {projectStore.renderSummary}
              {:else}
                {t("Listo para comenzar.")}
              {/if}
            </p>
            {#if projectStore.burnReport}
              <p class="mt-1 text-[11px] text-emerald-300">
                {t("Grabación completada")} · {projectStore.burnReport.backend} · {formatDecimal(projectStore.burnReport.elapsedMs / 1000, 1)} s
                {#if projectStore.burnReport.verified}· {t("verificada")}{/if}
              </p>
            {/if}
            {#if seriesTotal > 1 && !busy && !waitingIso}
              <!-- The series ended (all copies, «Detener» or an error). -->
              <p class={`mt-1 text-[11px] ${seriesDone === seriesTotal ? "text-emerald-300" : "text-amber-300"}`}>
                {t("Copias grabadas: {done} de {total}", { done: seriesDone, total: seriesTotal })}
              </p>
            {/if}
            {#if projectStore.renderVerification}
              <details class="mt-2 rounded-lg border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                <summary class="cursor-pointer select-none text-[11px] text-slate-400">
                  {t("Verificación estructural")} · {verdictLabel(projectStore.renderVerification.verdict)}
                </summary>
                <ul class="mt-1 max-h-32 space-y-0.5 overflow-y-auto font-mono text-[11px]">
                  {#each checkRows as row (row.key)}
                    <li class:text-rose-300={row.check.status === "fail"} class:text-amber-300={row.check.status === "warn"} class:text-emerald-300={row.check.status === "pass"} class:text-slate-500={row.check.status === "skipped"}>
                      <span title={row.details}>{localizeCheck(row.check)}{row.count > 1 ? ` ×${row.count}` : ""}</span>
                    </li>
                  {/each}
                </ul>
              </details>
            {/if}
            {#if projectStore.lastIso}
              <div class="mt-2 flex items-center justify-between gap-2 rounded-lg border border-slate-800 px-2 py-1.5">
                <span class="min-w-0 font-mono text-[11px] text-slate-400" title={producedList.join(" · ") || projectStore.lastIso}>
                  {#if producedList.length > 1}
                    <!-- Every disc of a multi-disc run, not only the last one. -->
                    {#each producedList as produced (produced)}
                      <span class="block truncate">{basename(produced)}</span>
                    {/each}
                  {:else}
                    <span class="block truncate">{basename(projectStore.lastIso)}</span>
                  {/if}
                </span>
                <button
                  type="button"
                  onclick={openOutput}
                  class="ui-btn ui-btn-sm shrink-0"
                >
                  {t("Abrir carpeta")}
                </button>
              </div>
            {/if}
          </div>

          {#if projectStore.renderLogs.length > 0}
            <details class="rounded-xl border border-slate-800 p-3">
              <summary class="ui-section-title cursor-pointer select-none">
                {t("Registro")} · {projectStore.renderLogs.length} {t("líneas")}
              </summary>
              <div class="mt-2 max-h-44 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950 p-2 font-mono text-[12px] leading-5 text-slate-500">
                {#each projectStore.renderLogs as line}
                  <p class="whitespace-pre-wrap">{line}</p>
                {/each}
              </div>
            </details>
          {/if}
        </div>

        <aside class="space-y-3">
          {#if needsBurn}
            <div class="rounded-md border border-slate-700 p-3">
              <div class="flex items-center justify-between">
                <span class="ui-section-title">{t("Unidad")}</span>
                <button
                  type="button"
                  onclick={refreshDevices}
                  disabled={refreshing || busy}
                  class="ui-btn ui-btn-sm"
                >
                  {refreshing ? "…" : t("Actualizar")}
                </button>
              </div>
              {#if devices.length === 0}
                <p class="mt-2 rounded-md bg-amber-500/10 px-2.5 py-1.5 text-[12px] leading-5 text-amber-200/80">
                  {t("No se detectaron unidades ópticas. Puedes generar el ISO y grabarlo después.")}
                </p>
              {:else}
                <select
                  class="ui-input mt-2 w-full"
                  value={effectiveDevice}
                  onchange={(event) => (selectedDevice = event.currentTarget.value)}
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

            <div class="rounded-md border border-slate-700 p-3">
              <label class="ui-label">
                <span class="ui-section-title mb-1 block">{t("Velocidad")}</span>
                <select
                  class="ui-input w-full"
                  value={effectiveSpeed}
                  onchange={(event) => (speed = Number(event.currentTarget.value))}
                  disabled={busy}
                >
                  {#each speedOptions as option}
                    <option value={option}>
                      {option}x{isBluRay ? ` · ${(option * 4.495).toFixed(1)} MB/s` : ""}
                    </option>
                  {/each}
                </select>
              </label>
              <label class="ui-label mt-2">
                <span class="ui-section-title mb-1 block">{t("Copias")}</span>
                <input
                  type="number"
                  min="1"
                  max={MAX_COPIES}
                  step="1"
                  class="ui-input w-full tabular-nums"
                  value={copies}
                  onchange={(event) => {
                    copies = clampCopies(event.currentTarget.valueAsNumber);
                    event.currentTarget.value = String(copies);
                  }}
                  disabled={busy || Boolean(waitingIso)}
                />
              </label>
              <label class="mt-2 flex items-center gap-1.5 text-[11px] text-slate-400">
                <input type="checkbox" bind:checked={verify} disabled={busy} class="ui-check accent-cyan-500" />
                {t("Verificar por lectura SHA-256")}
              </label>
              <label class="mt-1 flex items-center gap-1.5 text-[11px] text-slate-400">
                <input type="checkbox" bind:checked={eject} disabled={busy} class="ui-check accent-cyan-500" />
                {t("Expulsar al terminar")}
              </label>
            </div>
          {:else}
            <div class="rounded-md border border-slate-700 p-3">
              <p class="ui-section-title">{t("Salida")}</p>
              <p class="mt-1.5 text-[12px] leading-5 text-slate-500">
                {#if mode === "folder"}
                  {t("Se creará una carpeta con VIDEO_TS (DVD) o BDMV (Blu-ray), lista para copiarla a un USB, un reproductor multimedia o Kodi.")}
                {:else}
                  {t("Se generará un archivo")} <span class="font-mono text-slate-300">{t(".iso")}</span> {t("listo para grabarlo con cualquier herramienta del sistema.")}
                {/if}
              </p>
            </div>
          {/if}

          <div class="rounded-md border border-slate-700 p-3">
            <p class="ui-section-title">{t("Plan")}</p>
            {#if projectStore.plan}
              <div class="mt-1.5 flex items-center justify-between text-[11px]">
                <span class="text-slate-500">{t("Ocupación")}</span>
                <span class={projectStore.plan.feasible ? "font-mono text-emerald-300" : "font-mono text-rose-300"}>
                  {formatPercent(projectStore.planPercent)}
                </span>
              </div>
              <div class="mt-1 flex items-center justify-between text-[11px]">
                <span class="text-slate-500">{t("Objetivo de vídeo")}</span>
                <span class="font-mono text-slate-300">{projectStore.plan.videoKbps.toLocaleString(i18n.locale)} kbps</span>
              </div>
              <div class="mt-1 flex items-center justify-between text-[11px]">
                <span class="text-slate-500">{t("Calidad")}</span>
                <span class="text-slate-300">{projectStore.qualityLabel}</span>
              </div>
            {:else}
              <p class="mt-1.5 text-[12px] leading-5 text-slate-500">
                {t("El plan se calculará automáticamente al generar el disco.")}
              </p>
            {/if}
          </div>
          {#if unreachable.length > 0}
            <div class="rounded-md border border-amber-500/40 bg-amber-500/10 p-3" role="status">
              <p class="ui-section-title text-amber-300">{t("Revisa antes de generar")}</p>
              <ul class="mt-1.5 space-y-1 text-[12px] leading-5 text-slate-300">
                {#each unreachable as warning}
                  <li>{localizeDiagnostic(warning)}</li>
                {/each}
              </ul>
            </div>
          {/if}
        </aside>
      </div>
    </div>

    {#if blocker}
      <p class="border-t border-amber-500/30 bg-amber-500/10 px-5 py-2 text-[12px] text-amber-300" role="status">
        {blocker}
      </p>
    {/if}
    {#if waitingIso && !busy}
      <!-- Between copies: the dialog is idle until the disc is swapped. -->
      <div
        class="flex flex-wrap items-center justify-between gap-2 border-t border-cyan-500/30 bg-cyan-500/10 px-5 py-2.5"
        role="status"
      >
        <p class="min-w-0 flex-1 text-[12px] leading-5 text-cyan-200">
          {t("La copia {done} de {total} está lista. Inserta otro disco virgen y pulsa «Continuar».", {
            done: seriesDone,
            total: seriesTotal,
          })}
        </p>
        <div class="flex shrink-0 items-center gap-2">
          <button type="button" onclick={stopSeries} disabled={refreshing} class="ui-btn ui-btn-sm">
            {t("Detener")}
          </button>
          <button type="button" onclick={continueSeries} disabled={refreshing} class="ui-btn ui-btn-sm ui-btn-primary">
            {t("Continuar")}
          </button>
        </div>
      </div>
    {/if}
    <footer class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <div class="flex items-center gap-2">
        {#if busy}
          <button
            type="button"
            onclick={cancel}
            class="ui-btn ui-btn-danger"
          >
            {t("Cancelar")}
          </button>
          <span class="text-[11px] text-slate-500">
            {projectStore.rendering ? t("Generación en curso") : t("Grabación en curso")}
          </span>
        {:else}
          <button
            type="button"
            onclick={openOutput}
            class="ui-btn"
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
          class="ui-btn"
        >
          {t("Cerrar")}
        </button>
        <button
          type="button"
          onclick={start}
          disabled={!canStart}
          class="ui-btn ui-btn-primary"
        >
          {startLabel}
        </button>
      </div>
    </footer>
  </div>
</div>
