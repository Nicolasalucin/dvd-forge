<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { onDestroy, onMount } from "svelte";
  import { messageOf } from "../api/client";
  import { projectStore, type StudioMode } from "../stores/project.svelte";
  import JobHistory from "./JobHistory.svelte";
  import RecentProjects from "./RecentProjects.svelte";
  import ToolchainDoctor from "./ToolchainDoctor.svelte";
  import { i18n, LOCALES, t } from "../i18n/index.svelte";
  import { formatBytes, formatDuration } from "../utils/format";

  interface Props {
    onNewProject: () => void;
    onProduce: () => void;
  }

  let { onNewProject, onProduce }: Props = $props();

  let busy = $state(false);
  let localError = $state<string | null>(null);
  let fullscreen = $state(false);
  let lastPlanSignature = "";
  let planTimer: ReturnType<typeof setTimeout> | null = null;
  const appWindow = getCurrentWindow();
  const planSignature = $derived(projectStore.planSignature);

  $effect(() => {
    const signature = planSignature;
    if (!projectStore.hasProject || projectStore.episodes.length === 0) return;
    if (signature === lastPlanSignature) return;
    lastPlanSignature = signature;
    if (planTimer) clearTimeout(planTimer);
    planTimer = setTimeout(() => {
      planTimer = null;
      void projectStore.calculatePlan();
    }, 650);
  });

  onDestroy(() => {
    if (planTimer) clearTimeout(planTimer);
  });

  onMount(() => {
    let active = true;
    void appWindow
      .isFullscreen()
      .then((value) => {
        if (active) fullscreen = value;
      })
      .catch(() => undefined);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "F11") {
        event.preventDefault();
        void toggleFullscreen();
      } else if (event.key === "Escape" && fullscreen) {
        event.preventDefault();
        void appWindow
          .setFullscreen(false)
          .then(() => {
            fullscreen = false;
          })
          .catch(() => undefined);
      }
    };
    window.addEventListener("keydown", onKey);
    let unlisten: (() => void) | undefined;
    void appWindow
      .onResized(() => {
        void appWindow
          .isFullscreen()
          .then((value) => {
            if (active) fullscreen = value;
          })
          .catch(() => undefined);
      })
      .then((fn) => {
        if (active) {
          unlisten = fn;
        } else {
          fn();
        }
      })
      .catch(() => undefined);
    return () => {
      active = false;
      window.removeEventListener("keydown", onKey);
      unlisten?.();
    };
  });

  async function toggleFullscreen() {
    try {
      const isFull = await appWindow.isFullscreen();
      await appWindow.setFullscreen(!isFull);
      fullscreen = !isFull;
    } catch (cause) {
      localError = messageOf(cause);
    }
  }

  let recentOpen = $state(false);
  let historyOpen = $state(false);
  let doctorOpen = $state(false);
  const project = $derived(projectStore.project);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const isBluRay = $derived(
    project?.disc.capacity === "bd25" || project?.disc.capacity === "bd50",
  );
  const plan = $derived(projectStore.plan);
  const capacity = $derived(
    plan?.capacityBytes ??
      (project?.disc.capacity === "bd25"
        ? 25_000_000_000
        : project?.disc.capacity === "bd50"
          ? 50_000_000_000
          : project?.disc.capacity === "dvd9"
            ? 8_540_000_000
            : 4_700_000_000),
  );
  const fill = $derived(Math.min(100, Math.max(0, projectStore.planPercent)));
  const canProduce = $derived(Boolean(project));
  const layerBreakPercent = $derived(
    plan?.layerBreakSector !== undefined && plan?.layerBreakSector !== null
      ? Math.min(100, (plan.layerBreakSector * 2048 * 100) / Math.max(1, capacity))
      : null,
  );
  const modes = $derived([
    { id: "menu" as StudioMode, icon: "◈", label: "Menú", hint: "Diseño WYSIWYG HDMV", disabled: false },
    { id: "trim" as StudioMode, icon: "✂", label: "Edición", hint: "Recorte y capítulos", disabled: false },
    { id: "simulator" as StudioMode, icon: "▷", label: "Simulador", hint: "Mando y navegación real", disabled: false },
    { id: "vmMap" as StudioMode, icon: "⌘", label: "Mapa VM", hint: "Flujo de navegación del DVD", disabled: false },
    { id: "tracks" as StudioMode, icon: "♫", label: "Pistas", hint: "Audio, subtítulos y desfases", disabled: false },
    { id: "motion" as StudioMode, icon: "◉", label: "Animación", hint: "Menús y botones de vídeo", disabled: false },
    { id: "vbr" as StudioMode, icon: "▤", label: "Calidad VBR", hint: "Curva, prioridad y capacidad", disabled: false },
  ]);

  function plural(count: number, singular: string, pluralForm: string): string {
    return `${count} ${t(count === 1 ? singular : pluralForm)}`;
  }

  async function openProject() {
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Proyecto de DVD Forge"), extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    busy = true;
    localError = null;
    try {
      await projectStore.openProject(picked);
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  async function saveProject() {
    const current = projectStore.project;
    if (!current) return;
    const path = await save({
      defaultPath: `${current.name.replaceAll(" ", "-").toLowerCase()}.json`,
      filters: [{ name: t("Proyecto de DVD Forge"), extensions: ["json"] }],
    });
    if (!path) return;
    busy = true;
    localError = null;
    try {
      await projectStore.saveProject(path);
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  function selectMode(mode: (typeof modes)[number]) {
    if (mode.disabled) return;
    projectStore.studioMode = mode.id;
  }

  function rename(value: string) {
    projectStore.renameProject(value);
  }
</script>

<header class="shrink-0 border-b border-slate-800/80 bg-slate-950/90 backdrop-blur">
  <div class="flex flex-wrap items-center gap-3 px-4 py-2.5">
    <div class="flex items-center gap-2.5">
      <div class="flex h-9 w-9 items-center justify-center rounded-xl bg-gradient-to-br from-cyan-400 to-sky-600 text-sm font-black text-slate-950">
        {t("DF")}
      </div>
      <div class="min-w-0">
        {#if project}
          <input
            class="w-[190px] truncate rounded-md border border-transparent bg-transparent px-0 py-0 text-sm font-semibold tracking-tight text-white outline-none transition hover:border-slate-800 hover:px-1 focus:border-cyan-500/60 focus:px-1"
            value={project.name}
            aria-label={t("Nombre del proyecto")}
            oninput={(event) => rename(event.currentTarget.value)}
          />
          <p class="text-[10px] uppercase tracking-[0.18em] text-slate-500">
            {t(isBluRay ? "Blu-ray interactivo" : isAvchd ? "AVCHD HD interactivo" : "DVD-Video interactivo")}
            · {project.disc.capacity === "bd25"
              ? "BD-25"
              : project.disc.capacity === "bd50"
                ? "BD-50"
                : project.disc.capacity === "dvd9"
                  ? "DVD-9"
                  : "DVD-5"}
          </p>
        {:else}
          <p class="truncate text-sm font-semibold tracking-tight text-white">{t("DVD Forge Studio")}</p>
          <p class="text-[10px] uppercase tracking-[0.18em] text-slate-500">{t("Sin proyecto")}</p>
        {/if}
      </div>
    </div>

    <div class="mx-1 hidden h-8 w-px bg-slate-800 sm:block"></div>

    <div class="flex flex-wrap items-center gap-1.5">
      <button
        type="button"
        onclick={onNewProject}
        class="rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
      >
        {t("Nuevo")}
      </button>
      <button
        type="button"
        onclick={openProject}
        disabled={busy}
        class="rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-50"
      >
        {t("Abrir")}
      </button>
      <button
        type="button"
        onclick={saveProject}
        disabled={busy || !project}
        class="rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-50"
      >
        {t("Guardar")}
      </button>
      <div class="relative">
        <button
          type="button"
          onclick={() => {
            recentOpen = !recentOpen;
            historyOpen = false;
            if (recentOpen) void projectStore.refreshRecentProjects();
          }}
          class="rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
        >
          {t("Recientes")}
        </button>
        {#if recentOpen}
          <div class="absolute left-0 top-full z-50 mt-1 w-80 rounded-xl border border-slate-700/80 bg-slate-950/95 p-2 shadow-2xl shadow-black/60 backdrop-blur">
            <p class="mb-1.5 px-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-500">
              {t("Proyectos recientes")}
            </p>
            <div class="max-h-72 overflow-y-auto pr-0.5">
              <RecentProjects onOpened={() => (recentOpen = false)} />
            </div>
          </div>
        {/if}
      </div>
      <div class="relative">
        <button
          type="button"
          onclick={() => {
            historyOpen = !historyOpen;
            recentOpen = false;
            if (historyOpen) void projectStore.refreshJobHistory();
          }}
          class="rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] font-medium text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
        >
          {t("Historial")}
        </button>
        {#if historyOpen}
          <div class="absolute left-0 top-full z-50 mt-1 w-80 rounded-xl border border-slate-700/80 bg-slate-950/95 p-2 shadow-2xl shadow-black/60 backdrop-blur">
            <div class="mb-1.5 flex items-center justify-between px-1">
              <p class="text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-500">
                {t("Historial de trabajos")}
              </p>
              <button
                type="button"
                onclick={() => void projectStore.refreshJobHistory()}
                class="text-[9px] text-slate-500 transition hover:text-cyan-200"
              >
                {t("Actualizar")}
              </button>
            </div>
            <div class="max-h-80 overflow-y-auto pr-0.5">
              <JobHistory />
            </div>
          </div>
        {/if}
      </div>
    </div>

    <div class="ml-auto flex items-center gap-2">
      {#if project}
        <div class="hidden items-center gap-1.5 text-[10px] text-slate-500 xl:flex">
          <span class="rounded-md border border-slate-800 px-1.5 py-0.5">
            {project.disc.standard === "pal" ? "PAL" : "NTSC"}
          </span>
          <span class="rounded-md border border-slate-800 px-1.5 py-0.5">
            {project.disc.aspect === "sixteenNine" ? "16:9" : "4:3"}
          </span>
        </div>
      {/if}
      <label class="flex h-9 items-center gap-1 rounded-xl border border-slate-800 px-2 text-[10px] text-slate-300" title={t("Idioma / Language")}>
        <span aria-hidden="true">🌐</span>
        <select
          class="bg-transparent text-[10px] font-semibold uppercase text-slate-300 outline-none"
          value={i18n.locale}
          onchange={(event) => i18n.setLocale(event.currentTarget.value as typeof i18n.locale)}
          aria-label={t("Idioma de la interfaz")}
        >
          {#each LOCALES as locale}
            <option value={locale.id}>{locale.label}</option>
          {/each}
        </select>
      </label>
      <button
        type="button"
        onclick={() => (doctorOpen = true)}
        title={t("Diagnóstico de toolchain")}
        class="flex h-9 items-center gap-1.5 rounded-xl border border-slate-800 px-2.5 text-[10px] font-semibold text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
      >
        <span class="text-sm">⚕</span>
        <span class="hidden sm:inline">{t("Salud")}</span>
      </button>
      <button
        type="button"
        onclick={toggleFullscreen}
        title={fullscreen ? t("Salir de pantalla completa (F11)") : t("Pantalla completa (F11)")}
        class="flex h-9 w-9 items-center justify-center rounded-xl border border-slate-800 text-sm text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
      >
        {fullscreen ? "⤡" : "⛶"}
      </button>
      <button
        type="button"
        onclick={onProduce}
        disabled={!canProduce}
        class="rounded-xl bg-cyan-500 px-3.5 py-2 text-xs font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
      >
        {t("Producir disco")}
      </button>
    </div>
  </div>

  <div class="flex flex-wrap items-center gap-2 border-t border-slate-800/60 px-4 py-1.5">
    <div class="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
      {#each modes as mode}
        <button
          type="button"
          onclick={() => selectMode(mode)}
          disabled={mode.disabled}
          title={t(mode.hint)}
          class={`flex shrink-0 items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-[11px] font-semibold transition ${
            projectStore.studioMode === mode.id
              ? "bg-cyan-500/15 text-cyan-200 shadow-sm shadow-cyan-500/10"
              : "text-slate-400 hover:bg-slate-900 hover:text-slate-200"
          } ${mode.disabled ? "cursor-not-allowed opacity-35" : ""}`}
        >
          <span class="text-[12px]">{mode.icon}</span>
          <span>{t(mode.label)}</span>
        </button>
      {/each}
    </div>

    {#if project && !isAvchd}
      <div class="flex shrink-0 items-center gap-1.5">
        <select
          class="rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
          value={project.disc.standard}
          onchange={(event) =>
            projectStore.setStandard(event.currentTarget.value as "ntsc" | "pal")}
          title={t("Estándar de televisión")}
        >
          <option value="ntsc">{t("NTSC")}</option>
          <option value="pal">{t("PAL")}</option>
        </select>
        <select
          class="rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
          value={project.disc.aspect}
          onchange={(event) =>
            projectStore.setAspect(event.currentTarget.value as "fourThree" | "sixteenNine")}
          title={t("Relación de aspecto")}
        >
          <option value="fourThree">4:3</option>
          <option value="sixteenNine">16:9</option>
        </select>
      </div>
    {/if}

    {#if project}
      {#if project.disc.mode === "dvdVideo"}
        <select
          class="shrink-0 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
          value={project.disc.regionMask}
          onchange={(event) => projectStore.setRegionMask(Number(event.currentTarget.value))}
          title={t("Región DVD")}
        >
          <option value={0}>{t("Región libre")}</option>
          <option value={1}>{t("Región 1")}</option>
          <option value={2}>{t("Región 2")}</option>
          <option value={4}>{t("Región 3")}</option>
          <option value={8}>{t("Región 4")}</option>
          <option value={16}>{t("Región 5")}</option>
          <option value={32}>{t("Región 6")}</option>
        </select>
      {/if}
      <select
        class="shrink-0 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
        value={project.disc.capacity}
        onchange={(event) =>
          projectStore.setDiscCapacity(
            event.currentTarget.value as "dvd5" | "dvd9" | "bd25" | "bd50",
          )}
        title={t("Capacidad del disco")}
      >
        <option value="dvd5">{t("DVD-5 · 4,7 GB")}</option>
        <option value="dvd9">{t("DVD-9 · 8,5 GB")}</option>
        <option value="bd25">{t("BD-25 · 25 GB")}</option>
        <option value="bd50">{t("BD-50 · 50 GB")}</option>
      </select>
      {#if project.disc.capacity === "dvd5" || project.disc.capacity === "dvd9"}
        <select
          class="shrink-0 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
          value={project.disc.mode}
          onchange={(event) =>
            projectStore.setDiscMode(event.currentTarget.value as "dvdVideo" | "avchd")}
          title={t("Formato de autoría")}
        >
          <option value="dvdVideo">{t("DVD-Video")}</option>
          <option value="avchd">{t("AVCHD")}</option>
        </select>
      {/if}
    {/if}
  </div>

  <div class="flex flex-wrap items-center gap-3 border-t border-slate-800/60 px-4 py-1.5">
    <div class="flex min-w-0 flex-1 items-center gap-3">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Capacidad")}</span>
      <div class="relative h-2.5 min-w-[120px] max-w-md flex-1 overflow-visible rounded-full border border-slate-800 bg-slate-900">
        <div
          class={`h-full rounded-full transition-all duration-500 ${
            plan && !plan.feasible
              ? "bg-gradient-to-r from-rose-500 to-orange-400"
              : "bg-gradient-to-r from-cyan-500 via-sky-400 to-emerald-400"
          }`}
          style={`width: ${fill}%`}
        ></div>
        {#if layerBreakPercent !== null}
          <span
            class="absolute -top-1 h-4 w-0.5 bg-amber-300 shadow-[0_0_6px_rgba(252,211,77,0.8)]"
            style={`left: ${layerBreakPercent}%`}
            title={`Corte de capa ${plan?.layerBreakMode?.toUpperCase() ?? ""} · sector ${plan?.layerBreakSector}`}
          ></span>
        {/if}
      </div>
      <span class="shrink-0 font-mono text-[11px] text-cyan-300">
        {plan ? `${fill.toFixed(1)}%` : "—"}
      </span>
      <span class="hidden shrink-0 text-[11px] text-slate-500 sm:inline">
        {projectStore.planBusy
          ? t("Calculando plan…")
          : plan
            ? `${formatBytes(projectStore.planUsedBytes)} ${t("de")} ${formatBytes(capacity)} · ${projectStore.qualityLabel}`
            : project && project.episodes.length > 0
              ? t("El plan se actualiza automáticamente")
              : t("Añade episodios para calcular el plan")}
      </span>
      {#if layerBreakPercent !== null}
        <span class="hidden shrink-0 rounded-md border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-[9px] text-amber-200 lg:inline">
          {t("Corte de capa")} {plan?.layerBreakMode?.toUpperCase()} · {t("sector")} {plan?.layerBreakSector}
        </span>
      {/if}
    </div>
    <div class="flex shrink-0 items-center gap-3 text-[10px] text-slate-500">
      <span>{plural(project?.episodes.length ?? 0, "episodio", "episodios")}</span>
      <span>{plural(project?.menus.length ?? 0, "menú", "menús")}</span>
      {#if plan}
        <span>{formatDuration(plan.totalDurationMs)}</span>
      {/if}
      {#if projectStore.lastIso}
        <span class="max-w-[180px] truncate font-mono text-emerald-400/80" title={projectStore.lastIso}>
          {t("ISO listo")}
        </span>
      {/if}
    </div>
  </div>

  {#if doctorOpen}
    <ToolchainDoctor onClose={() => (doctorOpen = false)} />
  {/if}

  {#if localError || projectStore.error}
    <div class="flex items-start justify-between gap-3 border-t border-rose-500/20 bg-rose-500/10 px-4 py-1.5 text-[11px] text-rose-200">
      <span class="truncate">{localError ?? projectStore.error}</span>
      <button
        type="button"
        onclick={() => {
          localError = null;
          projectStore.error = null;
        }}
        class="shrink-0 text-rose-300/70 hover:text-rose-100"
      >
        ✕
      </button>
    </div>
  {/if}
</header>
