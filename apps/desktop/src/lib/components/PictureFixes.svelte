<script lang="ts">
  import { commands, type Episode_Serialize, type VideoFilters_Serialize } from "../api/bindings";
  import { unwrap } from "../api/client";
  import { t } from "../i18n/index.svelte";
  import { defaultVideoFilters, filtersAreNeutral, projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";

  interface Props {
    episode: Episode_Serialize;
    /** Source time of the frame to preview (the trimmer's playhead). */
    timeMs: number;
  }

  let { episode, timeMs }: Props = $props();

  const filters = $derived<VideoFilters_Serialize>(episode.filters ?? defaultVideoFilters());
  const neutral = $derived(filtersAreNeutral(filters));
  const asset = $derived(projectStore.assetFor(episode.video));
  const video = $derived(asset?.probe.video ?? null);
  const project = $derived(projectStore.project);
  const wide = $derived(project?.disc.aspect !== "fourThree");
  // Blu-ray/AVCHD 1080i and DVD keep interlacing; progressive discs do not.
  const progressive = $derived(
    project?.disc.video.kind === "avchd" ? project.disc.video.resolution !== "1080i" : false,
  );
  // Displayed shape after rotation: is it narrower than the TV picture?
  const narrow = $derived.by(() => {
    if (!video) return false;
    const sideways = filters.rotate === "clockwise" || filters.rotate === "counterClockwise";
    const width = sideways ? video.height : video.width;
    const height = sideways ? video.width : video.height;
    return height > 0 && width / height < (wide ? 16 / 9 : 4 / 3) * 0.97;
  });

  let preview = $state<string | null>(null);
  let previewBusy = $state(false);
  let detecting = $state(false);
  let error = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let token = 0;

  function set(patch: Partial<VideoFilters_Serialize>) {
    projectStore.updateEpisodeFilters(episode.id, patch);
  }

  function rotate(step: 1 | -1) {
    const order = ["none", "clockwise", "half", "counterClockwise"] as const;
    const index = order.indexOf(filters.rotate);
    set({ rotate: order[(index + step + order.length) % order.length] });
  }

  function setCrop(side: "left" | "top" | "right" | "bottom", value: string) {
    const crop = filters.crop ?? { left: 0, top: 0, right: 0, bottom: 0 };
    const number = Math.max(0, Math.min(4000, Math.round(Number(value) || 0)));
    set({ crop: { ...crop, [side]: number } });
  }

  async function detectBorders() {
    const path = asset ? projectStore.assetPath(asset) : null;
    if (!path) return;
    detecting = true;
    error = null;
    try {
      const crop = await unwrap(commands.mediaDetectCrop(path));
      set({ crop });
      if (!crop) error = t("No se encontraron bordes negros.");
    } catch (cause) {
      error = String(cause);
    } finally {
      detecting = false;
    }
  }

  // Preview follows every change, a moment after the last one.
  $effect(() => {
    const current = JSON.stringify(filters);
    const at = Math.max(0, Math.round(timeMs));
    const mode = episode.scaleMode;
    const path = asset ? projectStore.assetPath(asset) : null;
    if (!path) return;
    if (timer) clearTimeout(timer);
    const mine = ++token;
    timer = setTimeout(async () => {
      previewBusy = true;
      try {
        const png = await unwrap(
          commands.mediaFilterPreview(path, at, JSON.parse(current), wide, progressive, mode),
        );
        const url = await unwrap(commands.readImageDataUrl(png));
        if (mine === token) preview = url;
      } catch (cause) {
        if (mine === token) error = String(cause);
      } finally {
        if (mine === token) previewBusy = false;
      }
    }, 350);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  const sliders = [
    { key: "brightness", label: "Brillo" },
    { key: "contrast", label: "Contraste" },
    { key: "saturation", label: "Saturación" },
  ] as const;
</script>

<section class="rounded-xl border border-slate-800 p-2.5" aria-labelledby="picture-title">
  <div class="mb-2 flex items-center justify-between">
    <h3 id="picture-title" class="ui-section-title">{t("Imagen")}</h3>
    {#if !neutral}
      <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost" onclick={() => set(defaultVideoFilters())}>
        {t("Restablecer")}
      </button>
    {/if}
  </div>

  <div class="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
    <div>
      <div class="relative aspect-video overflow-hidden rounded-md border border-slate-800 bg-black">
        {#if preview}
          <img src={preview} alt={t("Vista previa con los ajustes")} class="h-full w-full object-contain" />
        {/if}
        {#if previewBusy}
          <span class="absolute right-1.5 top-1.5 rounded bg-slate-950/80 px-1.5 py-0.5 text-[11px] text-[#fff]">{t("Actualizando…")}</span>
        {/if}
      </div>
      <p class="ui-hint mt-1">{t("Así se verá en el disco, en el fotograma del cabezal.")}</p>
    </div>

    <div class="space-y-2.5">
      <div class="flex flex-wrap items-center gap-1">
        <span class="ui-label w-full">{t("Orientación")}</span>
        <button type="button" class="ui-btn ui-btn-sm" onclick={() => rotate(-1)} title={t("Girar a la izquierda")}>
          <Icon name="undo" size={14} />{t("Izquierda")}
        </button>
        <button type="button" class="ui-btn ui-btn-sm" onclick={() => rotate(1)} title={t("Girar a la derecha")}>
          <Icon name="redo" size={14} />{t("Derecha")}
        </button>
        <button
          type="button"
          class="ui-btn ui-btn-sm"
          aria-pressed={filters.flipHorizontal}
          onclick={() => set({ flipHorizontal: !filters.flipHorizontal })}
        >
          {t("Espejo")}
        </button>
      </div>

      <div>
        <div class="flex items-center justify-between">
          <span class="ui-label">{t("Recortar bordes (px)")}</span>
          <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost" onclick={detectBorders} disabled={detecting}>
            {detecting ? t("Buscando…") : t("Detectar bordes negros")}
          </button>
        </div>
        <div class="mt-1 grid grid-cols-4 gap-1">
          {#each [["left", "Izq."], ["top", "Arriba"], ["right", "Der."], ["bottom", "Abajo"]] as [side, label]}
            <label class="block">
              <span class="block text-[11px] text-slate-500">{t(label)}</span>
              <input
                type="number"
                min="0"
                step="2"
                class="ui-input ui-input-sm w-full font-mono"
                value={filters.crop?.[side as "left"] ?? 0}
                onchange={(event) => setCrop(side as "left", event.currentTarget.value)}
              />
            </label>
          {/each}
        </div>
      </div>

      {#each sliders as slider}
        <label class="block">
          <span class="flex items-center justify-between text-[12px] text-slate-300">
            {t(slider.label)}
            <span class="font-mono text-[11px] text-slate-500">{filters[slider.key] > 0 ? "+" : ""}{filters[slider.key]}</span>
          </span>
          <input
            type="range"
            min="-100"
            max="100"
            step="5"
            class="w-full"
            value={filters[slider.key]}
            ondblclick={() => set({ [slider.key]: 0 })}
            oninput={(event) => set({ [slider.key]: Number(event.currentTarget.value) })}
          />
        </label>
      {/each}

      <div>
        <span class="ui-label">{t("Reducir ruido")}</span>
        <div class="ui-seg mt-1">
          {#each [["off", "No"], ["light", "Suave"], ["strong", "Fuerte"]] as [value, label]}
            <button type="button" aria-pressed={filters.denoise === value} onclick={() => set({ denoise: value as "off" })}>{t(label)}</button>
          {/each}
        </div>
      </div>

      <div>
        <span class="ui-label">{t("Desentrelazar")}</span>
        <div class="ui-seg mt-1">
          {#each [["auto", "Automático"], ["on", "Sí"], ["off", "No"]] as [value, label]}
            <button type="button" aria-pressed={filters.deinterlace === value} onclick={() => set({ deinterlace: value as "auto" })}>{t(label)}</button>
          {/each}
        </div>
        <p class="ui-hint mt-0.5">{t("Automático: solo si el vídeo es entrelazado y el disco no.")}</p>
      </div>

      {#if narrow && (episode.scaleMode === "letterbox" || episode.scaleMode === "contain")}
        <div>
          <span class="ui-label">{t("Laterales del vídeo vertical")}</span>
          <div class="ui-seg mt-1">
            <button type="button" aria-pressed={filters.sideFill === "bars"} onclick={() => set({ sideFill: "bars" })}>{t("Barras negras")}</button>
            <button type="button" aria-pressed={filters.sideFill === "blur"} onclick={() => set({ sideFill: "blur" })}>{t("Fondo desenfocado")}</button>
          </div>
        </div>
      {/if}

      {#if !neutral && project?.disc.video.kind === "avchd"}
        <p class="ui-hint">{t("Con ajustes de imagen el vídeo se recodifica (no se copia).")}</p>
      {/if}
      {#if error}
        <p class="text-[12px] text-amber-300">{error}</p>
      {/if}
    </div>
  </div>
</section>
