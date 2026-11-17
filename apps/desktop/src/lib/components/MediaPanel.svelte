<script lang="ts">
  import { isPlaceholderAsset } from "../utils/assets";
  import * as actions from "../actions";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../ui/icons";
  import { t } from "../i18n/index.svelte";
  import type { ButtonStyleKind, MediaAsset_Serialize, MediaKind } from "../api/bindings";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { projectStore, type LayoutKind } from "../stores/project.svelte";
  import { basename, formatBytes, formatDuration } from "../utils/format";

  let search = $state("");
  let busy = $state(false);
  let notice = $state<string | null>(null);
  let localError = $state<string | null>(null);

  const project = $derived(projectStore.project);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  // AVCHD/Blu-ray projects support HDMV menus; only a project without any
  // menu is a continuous-playback disc where menu tools do not apply.
  const noMenus = $derived(!project || project.menus.length === 0);
  const selectedButton = $derived(projectStore.selectedButton);
  const selectedEpisode = $derived(projectStore.selectedEpisode);
  const assets = $derived(
    projectStore.assets.filter((asset) => {
      // New projects carry an empty placeholder background; not a real file.
      if (isPlaceholderAsset(asset)) return false;
      const term = search.trim().toLowerCase();
      if (!term) return true;
      return (
        asset.displayName.toLowerCase().includes(term) ||
        asset.kind.includes(term) ||
        asset.probe.formatName.toLowerCase().includes(term)
      );
    }),
  );

  const tabs: Array<{ id: "media" | "layouts" | "styles"; label: string; icon: IconName }> = [
    { id: "media", label: "Medios", icon: "film" },
    { id: "layouts", label: "Diseños", icon: "menu" },
    { id: "styles", label: "Estilos", icon: "layers" },
  ];

  type KindFilter = "all" | "video" | "audio" | "image";
  const kindFilters: Array<{ id: KindFilter; label: string }> = [
    { id: "all", label: "Todo" },
    { id: "video", label: "Vídeo" },
    { id: "audio", label: "Audio" },
    { id: "image", label: "Imagen" },
  ];
  let kindFilter = $state<KindFilter>("all");
  let menuFor = $state<string | null>(null);
  const visibleAssets = $derived(
    assets.filter((asset) => kindFilter === "all" || asset.kind === kindFilter),
  );

  $effect(() => {
    if (!menuFor) return;
    const close = () => (menuFor = null);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  });

  interface RowAction {
    label: string;
    icon: IconName;
    run: () => unknown;
    disabled?: boolean;
    hint?: string;
  }

  /** Actions offered in the "⋯" menu of a media row. */
  function secondaryActions(asset: MediaAsset_Serialize): RowAction[] {
    const list: RowAction[] = [];
    if (!noMenus && (asset.kind === "image" || asset.kind === "video")) {
      list.push({
        label: t("Usar como miniatura del botón"),
        icon: "photo",
        run: () => setButtonThumbnail(asset),
        disabled: !selectedButton,
        hint: selectedButton ? "" : t("Selecciona un botón del menú para asignarle una miniatura."),
      });
    }
    if (asset.kind === "image") list.push({ label: t("Usar como fondo del menú"), icon: "menu", run: () => setBackground(asset) });
    if (asset.kind === "video") list.push({ label: t("Usar como fondo animado"), icon: "film", run: () => setVideoBackground(asset) });
    if (asset.kind === "audio") list.push({ label: t("Usar como música del menú"), icon: "music", run: () => setMusic(asset) });
    return list;
  }

  const layouts: Array<{
    kind: LayoutKind;
    title: string;
    hint: string;
    preview: string;
  }> = [
    { kind: "grid2", title: "Cuadrícula 2×2", hint: "Miniaturas grandes", preview: "grid-cols-2" },
    { kind: "grid3", title: "Cuadrícula 3×2", hint: "Hasta 6 episodios", preview: "grid-cols-3" },
    { kind: "list", title: "Lista cinematográfica", hint: "Cine y series", preview: "grid-cols-2" },
    { kind: "direct", title: "Reproducción directa", hint: "Reproducción + capítulos", preview: "grid-cols-1" },
    { kind: "clear", title: "Lienzo limpio", hint: "Diseño manual", preview: "grid-cols-1" },
  ];

  const styles: Array<{
    kind: ButtonStyleKind;
    title: string;
    hint: string;
  }> = [
    { kind: "thumbnailCard", title: "Miniatura", hint: "Miniatura 16:9 + etiqueta" },
    { kind: "classicFrame", title: "Clásico", hint: "Marco DVD clásico" },
    { kind: "glassPill", title: "Cristal", hint: "Cápsula translúcida" },
    { kind: "playBadge", title: "Insignia Play", hint: "Botón de reproducción" },
    { kind: "textOnly", title: "Solo texto", hint: "Texto limpio" },
    { kind: "hotspot", title: "Zona activa", hint: "Zona interactiva invisible" },
  ];


  async function importMedia() {
    busy = true;
    localError = null;
    try {
      await actions.importMedia();
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }


  function addEpisode(asset: MediaAsset_Serialize) {
    const existing = projectStore.episodeForAsset(asset.id);
    if (existing) {
      projectStore.selectEpisode(existing.id);
      notice = t("Episodio {number} seleccionado.", { number: existing.number });
      return;
    }
    const episode = projectStore.addEpisodeFromAsset(asset.id);
    if (episode) {
      projectStore.selectEpisode(episode.id);
      notice = t("«{title}» añadido como episodio {number}.", { title: episode.title, number: episode.number });
    } else {
      localError = t("No se pudo añadir el vídeo como episodio.");
    }
  }

  async function setBackground(asset: MediaAsset_Serialize) {
    if (!projectStore.activeMenu) {
      localError = t("El proyecto no tiene un menú activo.");
      return;
    }
    const assetId = asset.id;
    if (!projectStore.previewForAsset(assetId)) {
      const path = projectStore.assetPath(asset);
      if (path) {
        try {
          const preview = await unwrap(commands.readImageDataUrl(path));
          projectStore.previewUrls[assetId] = preview;
        } catch (cause) {
          localError = messageOf(cause);
          return;
        }
      }
    }
    projectStore.setMenuBackground(assetId);
    notice = t("«{name}» es ahora el fondo del menú.", { name: asset.displayName });
  }

  function setVideoBackground(asset: MediaAsset_Serialize) {
    if (!projectStore.activeMenu) {
      localError = t("El proyecto no tiene un menú activo.");
      return;
    }
    projectStore.setMenuBackgroundVideo(asset.id);
    void projectStore.ensureVideoThumbnail(asset.id);
    notice = t("«{name}» será el fondo animado del menú.", { name: asset.displayName });
  }

  function setMusic(asset: MediaAsset_Serialize) {
    projectStore.setMenuAudio(asset.id);
    notice = t("«{name}» será la música del menú.", { name: asset.displayName });
  }

  function setButtonThumbnail(asset: MediaAsset_Serialize) {
    const button = selectedButton;
    if (!button) {
      localError = t("Selecciona un botón del menú para asignarle una miniatura.");
      return;
    }
    projectStore.updateButton(button.id, {
      thumbnail: {
        asset: asset.id,
        inFrame: 0,
        durationFrames: 0,
        fps:
          project?.disc.standard === "pal"
            ? { num: 25, den: 1 }
            : { num: 30_000, den: 1_001 },
      },
      styleKind: button.styleKind === "hotspot" || button.styleKind === "textOnly"
        ? "thumbnailCard"
        : button.styleKind,
    });
    if (asset.kind === "video") {
      void projectStore.ensureVideoThumbnail(asset.id);
    }
    notice = t("Miniatura asignada a «{name}».", { name: button.name });
  }

  function applyStyle(kind: ButtonStyleKind) {
    const button = selectedButton;
    if (button) {
      projectStore.updateButton(button.id, { styleKind: kind });
      if (kind === "thumbnailCard" && !button.thumbnail && selectedEpisode) {
        projectStore.updateButton(button.id, {
          thumbnail: {
            asset: selectedEpisode.video,
            inFrame: 0,
            durationFrames: 0,
            fps: { num: 30_000, den: 1_001 },
          },
        });
      }
      notice = t("Estilo aplicado a «{name}».", { name: button.name });
      return;
    }
    if (!projectStore.addButton(kind)) {
      localError = projectStore.error;
    } else {
      notice = t("Botón añadido con el estilo elegido.");
    }
  }

  function applyLayout(kind: LayoutKind) {
    if (!projectStore.applyLayout(kind)) {
      localError = projectStore.error ?? t("No se pudo aplicar la plantilla.");
    } else {
      notice = t(kind === "clear" ? "Lienzo vaciado." : "Plantilla aplicada.");
      projectStore.studioMode = "menu";
    }
  }

  function kindLabel(kind: MediaKind): string {
    return kind === "video" ? t("vídeo") : kind === "audio" ? t("audio") : kind === "image" ? t("imagen") : t("subs");
  }

  function assetMeta(asset: MediaAsset_Serialize): string {
    if (asset.probe.video) {
      return `${asset.probe.video.width}×${asset.probe.video.height} · ${formatDuration(asset.probe.durationMs)}`;
    }
    if (asset.probe.audio.length > 0) {
      const audio = asset.probe.audio[0];
      return `${audio.codec} · ${audio.channels} ${t("canales")} · ${formatDuration(asset.probe.durationMs)}`;
    }
    return `${asset.probe.formatName} · ${formatBytes(asset.probe.sizeBytes)}`;
  }
</script>

<aside class="ui-panel w-[280px] shrink-0 border-r border-slate-800">
  <div class="ui-tabs shrink-0" role="tablist">
    {#each tabs as tab}
      <button
        type="button"
        role="tab"
        aria-selected={projectStore.mediaTab === tab.id}
        onclick={() => (projectStore.mediaTab = tab.id)}
      >
        <Icon name={tab.icon} size={15} />{t(tab.label)}
      </button>
    {/each}
    <button
      type="button"
      class="ml-auto"
      onclick={() => (projectStore.leftCollapsed = true)}
      title={t("Ocultar biblioteca")}
      aria-label={t("Ocultar biblioteca")}
    >
      <span class="rotate-180"><Icon name="chevronRight" size={14} /></span>
    </button>
  </div>

  {#if localError || notice}
    <div
      class={`mx-3 mt-2 rounded-md px-2.5 py-1.5 text-[12px] leading-5 ${
        localError ? "bg-rose-500/10 text-rose-200" : "bg-emerald-500/10 text-emerald-200"
      }`}
      role={localError ? "alert" : "status"}
    >
      {localError ?? notice}
    </div>
  {/if}

  {#if projectStore.mediaTab === "media"}
    <div class="shrink-0 space-y-2 px-3 pb-2 pt-3">
      <div class="flex items-center gap-2">
        <div class="relative min-w-0 flex-1">
          <span class="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-slate-500"><Icon name="search" size={14} /></span>
          <input class="ui-input pl-7" placeholder={t("Buscar medios…")} bind:value={search} aria-label={t("Buscar medios…")} />
        </div>
        <button type="button" class="ui-btn ui-btn-primary" onclick={importMedia} disabled={busy || !project} title={t("Importar medios (Ctrl+I)")}>
          <Icon name="import" size={15} />{t("Importar")}
        </button>
      </div>
      <div class="ui-seg" role="group" aria-label={t("Tipo de medio")}>
        {#each kindFilters as filter}
          <button type="button" aria-pressed={kindFilter === filter.id} onclick={() => (kindFilter = filter.id)}>
            {t(filter.label)}
          </button>
        {/each}
      </div>
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
      {#if !project}
        <p class="ui-empty">{t("Crea o abre un proyecto para importar medios.")}</p>
      {:else if visibleAssets.length === 0}
        <div class="ui-empty">
          <Icon name="import" size={28} class="text-slate-500" />
          <p class="text-slate-300">{assets.length === 0 ? t("Sin medios") : t("Nada coincide con la búsqueda")}</p>
          <p class="ui-hint">{t("Importa vídeo, audio o imágenes, o arrástralos a la ventana.")}</p>
        </div>
      {:else}
        <ul class="space-y-0.5">
          {#each visibleAssets as asset (asset.id)}
            {@const episode = projectStore.episodeForAsset(asset.id)}
            {@const preview = projectStore.previewForAsset(asset.id)}
            <li class="group relative">
              <div
                class="ui-row pr-1"
                role="button"
                tabindex="0"
                ondblclick={() => asset.kind === "video" && addEpisode(asset)}
                onkeydown={(event) => event.key === "Enter" && asset.kind === "video" && addEpisode(asset)}
                title={asset.kind === "video" ? t("Doble clic para añadirlo como episodio") : asset.displayName}
              >
                <div class="flex h-9 w-14 shrink-0 items-center justify-center overflow-hidden rounded bg-black">
                  {#if preview}
                    <img src={preview} alt="" class="h-full w-full bg-black object-contain" />
                  {:else}
                    <Icon name={asset.kind === "video" ? "film" : asset.kind === "audio" ? "music" : asset.kind === "image" ? "photo" : "text"} size={16} class="text-slate-500" />
                  {/if}
                </div>
                <div class="min-w-0 flex-1">
                  <p class="truncate text-[13px] text-slate-100">{basename(asset.displayName)}</p>
                  <p class="truncate text-[11px] text-slate-500">{assetMeta(asset)}</p>
                </div>
                {#if asset.kind === "video"}
                  {#if episode}
                    <button type="button" class="ui-chip ui-chip-accent" onclick={() => addEpisode(asset)} title={t("Seleccionar episodio")}>E{episode.number}</button>
                  {:else}
                    <button type="button" class="ui-btn ui-btn-sm" onclick={() => addEpisode(asset)} title={t("Añadir como episodio")}>
                      <Icon name="add" size={14} />{t("Añadir")}
                    </button>
                  {/if}
                {/if}
                {#if secondaryActions(asset).length > 0}
                  <button
                    type="button"
                    class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon"
                    aria-label={t("Más acciones")}
                    aria-haspopup="menu"
                    aria-expanded={menuFor === asset.id}
                    onclick={(event) => {
                      event.stopPropagation();
                      menuFor = menuFor === asset.id ? null : asset.id;
                    }}
                  >⋯</button>
                {/if}
              </div>
              {#if menuFor === asset.id}
                <div class="row-menu absolute right-1 top-full z-30 mt-0.5 w-56 rounded-md border border-slate-700 bg-slate-900 py-1 shadow-xl" role="menu">
                  {#each secondaryActions(asset) as action}
                    <button
                      type="button"
                      role="menuitem"
                      class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-slate-200 hover:bg-slate-800 disabled:opacity-40"
                      disabled={action.disabled}
                      title={action.hint ?? ""}
                      onclick={() => {
                        menuFor = null;
                        void action.run();
                      }}
                    >
                      <Icon name={action.icon} size={15} class="text-slate-400" />{action.label}
                    </button>
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else if projectStore.mediaTab === "layouts"}
    <div class="min-h-0 flex-1 overflow-y-auto p-3">
      <p class="ui-hint">{t("Genera una composición completa a partir de los episodios y ajusta después en el lienzo.")}</p>
      <div class="mt-3 grid grid-cols-2 gap-2">
        {#each layouts as layout}
          <button
            type="button"
            onclick={() => applyLayout(layout.kind)}
            disabled={noMenus}
            class="rounded-md border border-slate-700 p-2 text-left hover:border-cyan-500 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <div class="grid h-14 grid-rows-2 gap-1 rounded bg-slate-950 p-1.5">
              <div class={`grid gap-1 ${layout.preview}`}>
                <span class="rounded-sm bg-cyan-500/30"></span>
                <span class="rounded-sm bg-slate-700"></span>
              </div>
              <div class={`grid gap-1 ${layout.preview}`}>
                <span class="rounded-sm bg-slate-700"></span>
                <span class="rounded-sm bg-slate-700/60"></span>
              </div>
            </div>
            <p class="mt-1.5 text-[12px] font-medium text-slate-100">{t(layout.title)}</p>
            <p class="text-[11px] text-slate-500">{t(layout.hint)}</p>
          </button>
        {/each}
      </div>
      {#if noMenus && isAvchd}
        <p class="ui-hint mt-3">
          {t("Este disco no tiene menú (reproducción continua). Crea una pantalla de menú desde la barra inferior para usar los diseños.")}
        </p>
      {/if}
    </div>
  {:else}
    <div class="min-h-0 flex-1 overflow-y-auto p-3">
      <p class="ui-hint">
        {selectedButton
          ? t("Aplica un estilo a «{name}».", { name: selectedButton.name })
          : t("Selecciona un botón o crea uno nuevo con el estilo elegido.")}
      </p>
      <div class="mt-3 grid grid-cols-2 gap-2">
        {#each styles as style}
          <button
            type="button"
            onclick={() => applyStyle(style.kind)}
            disabled={noMenus}
            aria-pressed={selectedButton?.styleKind === style.kind}
            class={`rounded-md border p-2 text-left disabled:cursor-not-allowed disabled:opacity-40 ${
              selectedButton?.styleKind === style.kind ? "border-cyan-500 bg-cyan-500/10" : "border-slate-700 hover:border-slate-500"
            }`}
          >
            <div class="flex h-12 items-center justify-center rounded bg-slate-950">
              {#if style.kind === "thumbnailCard"}
                <span class="h-7 w-12 rounded-sm border border-slate-500 bg-slate-700"></span>
              {:else if style.kind === "playBadge"}
                <span class="flex h-6 w-6 items-center justify-center rounded-full bg-slate-200 text-slate-900"><Icon name="play" size={12} /></span>
              {:else if style.kind === "glassPill"}
                <span class="h-5 w-14 rounded-full border border-white/30 bg-white/10"></span>
              {:else if style.kind === "classicFrame"}
                <span class="h-6 w-14 rounded-sm border-2 border-slate-400 bg-slate-800"></span>
              {:else if style.kind === "textOnly"}
                <span class="text-[13px] font-semibold text-slate-200">{t("Texto")}</span>
              {:else}
                <span class="h-5 w-14 rounded-sm border border-dashed border-cyan-400"></span>
              {/if}
            </div>
            <p class="mt-1.5 text-[12px] font-medium text-slate-100">{t(style.title)}</p>
            <p class="text-[11px] text-slate-500">{t(style.hint)}</p>
          </button>
        {/each}
      </div>
      {#if !selectedButton && !noMenus}
        <button
          type="button"
          class="ui-btn mt-3 w-full"
          onclick={() => {
            if (!projectStore.addButton("classicFrame")) localError = projectStore.error;
          }}
        >
          <Icon name="add" size={15} />{t("Añadir botón clásico")}
        </button>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .row-menu {
    transform-origin: top right;
    animation: row-menu-in 140ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  @keyframes row-menu-in {
    from {
      opacity: 0;
      transform: scale(0.97);
    }
  }
</style>
