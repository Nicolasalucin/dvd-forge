<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import CanvasStage from "../editor/CanvasStage.svelte";
  import TemplateGallery from "./TemplateGallery.svelte";
  import { projectStore } from "../stores/project.svelte";

  let busy = $state(false);
  let localError = $state<string | null>(null);
  let notice = $state<string | null>(null);

  const project = $derived(projectStore.project);
  const menu = $derived(projectStore.activeMenu);
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

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex min-w-0 items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Menú")}</span>
      {#if menus.length > 1}
        <select
          class="max-w-[180px] rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[11px] text-slate-200 outline-none focus:border-cyan-500/60"
          value={menu?.id ?? ""}
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
        <span class="truncate text-[11px] font-medium text-slate-300">{menu?.name ?? "—"}</span>
      {/if}
      {#if menu}
        <span class="hidden rounded-full border border-slate-800 px-2 py-0.5 font-mono text-[9px] text-slate-500 sm:inline">
          {menu.canvas.width}×{menu.canvas.height}
        </span>
      {/if}
    </div>

    <div class="ml-auto flex flex-wrap items-center gap-1.5">
      {#if menu}
        <label class="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-400">
          {t("Fondo")}
          <input
            type="color"
            class="h-4 w-6 cursor-pointer border-0 bg-transparent p-0"
            bind:value={projectStore.menuBackgroundColor}
          />
        </label>
        <button
          type="button"
          onclick={pickBackground}
          disabled={busy}
          class="rounded-lg border border-slate-800 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-50"
        >
          {t("Imagen…")}
        </button>
        <button
          type="button"
          onclick={pickMusic}
          disabled={busy}
          class="rounded-lg border border-slate-800 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-50"
        >
          {t("Música…")}
        </button>
        <span class="mx-1 h-5 w-px bg-slate-800"></span>
        <div class="flex rounded-lg border border-slate-800 bg-slate-900 p-0.5">
          {#each previewStates as state}
            <button
              type="button"
              onclick={() => (projectStore.previewState = state.id)}
              class={`rounded-md px-1.5 py-0.5 text-[9px] font-semibold transition ${
                projectStore.previewState === state.id
                  ? "bg-slate-700 text-white"
                  : "text-slate-500 hover:text-slate-300"
              }`}
            >
              {t(state.label)}
            </button>
          {/each}
        </div>
        <button
          type="button"
          onclick={() => (projectStore.showSafeAreas = !projectStore.showSafeAreas)}
          class={`rounded-lg border px-2 py-1 text-[10px] transition ${
            projectStore.showSafeAreas
              ? "border-cyan-500/40 bg-cyan-500/10 text-cyan-200"
              : "border-slate-800 text-slate-500 hover:text-slate-300"
          }`}
        >
          {t("Área segura")}
        </button>
        <button
          type="button"
          onclick={() => (projectStore.showNavigation = !projectStore.showNavigation)}
          class={`rounded-lg border px-2 py-1 text-[10px] transition ${
            projectStore.showNavigation
              ? "border-cyan-500/40 bg-cyan-500/10 text-cyan-200"
              : "border-slate-800 text-slate-500 hover:text-slate-300"
          }`}
        >
          {t("Guías nav")}
        </button>
        <button
          type="button"
          onclick={addButton}
          class="rounded-lg bg-cyan-500 px-2 py-1 text-[10px] font-bold text-slate-950 transition hover:bg-cyan-400"
        >
          {t("+ Botón")}
        </button>
      {/if}
    </div>
  </div>

  {#if localError || notice}
    <div
      class={`mx-3 mt-2 rounded-lg border px-3 py-1.5 text-[11px] ${
        localError
          ? "border-rose-500/30 bg-rose-500/10 text-rose-200"
          : "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
      }`}
    >
      {localError ?? notice}
    </div>
  {/if}

  <div class="flex min-h-0 flex-1 flex-col p-3">
    {#if menu}
      <div class="flex min-h-0 flex-1 flex-col gap-2">
        <TemplateGallery />
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
          onSelect={(id) => (projectStore.selectedButtonId = id)}
          onChange={(id, rect) => {
            const accepted = projectStore.updateButtonRect(id, rect);
            if (!accepted) localError = t("El botón se solapa o no respeta los 4 px de separación.");
            else localError = null;
            return accepted;
          }}
        />
        <div class="flex flex-wrap items-center justify-between gap-2 px-1 text-[10px] text-slate-500">
          <span>
            {t("Arrastra para mover · tiradores para redimensionar · coordenadas pares y separación mínima de 4 px.")}
            {t("Teclado: flechas mueven (Mayús = más rápido) · [ ] cambian de botón · Supr borra · Esc deselecciona.")}
          </span>
          <span>
            {menu.buttons.length} {t(menu.buttons.length === 1 ? "botón" : "botones")} ·
            {t(menu.audio ? "música activa" : "sin música")}
          </span>
        </div>
      </div>
    {:else}
      <div class="flex flex-1 items-center justify-center rounded-2xl border border-dashed border-slate-800 text-center">
        <div>
          <p class="text-sm font-medium text-slate-300">{t("No hay un menú activo")}</p>
          <p class="mt-1 text-xs text-slate-500">
            {t("Añade un menú al proyecto o abre la pestaña Menú desde un proyecto Blu-ray/AVCHD.")}
          </p>
        </div>
      </div>
    {/if}
  </div>

  {#if selected}
    <div class="shrink-0 border-t border-slate-800/70 px-3 py-1.5 text-[10px] text-slate-500">
      {t("Seleccionado:")} <span class="font-medium text-slate-300">{selected.name}</span>
      · {t(styleLabel(selected.styleKind))} · {t(actionLabel(selected.action.kind))}
    </div>
  {/if}
</section>
