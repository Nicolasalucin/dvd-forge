<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import { basename } from "../utils/format";

  let { onclose }: { onclose: () => void } = $props();

  let images = $state<string[]>([]);
  let music = $state<string | null>(null);
  let seconds = $state(5);
  let fade = $state(1);
  let title = $state(t("Presentación de fotos"));
  let working = $state(false);
  let error = $state<string | null>(null);

  const totalSeconds = $derived(images.length > 0 ? images.length * seconds + fade : 0);
  const canCreate = $derived(
    !working && images.length > 0 && seconds >= 1 && fade >= 0 && fade < seconds,
  );

  function minutes(value: number): string {
    const whole = Math.round(value);
    return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
  }

  async function pickImages() {
    const picked = await open({
      multiple: true,
      filters: [{ name: t("Fotos"), extensions: ["jpg", "jpeg", "png", "webp", "bmp", "tif", "tiff"] }],
    });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    // Keep the chosen order; add new photos after the existing ones.
    images = [...images, ...list.filter((path) => !images.includes(path))];
  }

  async function pickMusic() {
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Música"), extensions: ["mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "ac3"] }],
    });
    if (typeof picked === "string") music = picked;
  }

  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= images.length) return;
    const next = [...images];
    [next[index], next[target]] = [next[target], next[index]];
    images = next;
  }

  async function create() {
    working = true;
    error = null;
    const ok = await projectStore.createSlideshow({
      images,
      secondsPerImage: seconds,
      crossfadeSeconds: fade,
      music,
      title: title.trim() || t("Presentación de fotos"),
    });
    working = false;
    if (ok) onclose();
    else error = projectStore.error;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape" && !working) onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar")}
    class="absolute inset-0 cursor-default bg-slate-950/80"
    onclick={() => !working && onclose()}
  ></button>
  <div
    class="relative z-10 flex max-h-[90vh] w-full max-w-xl flex-col overflow-hidden rounded-xl border border-slate-700 bg-slate-950"
    role="dialog"
    aria-modal="true"
    aria-labelledby="slideshow-title"
  >
    <header class="border-b border-slate-800 px-5 py-4">
      <h2 id="slideshow-title" class="text-base font-semibold text-white">{t("Presentación de fotos")}</h2>
      <p class="mt-1 text-xs text-slate-400">
        {t("Las fotos se convierten en un vídeo con fundidos y se añade como episodio.")}
      </p>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-5 py-4 text-sm text-slate-200">
      <label class="flex flex-col gap-1">
        <span class="text-xs text-slate-400">{t("Título del episodio")}</span>
        <input
          class="rounded-md border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-sm outline-none focus:border-cyan-500"
          bind:value={title}
        />
      </label>

      <div class="flex items-center justify-between gap-3">
        <span class="text-xs text-slate-400">
          {t("{count} fotos · {duration}", { count: images.length, duration: minutes(totalSeconds) })}
        </span>
        <button
          type="button"
          class="rounded-md border border-slate-600 px-3 py-1.5 text-sm hover:border-cyan-500"
          onclick={pickImages}
          disabled={working}
        >{t("Añadir fotos…")}</button>
      </div>

      {#if images.length > 0}
        <ol class="max-h-48 overflow-y-auto rounded-md border border-slate-800">
          {#each images as image, index (image)}
            <li class="flex items-center gap-2 border-b border-slate-800/70 px-2.5 py-1 last:border-0">
              <span class="w-6 text-right text-xs tabular-nums text-slate-500">{index + 1}</span>
              <span class="min-w-0 flex-1 truncate text-xs" title={image}>{basename(image)}</span>
              <button type="button" class="px-1 text-xs text-slate-400 hover:text-white" aria-label={t("Subir")} onclick={() => move(index, -1)} disabled={index === 0}>↑</button>
              <button type="button" class="px-1 text-xs text-slate-400 hover:text-white" aria-label={t("Bajar")} onclick={() => move(index, 1)} disabled={index === images.length - 1}>↓</button>
              <button type="button" class="px-1 text-xs text-slate-400 hover:text-rose-300" aria-label={t("Quitar")} onclick={() => (images = images.filter((_, i) => i !== index))}>✕</button>
            </li>
          {/each}
        </ol>
      {/if}

      <div class="grid grid-cols-2 gap-3">
        <label class="flex flex-col gap-1">
          <span class="text-xs text-slate-400">{t("Segundos por foto")}</span>
          <input type="number" min="1" max="60" step="0.5" bind:value={seconds}
            class="rounded-md border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-sm outline-none focus:border-cyan-500" />
        </label>
        <label class="flex flex-col gap-1">
          <span class="text-xs text-slate-400">{t("Fundido (segundos)")}</span>
          <input type="number" min="0" max="3" step="0.25" bind:value={fade}
            class="rounded-md border border-slate-700 bg-slate-900 px-2.5 py-1.5 text-sm outline-none focus:border-cyan-500" />
        </label>
      </div>

      <div class="flex items-center justify-between gap-3">
        <span class="min-w-0 truncate text-xs text-slate-400" title={music ?? ""}>
          {music ? t("Música: {name}", { name: basename(music) }) : t("Sin música (pista de audio en silencio)")}
        </span>
        <div class="flex shrink-0 gap-2">
          {#if music}
            <button type="button" class="rounded-md px-2 py-1.5 text-xs text-slate-400 hover:text-white" onclick={() => (music = null)} disabled={working}>{t("Quitar")}</button>
          {/if}
          <button type="button" class="rounded-md border border-slate-600 px-3 py-1.5 text-sm hover:border-cyan-500" onclick={pickMusic} disabled={working}>{t("Elegir música…")}</button>
        </div>
      </div>

      {#if error}
        <p class="rounded-md bg-rose-500/10 px-3 py-2 text-xs text-rose-200" role="alert">{error}</p>
      {/if}
    </div>

    <footer class="flex justify-end gap-2 border-t border-slate-800 px-5 py-3">
      <button type="button" class="rounded-md px-3 py-1.5 text-sm text-slate-300 hover:text-white" onclick={onclose} disabled={working}>{t("Cancelar")}</button>
      <button
        type="button"
        class="rounded-md bg-cyan-500 px-3 py-1.5 text-sm font-semibold text-slate-950 hover:bg-cyan-400 disabled:opacity-50"
        onclick={create}
        disabled={!canCreate}
      >{working ? t("Creando…") : t("Crear y añadir")}</button>
    </footer>
  </div>
</div>
