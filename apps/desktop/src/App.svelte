<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { commands, type AutosaveEntry } from "./lib/api/bindings";
  import { messageOf } from "./lib/api/client";
  import * as actions from "./lib/actions";
  import { i18n, t } from "./lib/i18n/index.svelte";
  import Inspector from "./lib/components/Inspector.svelte";
  import MediaPanel from "./lib/components/MediaPanel.svelte";
  import NewProjectModal from "./lib/components/NewProjectModal.svelte";
  import StatusBar from "./lib/components/StatusBar.svelte";
  import StoryboardBar from "./lib/components/StoryboardBar.svelte";
  import StudioHeader from "./lib/components/StudioHeader.svelte";
  import Welcome from "./lib/components/Welcome.svelte";
  import Lazy from "./lib/components/Lazy.svelte";
  import { projectStore } from "./lib/stores/project.svelte";
  import { handleShortcut, refreshAppMenu } from "./lib/ui/appMenu";
  import Icon from "./lib/ui/Icon.svelte";
  import { applyTheme, ui } from "./lib/ui/state.svelte";
  import { cancelShutdown } from "./lib/ui/finish";
  import { checkForUpdates } from "./lib/ui/updates";

  // Screens and dialogs that are not visible at startup load on first use.
  const screens = {
    menu: () => import("./lib/components/MenuWorkspace.svelte"),
    trim: () => import("./lib/components/TrimmerStudio.svelte"),
    simulator: () => import("./lib/components/Simulator.svelte"),
    vmMap: () => import("./lib/components/VmMap.svelte"),
    tracks: () => import("./lib/components/TracksPanel.svelte"),
    motion: () => import("./lib/components/MotionPanel.svelte"),
    vbr: () => import("./lib/components/VbrPanel.svelte"),
    production: () => import("./lib/components/ProductionModal.svelte"),
    guide: () => import("./lib/components/GuideTour.svelte"),
    about: () => import("./lib/components/AboutDialog.svelte"),
    slideshow: () => import("./lib/components/SlideshowDialog.svelte"),
    inspector: () => import("./lib/components/DiscInspector.svelte"),
  };

  // Walkthrough builds only (VITE_WALKTHROUGH=1 at build time; never set by
  // release CI): lets the test driver do what a native file dialog would.
  if (import.meta.env.VITE_WALKTHROUGH === "1") {
    (window as unknown as Record<string, unknown>).__kd = { projectStore, actions, ui };
  }

  let recovery = $state<AutosaveEntry[]>([]);
  let dragging = $state(false);

  // The menu bar mirrors language, theme, expert mode, recent files and
  // undo availability.
  $effect(() => {
    void [
      i18n.locale,
      ui.theme,
      projectStore.expertMode,
      projectStore.recentProjects.length,
      Boolean(projectStore.project),
      projectStore.canUndo,
      projectStore.canRedo,
    ];
    refreshAppMenu();
  });

  onMount(() => {
    applyTheme(ui.theme);
    // Walkthrough/test builds never call the network.
    if (import.meta.env.VITE_WALKTHROUGH !== "1") void checkForUpdates();
    projectStore.startAutosave();
    void projectStore.refreshToolsDoctor();
    void (async () => {
      await projectStore.refreshRecentProjects();
      try {
        const startupProject = await commands.startupProjectPath();
        if (startupProject) {
          await projectStore.openProject(startupProject);
          return;
        }
        recovery = await commands.autosaveList();
      } catch (cause) {
        projectStore.error = messageOf(cause);
      }
    })();

    const onKey = (event: KeyboardEvent) => {
      if (handleShortcut(event)) event.preventDefault();
    };
    window.addEventListener("keydown", onKey);

    // Drop files anywhere: projects open, media is imported.
    const cleanups: Array<() => void> = [];
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter" || payload.type === "over") dragging = true;
        else if (payload.type === "leave") dragging = false;
        else if (payload.type === "drop") {
          dragging = false;
          const paths = payload.paths;
          const project = paths.find((path) => /\.(json|kinodisc|dvdforge-bundle)$/i.test(path));
          if (project && paths.length === 1) void actions.openPath(project);
          else if (projectStore.project) void actions.importMedia(paths);
          else ui.notice = t("Crea o abre un proyecto para añadir vídeos.");
        }
      })
      .then((unlisten) => cleanups.push(unlisten))
      .catch(() => undefined);

    // Closing the window with unsaved changes asks first.
    void getCurrentWindow()
      .onCloseRequested(async (event) => {
        if (!(await projectStore.confirmLoseChanges())) event.preventDefault();
      })
      .then((unlisten) => cleanups.push(unlisten))
      .catch(() => undefined);

    return () => {
      window.removeEventListener("keydown", onKey);
      for (const cleanup of cleanups) cleanup();
    };
  });

  function recover(entry: AutosaveEntry) {
    projectStore.recoverAutosave(entry);
    recovery = [];
  }

  async function discardRecovery() {
    for (const entry of recovery) await projectStore.discardAutosave(entry.projectId);
    recovery = [];
  }

  function savedAt(value: string): string {
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? value
      : date.toLocaleString(i18n.locale, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  }
</script>

<div class="flex h-screen min-h-0 flex-col overflow-hidden bg-slate-950 text-slate-100">
  <StudioHeader />

  {#if !projectStore.project}
    <Welcome />
  {:else}
    <div class="flex min-h-0 flex-1">
      {#if projectStore.leftCollapsed}
        <button
          type="button"
          onclick={() => (projectStore.leftCollapsed = false)}
          class="flex w-8 shrink-0 flex-col items-center justify-center gap-2 border-r border-slate-800 bg-slate-900 text-slate-400 hover:text-slate-100"
          title={t("Mostrar biblioteca")}
        >
          <Icon name="chevronRight" size={14} />
          <span class="text-[12px] [writing-mode:vertical-rl]">{t("Medios")}</span>
        </button>
      {:else}
        <MediaPanel />
      {/if}

      <main class="flex min-h-0 min-w-0 flex-1 flex-col">
        <div class="flex min-h-0 flex-1">
          {#if projectStore.studioMode === "menu"}
            <Lazy loader={screens.menu} />
          {:else if projectStore.studioMode === "trim"}
            <Lazy loader={screens.trim} />
          {:else if projectStore.studioMode === "simulator"}
            <Lazy loader={screens.simulator} />
          {:else if projectStore.studioMode === "vmMap"}
            <Lazy loader={screens.vmMap} />
          {:else if projectStore.studioMode === "tracks"}
            <Lazy loader={screens.tracks} />
          {:else if projectStore.studioMode === "motion"}
            <Lazy loader={screens.motion} />
          {:else if projectStore.studioMode === "vbr"}
            <Lazy loader={screens.vbr} />
          {/if}
        </div>
        <StoryboardBar />
      </main>

      {#if projectStore.rightCollapsed}
        <button
          type="button"
          onclick={() => (projectStore.rightCollapsed = false)}
          class="flex w-8 shrink-0 flex-col items-center justify-center gap-2 border-l border-slate-800 bg-slate-900 text-slate-400 hover:text-slate-100"
          title={t("Mostrar propiedades")}
        >
          <span class="rotate-180"><Icon name="chevronRight" size={14} /></span>
          <span class="text-[12px] [writing-mode:vertical-rl]">{t("Propiedades")}</span>
        </button>
      {:else}
        <Inspector />
      {/if}
    </div>
  {/if}

  <StatusBar />

  {#if dragging}
    <div class="pointer-events-none fixed inset-2 z-[60] flex items-center justify-center rounded-lg border-2 border-dashed border-cyan-500 bg-cyan-500/10">
      <p class="rounded-md bg-slate-900 px-4 py-2 text-[14px] text-slate-100 shadow-xl">
        {projectStore.project ? t("Suelta para añadir los archivos al proyecto") : t("Suelta un proyecto para abrirlo")}
      </p>
    </div>
  {/if}

  {#if projectStore.productionOpen}
    <Lazy loader={screens.production} />
  {/if}
  {#if ui.newProject}
    <NewProjectModal onClose={() => (ui.newProject = false)} />
  {/if}
  {#if projectStore.guideOpen}
    <Lazy loader={screens.guide} props={{ onClose: () => (projectStore.guideOpen = false), onNewProject: () => (ui.newProject = true) }} />
  {/if}
  {#if ui.shutdownCountdown !== null}
    <!-- Above every other dialog (production, queue) that may still be open. -->
    <div class="fixed inset-0 z-[60] flex items-center justify-center p-4">
      <div class="ui-backdrop"></div>
      <div class="ui-dialog w-full max-w-[400px]" role="alertdialog" aria-modal="true" aria-labelledby="shutdown-title">
        <div class="ui-dialog-header">
          <h2 id="shutdown-title" class="ui-title">{t("La computadora se apagará")}</h2>
        </div>
        <div class="px-4 py-3">
          <p class="text-[13px] text-slate-300">
            {t("El trabajo terminó. Se apagará en {seconds} s.", { seconds: ui.shutdownCountdown })}
          </p>
        </div>
        <div class="ui-dialog-footer">
          <button type="button" class="ui-btn ui-btn-primary" onclick={cancelShutdown}>{t("Cancelar el apagado")}</button>
        </div>
      </div>
    </div>
  {/if}

  {#if ui.about}
    <Lazy loader={screens.about} props={{ onclose: () => (ui.about = false) }} />
  {/if}
  {#if ui.slideshow}
    <Lazy loader={screens.slideshow} props={{ onclose: () => (ui.slideshow = false) }} />
  {/if}
  {#if ui.inspector}
    <Lazy loader={screens.inspector} props={{ onclose: () => (ui.inspector = false) }} />
  {/if}

  {#if recovery.length > 0 && !projectStore.project}
    <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div class="absolute inset-0 bg-black/50"></div>
      <div class="relative z-10 w-full max-w-md rounded-lg border border-slate-700 bg-slate-900 p-5 shadow-xl" role="dialog" aria-modal="true" aria-labelledby="recovery-title">
        <div class="flex items-center gap-2">
          <Icon name="info" class="text-cyan-400" />
          <h2 id="recovery-title" class="text-[15px] font-semibold text-slate-100">{t("Recuperar cambios sin guardar")}</h2>
        </div>
        <p class="mt-2 text-[13px] leading-5 text-slate-300">
          {t("Kinodisc se cerró sin guardar estos proyectos. Puedes abrir la última copia automática.")}
        </p>
        <ul class="mt-3 space-y-1.5">
          {#each recovery as entry (entry.projectId)}
            <li class="flex items-center justify-between gap-3 rounded-md border border-slate-800 px-3 py-2">
              <span class="min-w-0">
                <span class="block truncate text-[13px] text-slate-100">{entry.name}</span>
                <span class="text-[12px] text-slate-500">{savedAt(entry.savedAt)}</span>
              </span>
              <button type="button" class="on-accent shrink-0 rounded-md bg-cyan-500 px-3 py-1 text-[13px] font-semibold hover:bg-cyan-400" onclick={() => recover(entry)}>{t("Recuperar")}</button>
            </li>
          {/each}
        </ul>
        <div class="mt-4 flex justify-end">
          <button type="button" class="rounded-md px-3 py-1.5 text-[13px] text-slate-300 hover:text-slate-100" onclick={discardRecovery}>{t("Descartar")}</button>
        </div>
      </div>
    </div>
  {/if}
</div>
