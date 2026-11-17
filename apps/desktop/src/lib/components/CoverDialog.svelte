<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { coverSize, LABEL_MM, renderCover, renderLabel, type CoverContent } from "../editor/cover";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import { ui } from "../ui/state.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const project = $derived(projectStore.project);
  const bluRay = $derived(Boolean(project && project.disc.capacity.startsWith("bd")));
  const size = $derived(coverSize(bluRay));

  let kind = $state<"cover" | "label">("cover");
  let title = $state(projectStore.project?.name ?? "");
  let subtitle = $state("");
  let description = $state("");
  let preview = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function rgb(color: [number, number, number] | number[] | undefined, fallback: string): string {
    if (!color || color.length < 3) return fallback;
    return `#${color.slice(0, 3).map((value) => Number(value).toString(16).padStart(2, "0")).join("")}`;
  }

  /** Colours and font come from the root menu (its template). */
  function content(): CoverContent | null {
    const current = projectStore.project;
    if (!current) return null;
    const root = current.menus.find((menu) => menu.id === current.rootMenu) ?? current.menus[0];
    const background = root?.items.find((item) => item.kind === "shape" && item.id === "tpl:bg");
    const heading = root?.items.find((item) => item.kind === "text" && item.id === "tpl:title");
    const button = root?.buttons[0];
    const total = current.episodes.reduce((sum, episode) => sum + projectStore.episodeEffectiveDuration(episode), 0);
    const video = current.disc.video;
    const format =
      video.kind === "avchd"
        ? `${bluRay ? "Blu-ray" : "AVCHD"} · ${video.resolution}`
        : `DVD-Video · ${current.disc.standard.toUpperCase()}`;
    const minutes = Math.round(total / 60_000);
    return {
      title: title.trim() || current.name,
      subtitle,
      description,
      episodes: current.episodes.map((episode) => ({
        title: episode.title,
        durationMs: projectStore.episodeEffectiveDuration(episode),
        preview: projectStore.episodePreview(episode),
      })),
      specs: `${format} · ${current.disc.aspect === "sixteenNine" ? "16:9" : "4:3"} · ${t("{minutes} min", { minutes })}`,
      background:
        background && background.kind === "shape" ? background.fill : projectStore.menuBackgroundColor,
      text: heading && heading.kind === "text" ? heading.color : (button?.textColor ?? "#ffffff"),
      accent: rgb(root?.subpicture.palette.colors[2]?.rgb, "#d4af37"),
      font: heading && heading.kind === "text" ? heading.fontFamily : (button?.fontFamily ?? "Segoe UI"),
    };
  }

  async function render(): Promise<HTMLCanvasElement | null> {
    const data = content();
    if (!data) return null;
    return kind === "cover" ? renderCover(data, size) : renderLabel(data);
  }

  // Preview follows every change, a moment after typing stops.
  $effect(() => {
    void [kind, title, subtitle, description];
    if (timer) clearTimeout(timer);
    timer = setTimeout(async () => {
      const canvas = await render();
      if (canvas) preview = canvas.toDataURL("image/png");
    }, 300);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  async function savePng() {
    error = null;
    const name = `${(title.trim() || "kinodisc").replace(/[\\/:*?"<>|]/g, "_")} - ${kind === "cover" ? t("carátula") : t("etiqueta")}.png`;
    const path = await save({ defaultPath: name, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (!path) return;
    busy = true;
    try {
      const canvas = await render();
      if (!canvas) return;
      await unwrap(commands.imageSavePng(path, canvas.toDataURL("image/png")));
      ui.notice = t("Guardado a 300 ppp: imprímelo al 100 % (tamaño real).");
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  /** Prints at real size through the system dialog (a page sized in mm). */
  async function printIt() {
    const canvas = await render();
    if (!canvas) return;
    const url = canvas.toDataURL("image/png");
    const width = kind === "cover" ? size.widthMm : LABEL_MM;
    const height = kind === "cover" ? size.heightMm : LABEL_MM;
    const frame = document.createElement("iframe");
    frame.style.position = "fixed";
    frame.style.width = "0";
    frame.style.height = "0";
    frame.style.border = "0";
    document.body.appendChild(frame);
    const doc = frame.contentDocument;
    if (!doc) return;
    doc.open();
    doc.write(
      `<!doctype html><html><head><style>@page{size:A4 landscape;margin:10mm}body{margin:0}img{width:${width}mm;height:${height}mm;display:block}</style></head><body><img src="${url}"></body></html>`,
    );
    doc.close();
    const image = doc.querySelector("img");
    const go = () => {
      frame.contentWindow?.focus();
      frame.contentWindow?.print();
      setTimeout(() => frame.remove(), 1000);
    };
    if (image && !image.complete) image.onload = go;
    else go();
  }
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && !busy && onclose()} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" aria-label={t("Cerrar")} class="ui-backdrop" onclick={() => !busy && onclose()}></button>
  <div class="ui-dialog w-full max-w-5xl" role="dialog" aria-modal="true" aria-labelledby="cover-title">
    <header class="ui-dialog-header">
      <div>
        <h2 id="cover-title" class="ui-title">{t("Carátula y etiqueta")}</h2>
        <p class="ui-hint mt-0.5">{t("Para imprimir a tamaño real, con los colores y la letra del menú.")}</p>
      </div>
      <div class="ui-seg shrink-0 whitespace-nowrap">
        <button type="button" aria-pressed={kind === "cover"} onclick={() => (kind = "cover")}>{t("Carátula")}</button>
        <button type="button" aria-pressed={kind === "label"} onclick={() => (kind = "label")}>{t("Etiqueta del disco")}</button>
      </div>
    </header>

    <div class="grid min-h-0 gap-4 overflow-y-auto px-5 py-4 md:grid-cols-[minmax(0,1fr)_260px]">
      <div class="flex min-h-[260px] items-center justify-center rounded-md border border-slate-800 bg-slate-950 p-3">
        {#if preview}
          <img
            src={preview}
            alt={t("Vista previa")}
            class="max-h-[56vh] max-w-full object-contain shadow-lg {kind === 'label' ? 'rounded-full' : ''}"
          />
        {:else}
          <p class="ui-hint">{t("Preparando…")}</p>
        {/if}
      </div>
      <div class="space-y-3">
        <label class="block">
          <span class="ui-label">{t("Título")}</span>
          <input class="ui-input w-full" bind:value={title} />
        </label>
        <label class="block">
          <span class="ui-label">{t("Subtítulo")}</span>
          <input class="ui-input w-full" bind:value={subtitle} placeholder={t("Por ejemplo, la fecha o el lugar")} />
        </label>
        {#if kind === "cover"}
          <label class="block">
            <span class="ui-label">{t("Texto de la contraportada")}</span>
            <textarea class="ui-input h-28 w-full resize-none py-1.5" bind:value={description}></textarea>
          </label>
        {/if}
        <p class="ui-hint">
          {kind === "cover"
            ? t("Tamaño: {width} × {height} mm (lomo de {spine} mm).", { width: size.widthMm, height: size.heightMm, spine: size.spineMm })
            : t("Tamaño: {size} mm, con el centro en blanco.", { size: LABEL_MM })}
        </p>
        {#if error}
          <p class="text-[12px] text-rose-300" role="alert">{error}</p>
        {/if}
      </div>
    </div>

    <footer class="ui-dialog-footer">
      <button type="button" class="ui-btn ui-btn-ghost" onclick={onclose} disabled={busy}>{t("Cerrar")}</button>
      <button type="button" class="ui-btn" onclick={printIt} disabled={busy || !preview}>{t("Imprimir…")}</button>
      <button type="button" class="ui-btn ui-btn-primary" onclick={savePng} disabled={busy || !preview}>
        <Icon name="save" size={14} />{t("Guardar PNG…")}
      </button>
    </footer>
  </div>
</div>
