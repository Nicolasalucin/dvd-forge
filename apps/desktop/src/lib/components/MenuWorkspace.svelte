<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import CanvasStage from "../editor/CanvasStage.svelte";
  import TemplateGallery from "./TemplateGallery.svelte";
  import Icon from "../ui/Icon.svelte";
  import type { IconName } from "../ui/icons";

  // Right-click menu on a canvas button.
  let context = $state<{ x: number; y: number } | null>(null);
  $effect(() => {
    if (!context) return;
    const close = () => (context = null);
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
    };
  });
  type ContextEntry = { label: string; icon: IconName; run: () => unknown; hint?: string } | "separator";
  const contextEntries: ContextEntry[] = [
    { label: "Duplicar", icon: "duplicate", run: () => projectStore.duplicateSelectedButton(), hint: "Ctrl+D" },
    { label: "Eliminar", icon: "trash", run: () => projectStore.deleteSelectedButton(), hint: "Supr" },
    "separator",
    { label: "Centrar horizontalmente", icon: "menu", run: () => projectStore.arrangeButtons("centerH") },
    { label: "Centrar verticalmente", icon: "menu", run: () => projectStore.arrangeButtons("centerV") },
    { label: "Traer al frente", icon: "layers", run: () => projectStore.restackSelectedButton("front") },
    { label: "Enviar atrás", icon: "layers", run: () => projectStore.restackSelectedButton("back") },
    "separator",
    { label: "Alinear todos arriba", icon: "tracks", run: () => projectStore.arrangeButtons("alignTop") },
    { label: "Alinear todos al centro", icon: "tracks", run: () => projectStore.arrangeButtons("alignMiddle") },
    { label: "Distribuir en horizontal", icon: "tracks", run: () => projectStore.arrangeButtons("distributeH") },
    { label: "Distribuir en vertical", icon: "tracks", run: () => projectStore.arrangeButtons("distributeV") },
  ];

  // Templates open in a drop-down so the canvas keeps the space.
  let templatesOpen = $state(
    (projectStore.activeMenu?.buttons.length ?? 0) === 0 && projectStore.episodes.length > 0,
  );
  $effect(() => {
    if (!templatesOpen) return;
    const close = () => (templatesOpen = false);
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  });
  import { projectStore } from "../stores/project.svelte";

  let busy = $state(false);
  let localError = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const project = $derived(projectStore.project);
  const menu = $derived(projectStore.activeMenu);
  const isRootMenu = $derived(Boolean(menu && menu.id === projectStore.project?.rootMenu));
  const autoPlay = $derived(menu?.timing.autoPlay ?? false);
  const autoPlayHd = $derived(projectStore.project?.disc.mode === "avchd");
  const menus = $derived(project?.menus ?? []);
  const selected = $derived(projectStore.selectedButton);

  /**
   * Keyboard editing of the menu canvas: arrows move the selected button
   * (Shift = 4× faster), [ and ] select the previous/next button, Delete
   * removes it and Escape deselects. Ignored while typing in a field.
   */
  function onCanvasKey(event: KeyboardEvent) {
    if (projectStore.studioMode !== "menu" || !menu) return;
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const target = event.target as HTMLElement | null;
    if (target && (target.closest("input, textarea, select, [contenteditable='true']") || document.querySelector("[role=dialog][aria-modal=true]"))) return;
    const buttons = menu.buttons;
    if (event.key === "[" || event.key === "]") {
      if (buttons.length === 0) return;
      const current = buttons.findIndex((button) => button.id === projectStore.selectedButtonId);
      const delta = event.key === "]" ? 1 : -1;
      const next = (current + delta + buttons.length) % buttons.length;
      projectStore.selectedButtonId = buttons[current < 0 ? 0 : next].id;
      event.preventDefault();
      return;
    }
    const item = projectStore.selectedItem;
    if (item) {
      if (event.key === "Escape") projectStore.selectItem(null);
      else if (event.key === "Delete") projectStore.deleteSelectedItem();
      else {
        const step = Math.max(2, menu.canvas.gridPx || 8) * (event.shiftKey ? 4 : 1);
        const move = ({ ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] } as Record<string, [number, number]>)[event.key];
        if (!move) return;
        projectStore.updateItem(item.id, {
          rect: { ...item.rect, x: Math.max(0, (item.rect.x ?? 0) + move[0]), y: Math.max(0, (item.rect.y ?? 0) + move[1]) },
        });
      }
      event.preventDefault();
      return;
    }
    if (!selected) return;
    if (event.key === "Escape") {
      projectStore.selectedButtonId = null;
      event.preventDefault();
      return;
    }
    if (event.key === "Delete") {
      projectStore.deleteSelectedButton();
      event.preventDefault();
      return;
    }
    const step = Math.max(2, menu.canvas.gridPx || 8) * (event.shiftKey ? 4 : 1);
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = moves[event.key];
    if (!move) return;
    event.preventDefault();
    const rect = selected.rect;
    const accepted = projectStore.updateButtonRect(selected.id, {
      x: (rect.x ?? 0) + move[0],
      y: (rect.y ?? 0) + move[1],
      w: rect.w ?? 0,
      h: rect.h ?? 0,
    });
    if (!accepted) localError = t("El botón se solapa o no respeta los 4 px de separación.");
  }

  function styleLabel(kind: string): string {
    switch (kind) {
      case "thumbnailCard":
        return "Miniatura";
      case "classicFrame":
        return "Clásico";
      case "glassPill":
        return "Cristal";
      case "playBadge":
        return "Insignia Play";
      case "textOnly":
        return "Solo texto";
      case "hotspot":
        return "Zona activa";
      default:
        return "Estilo";
    }
  }

  function actionLabel(kind: string): string {
    switch (kind) {
      case "playEpisode":
        return t("Reproducir episodio");
      case "playAll":
        return t("Reproducir todo");
      case "gotoMenu":
        return t("Ir a menú");
      case "jumpTitleChapter":
        return t("Ir a capítulo");
      case "playFirstPlay":
        return t("Reproducir inicio");
      case "setAudioTrack":
        return t("Cambiar pista de audio");
      case "setSubtitleTrack":
        return t("Elegir subtítulos");
      case "resume":
        return t("Reanudar");
      default:
        return t("Sin acción");
    }
  }
  const previewStates = [
    { id: "normal" as const, label: "Normal" },
    { id: "highlight" as const, label: "Resaltado" },
    { id: "select" as const, label: "Selección" },
  ];

  async function pickBackground() {
    if (!menu) return;
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Imagen de fondo"), extensions: ["png", "jpg", "jpeg", "webp"] }],
    });
    if (typeof picked !== "string") return;
    busy = true;
    localError = null;
    try {
      const assetId = await projectStore.addImageAssetFromPath(picked);
      if (!assetId) throw new Error(projectStore.error ?? t("No se pudo importar la imagen."));
      projectStore.setMenuBackground(assetId);
      notice = t("Fondo actualizado.");
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  async function pickMusic() {
    if (!menu) return;
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Música"), extensions: ["mp3", "wav", "flac", "ogg", "ac3"] }],
    });
    if (typeof picked !== "string") return;
    busy = true;
    localError = null;
    try {
      const probe = await unwrap(commands.mediaProbeFile(picked));
      const assetId = projectStore.addAsset(picked, probe, "audio");
      projectStore.setMenuAudio(assetId);
      notice = t("Música del menú actualizada.");
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  function addButton() {
    if (!projectStore.addButton("classicFrame")) {
      localError = projectStore.error ?? t("No queda espacio libre.");
    } else {
      notice = t("Botón añadido.");
    }
  }
</script>

<svelte:window onkeydown={onCanvasKey} />

<section class="relative flex min-h-0 min-w-0 flex-1 flex-col bg-slate-950">
  <div class="flex min-h-9 min-w-0 shrink-0 flex-wrap items-center gap-1.5 border-b border-slate-800 bg-slate-900 px-2 py-1">
    {#if menus.length > 1}
      <select
        class="ui-input ui-input-sm w-44 shrink-0"
        value={menu?.id ?? ""}
        aria-label={t("Menú")}
        onchange={(event) => {
          const id = (event.currentTarget as HTMLSelectElement).value;
          if (id) projectStore.selectMenu(id);
        }}
      >
        {#each menus as item}
          <option value={item.id}>{item.name}</option>
        {/each}
      </select>
    {:else}
      <span class="max-w-44 shrink-0 truncate px-1 text-[13px] font-medium text-slate-100">{menu?.name ?? "—"}</span>
    {/if}
    {#if menu}
      <span class="ui-chip font-mono">{menu.canvas.width}×{menu.canvas.height}</span>
      <span class="mx-1 h-5 w-px bg-slate-800"></span>
      <button type="button" class="ui-btn ui-btn-sm ui-btn-primary" onclick={addButton} title={t("Añadir un botón al menú")}>
        <Icon name="add" size={14} />{t("Botón")}
      </button>
      <button type="button" class="ui-btn ui-btn-sm" onclick={() => projectStore.addTextItem()} title={t("Añadir un título o texto al menú")}>
        <Icon name="text" size={14} />{t("Texto")}
      </button>
      <button
        type="button"
        class="ui-btn ui-btn-sm"
        aria-expanded={templatesOpen}
        onclick={(event) => {
          event.stopPropagation();
          templatesOpen = !templatesOpen;
        }}
      >
        <Icon name="layers" size={14} /><span class="hidden lg:inline">{t("Plantillas")}</span><Icon name="chevronDown" size={12} />
      </button>
      <button type="button" class="ui-btn ui-btn-sm" onclick={pickBackground} disabled={busy} title={t("Imagen de fondo")}>
        <Icon name="photo" size={14} /><span class="hidden 2xl:inline">{t("Fondo")}</span>
      </button>
      <button type="button" class="ui-btn ui-btn-sm" onclick={pickMusic} disabled={busy} title={t("Música del menú")}>
        <Icon name="music" size={14} /><span class="hidden 2xl:inline">{t("Música")}</span>
      </button>
      {#if isRootMenu}
        <!-- Auto-play: Play All when nobody chooses anything. -->
        <button
          type="button"
          class="ui-btn ui-btn-sm"
          aria-pressed={autoPlay}
          class:ui-btn-primary={autoPlay}
          disabled={busy || autoPlayHd}
          onclick={() => projectStore.setMenuAutoPlay(!autoPlay)}
          title={autoPlayHd
            ? t("Solo en DVD-Video: en Blu-ray y AVCHD el menú espera hasta que elijas.")
            : t("Si nadie elige nada, el disco empieza a reproducir todo cuando el menú termina ({seconds} s).", {
                seconds: Math.round((projectStore.activeMenu?.timing.durationMs ?? 30_000) / 1000),
              })}
        >
          <Icon name="play" size={14} /><span class="hidden 2xl:inline">{t("Reproducir solo")}</span>
        </button>
      {/if}

      <div class="ml-auto flex items-center gap-1.5">
        <div class="ui-seg [&>button]:whitespace-nowrap" role="group" aria-label={t("Estado de los botones")}>
          {#each previewStates as state}
            <button type="button" aria-pressed={projectStore.previewState === state.id} onclick={() => (projectStore.previewState = state.id)}>
              {t(state.label)}
            </button>
          {/each}
        </div>
        <button
          type="button"
          class="ui-btn ui-btn-sm ui-btn-icon"
          class:ui-btn-ghost={!projectStore.showSafeAreas}
          aria-pressed={projectStore.showSafeAreas}
          onclick={() => (projectStore.showSafeAreas = !projectStore.showSafeAreas)}
          title={t("Área segura de TV")}
          aria-label={t("Área segura de TV")}
        ><Icon name="layers" size={15} /></button>
        <button
          type="button"
          class="ui-btn ui-btn-sm ui-btn-icon"
          class:ui-btn-ghost={!projectStore.showNavigation}
          aria-pressed={projectStore.showNavigation}
          onclick={() => (projectStore.showNavigation = !projectStore.showNavigation)}
          title={t("Guías de navegación del mando")}
          aria-label={t("Guías de navegación del mando")}
        ><Icon name="remote" size={15} /></button>
      </div>
    {/if}
  </div>

    {#if templatesOpen}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="templates-pop absolute left-2 top-9 z-30 mt-1 w-[min(720px,calc(100%-1rem))] max-h-[70vh] overflow-hidden rounded-md border border-slate-700 bg-slate-900 shadow-xl"
        onpointerdown={(event) => event.stopPropagation()}
      >
        <TemplateGallery onapplied={() => (templatesOpen = false)} />
      </div>
    {/if}

  {#if localError || notice}
    <div
      class={`mx-3 mt-2 rounded-md px-3 py-1.5 text-[12px] ${localError ? "bg-rose-500/10 text-rose-200" : "bg-emerald-500/10 text-emerald-200"}`}
      role={localError ? "alert" : "status"}
    >
      {localError ?? notice}
    </div>
  {/if}

  <div class="flex min-h-0 flex-1 flex-col p-3">
    {#if menu}
      <CanvasStage
        {menu}
        backgroundColor={projectStore.menuBackgroundColor}
        backgroundUrl={projectStore.backgroundPreviewUrl}
        thumbnails={projectStore.thumbnailPreviewUrls}
        selectedButtonId={projectStore.selectedButtonId}
        focusButtonId={projectStore.selectedButtonId}
        previewState={projectStore.previewState}
        showSafeAreas={projectStore.showSafeAreas}
        showNavigation={projectStore.showNavigation}
        onSelect={(id) => {
          projectStore.selectedButtonId = id;
          projectStore.selectedItemId = null;
        }}
        onContextMenu={(_, x, y) => {
          context = {
            x: Math.min(x, window.innerWidth - 270),
            y: Math.min(y, window.innerHeight - 380),
          };
        }}
        selectedItemId={projectStore.selectedItemId}
        onSelectItem={(id) => projectStore.selectItem(id)}
        onMoveItem={(id, rect) => projectStore.updateItem(id, { rect })}
        onChange={(id, rect) => {
          const accepted = projectStore.updateButtonRect(id, rect);
          if (!accepted) localError = t("El botón se solapa o no respeta los 4 px de separación.");
          else localError = null;
          return accepted;
        }}
      />
    {:else}
      <div class="ui-empty flex-1 rounded-lg border border-dashed border-slate-700">
        <Icon name="menu" size={28} class="text-slate-500" />
        <p class="text-slate-200">{t("No hay un menú activo")}</p>
        <p class="ui-hint">{t("Añade un menú al proyecto o abre la pestaña Menú desde un proyecto Blu-ray/AVCHD.")}</p>
      </div>
    {/if}
  </div>

  {#if menu}
    <div class="flex h-7 shrink-0 items-center gap-3 border-t border-slate-800 bg-slate-900 px-3 text-[12px] text-slate-400">
      {#if selected}
        <span class="truncate">
          <span class="text-slate-200">{selected.name}</span> · {t(styleLabel(selected.styleKind))} · {t(actionLabel(selected.action.kind))}
        </span>
      {:else}
        <span>{menu.buttons.length} {t(menu.buttons.length === 1 ? "botón" : "botones")} · {t(menu.audio ? "música activa" : "sin música")}</span>
      {/if}
      <span class="ml-auto hidden truncate text-slate-500 xl:inline" title={t("Teclado: flechas mueven (Mayús = más rápido) · [ ] cambian de botón · Supr borra · Esc deselecciona.")}>
        {t("Flechas mueven · [ ] cambian de botón · Supr borra · Ctrl+D duplica")}
      </span>
    </div>
  {/if}
</section>

{#if context}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="context-menu fixed z-50 w-64 whitespace-nowrap rounded-md border border-slate-700 bg-slate-900 py-1 shadow-xl"
    style={`left:${context.x}px; top:${context.y}px`}
    role="menu"
    tabindex="-1"
    onpointerdown={(event) => event.stopPropagation()}
    oncontextmenu={(event) => event.preventDefault()}
  >
    {#each contextEntries as entry}
      {#if entry === "separator"}
        <div class="my-1 border-t border-slate-800"></div>
      {:else}
        <button
          type="button"
          role="menuitem"
          class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] text-slate-200 hover:bg-slate-800"
          onclick={() => {
            context = null;
            void entry.run();
          }}
        >
          <Icon name={entry.icon} size={14} class="text-slate-400" />
          <span class="flex-1">{t(entry.label)}</span>
          {#if entry.hint}<span class="text-[11px] text-slate-500">{t(entry.hint)}</span>{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .context-menu {
    transform-origin: top left;
    animation: templates-in 120ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  .templates-pop {
    transform-origin: top left;
    animation: templates-in 160ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  @keyframes templates-in {
    from {
      opacity: 0;
      transform: scale(0.98) translateY(-2px);
    }
  }
</style>
