<script lang="ts">
  import { onDestroy } from "svelte";
  import type { PlayerPreset } from "../api/bindings";
  import * as actions from "../actions";
  import { t } from "../i18n/index.svelte";
  import { localizeError } from "../i18n/diagnostics";
  import { projectStore, type StudioMode } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../ui/icons";
  import { ui } from "../ui/state.svelte";
  import BatchQueue from "./BatchQueue.svelte";
  import JobHistory from "./JobHistory.svelte";
  import ToolchainDoctor from "./ToolchainDoctor.svelte";

  let lastPlanSignature = "";
  let planTimer: ReturnType<typeof setTimeout> | null = null;
  const planSignature = $derived(projectStore.planSignature);
  const project = $derived(projectStore.project);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const hasEpisodes = $derived((project?.episodes.length ?? 0) > 0);

  // The bitrate plan follows every edit, a moment after typing stops.
  $effect(() => {
    const signature = planSignature;
    if (!projectStore.hasProject) return;
    // Some edits (template, undo, priority, disc settings) clear the plan
    // without changing the signature: recalculate whenever it is missing.
    const missing =
      hasEpisodes && projectStore.plan === null && !projectStore.planBusy && !projectStore.planError;
    if (signature === lastPlanSignature && !missing) return;
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

  const modes = $derived<Array<{ id: StudioMode; icon: IconName; label: string; hint: string; expert: boolean }>>([
    { id: "trim", icon: "cut", label: "Vídeos", hint: "Recorte y capítulos", expert: false },
    { id: "menu", icon: "menu", label: "Menú", hint: "Diseño del menú del disco", expert: false },
    { id: "tracks", icon: "tracks", label: "Audio y subtítulos", hint: "Pistas, idiomas y desfases", expert: false },
    { id: "simulator", icon: "remote", label: "Vista previa", hint: "Prueba el disco con un mando virtual", expert: false },
    { id: "motion", icon: "film", label: "Animación", hint: "Menús y botones de vídeo", expert: true },
    { id: "vmMap", icon: "layers", label: "Mapa VM", hint: "Flujo de navegación del DVD", expert: true },
    { id: "vbr", icon: "gauge", label: "Calidad VBR", hint: "Curva, prioridad y capacidad", expert: true },
  ]);

  let discOpen = $state(false);
  $effect(() => {
    if (!discOpen) return;
    const close = () => (discOpen = false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  });
  const discSummary = $derived(
    project
      ? [
          { dvd5: "DVD-5", dvd9: "DVD-9", bd25: "BD-25", bd50: "BD-50" }[project.disc.capacity],
          project.disc.mode === "avchd" ? "HD" : project.disc.standard === "pal" ? "PAL" : "NTSC",
          project.disc.mode === "avchd" ? null : project.disc.aspect === "sixteenNine" ? "16:9" : "4:3",
        ]
          .filter(Boolean)
          .join(" · ")
      : "",
  );

  const tool = "flex h-8 items-center gap-1.5 rounded-md px-2 text-[13px] text-slate-300 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-40 disabled:hover:bg-transparent";
  const select = "h-7 rounded-md border border-slate-700 bg-slate-900 px-1.5 text-[12px] text-slate-200 outline-none focus:border-cyan-500";
</script>

<header class="relative z-40 shrink-0 border-b border-slate-800 bg-slate-900">
  <!-- Toolbar -->
  <div class="flex h-11 items-center gap-1 px-2">
    <div class="mr-2 flex min-w-0 items-center gap-2 pl-1">
      <Icon name="disc" size={20} class="text-cyan-400" />
      {#if project}
        <input
          class="w-48 truncate rounded-md border border-transparent bg-transparent px-1 py-0.5 text-[13px] font-semibold text-slate-100 outline-none hover:border-slate-700 focus:border-cyan-500"
          value={project.name}
          aria-label={t("Nombre del proyecto")}
          oninput={(event) => projectStore.renameProject(event.currentTarget.value)}
        />
        {#if projectStore.dirty}
          <span class="text-[11px] text-slate-500" title={t("Hay cambios sin guardar")}>●</span>
        {/if}
      {:else}
        <span class="text-[13px] font-semibold text-slate-100">Kinodisc</span>
      {/if}
    </div>

    <div class="mx-1 h-5 w-px bg-slate-800"></div>
    <button type="button" class={tool} onclick={actions.newProject} title={t("Nuevo proyecto (Ctrl+N)")}>
      <Icon name="newFile" /><span class="hidden xl:inline">{t("Nuevo")}</span>
    </button>
    <button type="button" class={tool} onclick={actions.openProject} title={t("Abrir (Ctrl+O)")}>
      <Icon name="open" /><span class="hidden xl:inline">{t("Abrir")}</span>
    </button>
    <button type="button" class={tool} onclick={actions.saveProject} disabled={!project} title={t("Guardar (Ctrl+S)")}>
      <Icon name="save" /><span class="hidden xl:inline">{t("Guardar")}</span>
    </button>

    <div class="mx-1 h-5 w-px bg-slate-800"></div>
    <button type="button" class={tool} onclick={actions.undo} disabled={!projectStore.canUndo} title={t("Deshacer (Ctrl+Z)")} aria-label={t("Deshacer")}>
      <Icon name="undo" />
    </button>
    <button type="button" class={tool} onclick={actions.redo} disabled={!projectStore.canRedo} title={t("Rehacer (Ctrl+Y)")} aria-label={t("Rehacer")}>
      <Icon name="redo" />
    </button>

    <div class="mx-1 h-5 w-px bg-slate-800"></div>
    <button type="button" class={tool} onclick={() => actions.importMedia()} disabled={!project} title={t("Importar medios (Ctrl+I)")}>
      <Icon name="import" /><span class="hidden lg:inline">{t("Importar")}</span>
    </button>
    <button type="button" class={tool} onclick={() => (ui.slideshow = true)} disabled={!project} title={t("Presentación de fotos")}>
      <Icon name="photo" /><span class="hidden lg:inline">{t("Fotos")}</span>
    </button>
    <button type="button" class={tool} onclick={() => (ui.inspector = true)} disabled={!project} title={t("Inspeccionar o importar un disco")}>
      <Icon name="inspect" /><span class="hidden lg:inline">{t("Disco")}</span>
    </button>

    <div class="ml-auto flex items-center gap-1">
      <button type="button" class={tool} onclick={() => (projectStore.guideOpen = !projectStore.guideOpen)} title={t("Guía paso a paso (F1)")} aria-label={t("Guía")}>
        <Icon name="help" />
      </button>
      <button
        type="button"
        onclick={actions.produce}
        disabled={!project || !hasEpisodes}
        title={hasEpisodes ? t("Producir disco (Ctrl+R)") : t("Añade al menos un vídeo para producir el disco")}
        class="on-accent ml-1 flex h-8 items-center gap-1.5 rounded-md bg-cyan-500 px-3 text-[13px] font-semibold hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
      >
        <Icon name="burn" />{t("Producir disco")}
      </button>
    </div>
  </div>

  {#if project}
    <!-- Workspaces and disc settings -->
    <div class="flex h-9 items-center gap-2 border-t border-slate-800 px-2">
      <nav class="flex min-w-0 flex-1 items-stretch self-stretch overflow-x-auto" aria-label={t("Espacios de trabajo")}>
        {#each modes.filter((item) => projectStore.expertMode || !item.expert) as mode}
          <button
            type="button"
            onclick={() => (projectStore.studioMode = mode.id)}
            title={t(mode.hint)}
            aria-current={projectStore.studioMode === mode.id ? "page" : undefined}
            class={`flex shrink-0 items-center gap-1.5 border-b-2 px-3 text-[13px] ${
              projectStore.studioMode === mode.id
                ? "border-cyan-500 text-slate-100"
                : "border-transparent text-slate-400 hover:text-slate-200"
            }`}
          >
            <Icon name={mode.icon} size={15} />{t(mode.label)}
          </button>
        {/each}
      </nav>

      <div class="relative shrink-0">
        <button
          type="button"
          class="ui-btn ui-btn-sm"
          aria-haspopup="dialog"
          aria-expanded={discOpen}
          onclick={(event) => {
            event.stopPropagation();
            discOpen = !discOpen;
          }}
          title={t("Ajustes del disco")}
        >
          <Icon name="disc" size={14} />{discSummary}<Icon name="chevronDown" size={12} />
        </button>
        {#if discOpen}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="disc-pop absolute right-0 top-full z-40 mt-1 w-72 space-y-2.5 rounded-md border border-slate-700 bg-slate-900 p-3 shadow-xl"
            role="dialog"
            aria-label={t("Ajustes del disco")}
            tabindex="-1"
            onpointerdown={(event) => event.stopPropagation()}
          >
            <p class="ui-title">{t("Ajustes del disco")}</p>
            <div class="disc-fields grid gap-2">
        {#if project.disc.capacity === "dvd5" || project.disc.capacity === "dvd9"}
          <span class="ui-label mb-0">{t("Formato")}</span>
<select class="ui-input" value={project.disc.mode} title={t("Formato de autoría")}
            onchange={(event) => projectStore.setDiscMode(event.currentTarget.value as "dvdVideo" | "avchd")}>
            <option value="dvdVideo">{t("DVD-Video")}</option>
            <option value="avchd">{t("AVCHD")}</option>
          </select>
        {/if}
        <span class="ui-label mb-0">{t("Capacidad")}</span>
<select class="ui-input" value={project.disc.capacity} title={t("Capacidad del disco")}
          onchange={(event) => projectStore.setDiscCapacity(event.currentTarget.value as "dvd5" | "dvd9" | "bd25" | "bd50")}>
          <option value="dvd5">{t("DVD-5 · 4,7 GB")}</option>
          <option value="dvd9">{t("DVD-9 · 8,5 GB")}</option>
          <option value="bd25">{t("BD-25 · 25 GB")}</option>
          <option value="bd50">{t("BD-50 · 50 GB")}</option>
        </select>
        {#if !isAvchd}
          <span class="ui-label mb-0">{t("Sistema de TV")}</span>
<select class="ui-input" value={project.disc.standard} title={t("Estándar de televisión")}
            onchange={(event) => projectStore.setStandard(event.currentTarget.value as "ntsc" | "pal")}>
            <option value="pal">{t("PAL")}</option>
            <option value="ntsc">{t("NTSC")}</option>
          </select>
          <span class="ui-label mb-0">{t("Pantalla")}</span>
<select class="ui-input" value={project.disc.aspect} title={t("Relación de aspecto")}
            onchange={(event) => projectStore.setAspect(event.currentTarget.value as "fourThree" | "sixteenNine")}>
            <option value="sixteenNine">16:9</option>
            <option value="fourThree">4:3</option>
          </select>
        {/if}
        {#if projectStore.expertMode}
          {#if project.disc.mode === "dvdVideo"}
            <span class="ui-label mb-0">{t("Región")}</span>
<select class="ui-input" value={project.disc.regionMask} title={t("Región DVD")}
              onchange={(event) => projectStore.setRegionMask(Number(event.currentTarget.value))}>
              <option value={0}>{t("Región libre")}</option>
              <option value={1}>{t("Región 1")}</option>
              <option value={2}>{t("Región 2")}</option>
              <option value={4}>{t("Región 3")}</option>
              <option value={8}>{t("Región 4")}</option>
              <option value={16}>{t("Región 5")}</option>
              <option value={32}>{t("Región 6")}</option>
            </select>
          {/if}
          <span class="ui-label mb-0">{t("Reproductor")}</span>
<select class="ui-input" value={project.disc.playerPreset ?? "any"} aria-label={t("Reproductor")}
            title={t("Ajusta el disco para un reproductor concreto (más compatible)")}
            onchange={(event) => projectStore.setPlayerPreset(event.currentTarget.value as PlayerPreset)}>
            <option value="any">{t("Cualquier reproductor")}</option>
            <option value="ps2">PlayStation 2</option>
            <option value="playStation">PlayStation 3/4/5</option>
            <option value="bluRayPlayer">{t("Reproductor Blu-ray de salón")}</option>
            <option value="dvdPlayer">{t("Reproductor DVD antiguo")}</option>
          </select>
        {/if}
            </div>
            <p class="ui-hint">{t("Los cambios recalculan el espacio y la calidad automáticamente.")}</p>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  {#if ui.notice && !projectStore.error}
    <div class="flex items-center justify-between gap-3 border-t border-emerald-500/25 bg-emerald-500/10 px-3 py-1.5 text-[12px] text-emerald-200" role="status">
      <span class="truncate">{ui.notice}</span>
      <button type="button" onclick={() => (ui.notice = null)} aria-label={t("Cerrar")}><Icon name="close" size={14} /></button>
    </div>
  {/if}
  {#if projectStore.error}
    <div class="flex items-center justify-between gap-3 border-t border-rose-500/25 bg-rose-500/10 px-3 py-1.5 text-[12px] text-rose-200" role="alert">
      <span class="truncate" title={localizeError(projectStore.error)}>{localizeError(projectStore.error)}</span>
      <button type="button" onclick={() => (projectStore.error = null)} aria-label={t("Cerrar")}><Icon name="close" size={14} /></button>
    </div>
  {/if}

  {#if ui.doctor}
    <ToolchainDoctor onClose={() => (ui.doctor = false)} />
  {/if}
  {#if ui.queue}
    <BatchQueue onClose={() => (ui.queue = false)} />
  {/if}
  {#if ui.history}
    <div class="fixed inset-0 z-50 flex items-center justify-center p-4">
      <button type="button" aria-label={t("Cerrar")} class="absolute inset-0 cursor-default bg-black/50" onclick={() => (ui.history = false)}></button>
      <div class="relative z-10 flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border border-slate-700 bg-slate-900 shadow-xl" role="dialog" aria-modal="true" aria-labelledby="history-title">
        <div class="flex items-center justify-between border-b border-slate-800 px-4 py-3">
          <h2 id="history-title" class="text-[14px] font-semibold text-slate-100">{t("Historial de trabajos")}</h2>
          <button type="button" class="text-slate-400 hover:text-slate-100" onclick={() => (ui.history = false)} aria-label={t("Cerrar")}><Icon name="close" /></button>
        </div>
        <div class="min-h-0 overflow-y-auto p-3"><JobHistory /></div>
      </div>
    </div>
  {/if}
</header>

<style>
  .disc-pop {
    transform-origin: top right;
    animation: disc-in 140ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  @keyframes disc-in {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
</style>
