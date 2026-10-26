<script lang="ts">
  import { t } from "./lib/i18n/index.svelte";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { commands } from "./lib/api/bindings";
  import { messageOf } from "./lib/api/client";
  import Inspector from "./lib/components/Inspector.svelte";
  import MediaPanel from "./lib/components/MediaPanel.svelte";
  import MenuWorkspace from "./lib/components/MenuWorkspace.svelte";
  import NewProjectModal from "./lib/components/NewProjectModal.svelte";
  import ProductionModal from "./lib/components/ProductionModal.svelte";
  import RecentProjects from "./lib/components/RecentProjects.svelte";
  import MotionPanel from "./lib/components/MotionPanel.svelte";
  import Simulator from "./lib/components/Simulator.svelte";
  import StoryboardBar from "./lib/components/StoryboardBar.svelte";
  import StudioHeader from "./lib/components/StudioHeader.svelte";
  import TracksPanel from "./lib/components/TracksPanel.svelte";
  import TrimmerStudio from "./lib/components/TrimmerStudio.svelte";
  import VbrPanel from "./lib/components/VbrPanel.svelte";
  import VmMap from "./lib/components/VmMap.svelte";
  import { projectStore } from "./lib/stores/project.svelte";

  let newProjectOpen = $state(false);

  onMount(() => {
    void (async () => {
      await projectStore.refreshRecentProjects();
      try {
        const startupProject = await commands.startupProjectPath();
        if (startupProject) {
          await projectStore.openProject(startupProject);
          return;
        }
      } catch (cause) {
        projectStore.error = messageOf(cause);
      }
      if (!projectStore.hasProject) newProjectOpen = true;
    })();
  });

  async function openProject() {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Proyecto de DVD Forge", extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    try {
      await projectStore.openProject(picked);
    } catch (cause) {
      projectStore.error = messageOf(cause);
    }
  }
</script>

<div class="flex h-screen min-h-0 flex-col overflow-hidden bg-[#070b14] text-slate-100">
  <StudioHeader
    onNewProject={() => (newProjectOpen = true)}
    onProduce={() => (projectStore.productionOpen = true)}
  />

  {#if !projectStore.project}
    <main class="flex min-h-0 flex-1 items-center justify-center p-6">
      <div class="w-full max-w-xl rounded-3xl border border-slate-800/80 bg-slate-950/60 p-8 text-center shadow-2xl shadow-black/30">
        <div class="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-cyan-400 to-sky-600 text-lg font-black text-slate-950">
          {t("DF")}
        </div>
        <h1 class="mt-4 text-2xl font-semibold tracking-tight text-white">{t("DVD Forge Studio")}</h1>
        <p class="mx-auto mt-2 max-w-md text-sm leading-6 text-slate-400">
          {t("Un taller único para importar medios, recortar episodios, diseñar menús Konva, simular la navegación real y producir DVD-Video, AVCHD o Blu-ray.")}
        </p>
        <div class="mt-6 flex flex-wrap items-center justify-center gap-2">
          <button
            type="button"
            onclick={() => (newProjectOpen = true)}
            class="rounded-xl bg-cyan-500 px-4 py-2 text-sm font-bold text-slate-950 transition hover:bg-cyan-400"
          >
            {t("Nuevo proyecto")}
          </button>
          <button
            type="button"
            onclick={openProject}
            class="rounded-xl border border-slate-700 px-4 py-2 text-sm font-medium text-slate-200 transition hover:border-cyan-500/60 hover:text-white"
          >
            {t("Abrir proyecto…")}
          </button>
        </div>
        <div class="mt-7 grid gap-2 text-left sm:grid-cols-3">
          {#each [
            { icon: "▣", title: "Medios", hint: "Vídeo, audio e imágenes" },
            { icon: "▤", title: "Menús", hint: "6 estilos y navegación real" },
            { icon: "◉", title: "Producción", hint: "ISO y grabación guiada" },
          ] as feature}
            <div class="rounded-xl border border-slate-800/80 bg-slate-900/40 p-3">
              <span class="text-base text-cyan-300">{feature.icon}</span>
              <p class="mt-1 text-[11px] font-semibold text-slate-200">{t(feature.title)}</p>
              <p class="mt-0.5 text-[10px] leading-4 text-slate-500">{t(feature.hint)}</p>
            </div>
          {/each}
        </div>
        {#if projectStore.recentProjects.length > 0}
          <div class="mt-5 text-left">
            <p class="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-500">
              {t("Proyectos recientes")}
            </p>
            <div class="max-h-40 overflow-y-auto pr-1">
              <RecentProjects compact />
            </div>
          </div>
        {/if}
      </div>
    </main>
  {:else}
    <div class="flex min-h-0 flex-1">
      {#if projectStore.leftCollapsed}
        <button
          type="button"
          onclick={() => (projectStore.leftCollapsed = false)}
          class="flex w-9 shrink-0 flex-col items-center justify-center gap-2 border-r border-slate-800/80 bg-slate-950/60 text-slate-500 transition hover:bg-slate-900 hover:text-slate-200"
          title={t("Mostrar biblioteca")}
        >
          <span class="text-xs">▸</span>
          <span class="text-[9px] uppercase tracking-[0.2em] [writing-mode:vertical-rl]">{t("Medios")}</span>
        </button>
      {:else}
        <MediaPanel />
      {/if}

      <main class="flex min-h-0 min-w-0 flex-1 flex-col">
        <div class="flex min-h-0 flex-1">
          {#if projectStore.studioMode === "menu"}
            <MenuWorkspace />
          {:else if projectStore.studioMode === "trim"}
            <TrimmerStudio />
          {:else if projectStore.studioMode === "simulator"}
            <Simulator />
          {:else if projectStore.studioMode === "vmMap"}
            <VmMap />
          {:else if projectStore.studioMode === "tracks"}
            <TracksPanel />
          {:else if projectStore.studioMode === "motion"}
            <MotionPanel />
          {:else if projectStore.studioMode === "vbr"}
            <VbrPanel />
          {/if}
        </div>
        <StoryboardBar />
      </main>

      {#if projectStore.rightCollapsed}
        <button
          type="button"
          onclick={() => (projectStore.rightCollapsed = false)}
          class="flex w-9 shrink-0 flex-col items-center justify-center gap-2 border-l border-slate-800/80 bg-slate-950/60 text-slate-500 transition hover:bg-slate-900 hover:text-slate-200"
          title={t("Mostrar inspector")}
        >
          <span class="text-xs">◂</span>
          <span class="text-[9px] uppercase tracking-[0.2em] [writing-mode:vertical-rl]">{t("Inspector")}</span>
        </button>
      {:else}
        <Inspector />
      {/if}
    </div>
  {/if}

  {#if projectStore.productionOpen}
    <ProductionModal />
  {/if}

  {#if newProjectOpen}
    <NewProjectModal onClose={() => (newProjectOpen = false)} />
  {/if}
</div>
