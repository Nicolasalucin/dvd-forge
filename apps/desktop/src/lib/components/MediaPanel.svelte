<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
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
  const selectedButton = $derived(projectStore.selectedButton);
  const selectedEpisode = $derived(projectStore.selectedEpisode);
  const assets = $derived(
    projectStore.assets.filter((asset) => {
      const term = search.trim().toLowerCase();
      if (!term) return true;
      return (
        asset.displayName.toLowerCase().includes(term) ||
        asset.kind.includes(term) ||
        asset.probe.formatName.toLowerCase().includes(term)
      );
    }),
  );

  const tabs = [
    { id: "media" as const, label: "Medios", icon: "▣" },
    { id: "layouts" as const, label: "Plantillas", icon: "▦" },
    { id: "styles" as const, label: "Estilos", icon: "✦" },
  ];

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
    if (!project) return;
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: "Vídeo, audio e imagen",
          extensions: [
            "mp4", "mkv", "mov", "avi", "webm", "mpg", "mpeg",
            "mp3", "wav", "flac", "ogg", "ac3",
            "png", "jpg", "jpeg", "webp",
          ],
        },
        { name: "Todos los archivos", extensions: ["*"] },
      ],
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    busy = true;
    localError = null;
    notice = null;
    try {
      const count = await projectStore.importMedia(paths);
      notice = t("{count} archivo(s) importados.", { count });
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
          projectStore.previewUrls = { ...projectStore.previewUrls, [assetId]: preview };
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

<aside class="flex min-h-0 w-[300px] shrink-0 flex-col border-r border-slate-800/80 bg-slate-950/70">
  <div class="flex shrink-0 items-center gap-1 border-b border-slate-800/80 p-2">
    {#each tabs as tab}
      <button
        type="button"
        onclick={() => (projectStore.mediaTab = tab.id)}
        class={`flex flex-1 items-center justify-center gap-1.5 whitespace-nowrap rounded-lg px-2 py-1.5 text-[11px] font-semibold transition ${
          projectStore.mediaTab === tab.id
            ? "bg-slate-800 text-white"
            : "text-slate-500 hover:bg-slate-900 hover:text-slate-300"
        }`}
      >
        <span class="text-xs">{tab.icon}</span>
        {t(tab.label)}
      </button>
    {/each}
    <button
      type="button"
      onclick={() => (projectStore.leftCollapsed = true)}
      class="shrink-0 rounded-lg border border-slate-800 px-1.5 py-1.5 text-[10px] text-slate-500 transition hover:border-slate-700 hover:text-slate-200"
      title={t("Ocultar biblioteca")}
    >
      «
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto p-3">
    {#if localError || notice}
      <div
        class={`mb-3 rounded-lg border px-2.5 py-2 text-[11px] leading-5 ${
          localError
            ? "border-rose-500/30 bg-rose-500/10 text-rose-200"
            : "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
        }`}
      >
        {localError ?? notice}
      </div>
    {/if}

    {#if projectStore.mediaTab === "media"}
      <div class="flex items-center gap-2">
        <input
          class="min-w-0 flex-1 rounded-lg border border-slate-800 bg-slate-900 px-2.5 py-1.5 text-[11px] text-slate-200 outline-none placeholder:text-slate-600 focus:border-cyan-500/60"
          placeholder={t("Buscar medios…")}
          bind:value={search}
        />
        <button
          type="button"
          onclick={importMedia}
          disabled={busy || !project}
          class="shrink-0 rounded-lg bg-cyan-500 px-2.5 py-1.5 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-wait disabled:opacity-50"
        >
          {busy ? "…" : t("+ Importar")}
        </button>
      </div>

      {#if !project}
        <p class="mt-8 px-2 text-center text-xs leading-5 text-slate-500">
          {t("Crea o abre un proyecto para importar medios.")}
        </p>
      {:else if assets.length === 0}
        <div class="mt-8 rounded-xl border border-dashed border-slate-800 px-3 py-8 text-center">
          <p class="text-xs font-medium text-slate-300">{t("Sin medios")}</p>
          <p class="mt-1 text-[11px] leading-5 text-slate-500">
            {t("Importa vídeo, audio o imágenes para empezar.")}
          </p>
        </div>
      {:else}
        <ul class="mt-3 space-y-2">
          {#each assets as asset}
            {@const episode = projectStore.episodeForAsset(asset.id)}
            {@const preview = projectStore.previewForAsset(asset.id)}
            <li class="rounded-xl border border-slate-800 bg-slate-900/50 p-2.5">
              <div class="flex gap-2.5">
                <div class="flex h-12 w-16 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-slate-800 bg-slate-950">
                  {#if preview}
                    <img src={preview} alt="" class="h-full w-full object-cover" />
                  {:else}
                    <span class="text-lg text-slate-600">
                      {asset.kind === "video" ? "▷" : asset.kind === "audio" ? "♫" : "▧"}
                    </span>
                  {/if}
                </div>
                <div class="min-w-0 flex-1">
                  <p class="truncate text-[11px] font-semibold text-slate-200" title={asset.displayName}>
                    {basename(asset.displayName)}
                  </p>
                  <p class="mt-0.5 truncate text-[10px] text-slate-500">{assetMeta(asset)}</p>
                  <div class="mt-1 flex items-center gap-1.5">
                    <span class="rounded-full border border-slate-700 px-1.5 py-0.5 text-[9px] uppercase tracking-wider text-slate-400">
                      {kindLabel(asset.kind)}
                    </span>
                    {#if episode}
                      <span class="rounded-full border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[9px] text-emerald-300">
                        E{episode.number}
                      </span>
                    {/if}
                  </div>
                </div>
              </div>

              <div class="mt-2 flex flex-wrap gap-1.5">
                {#if asset.kind === "video"}
                  <button
                    type="button"
                    onclick={() => addEpisode(asset)}
                    class="rounded-md border border-slate-700 px-2 py-1 text-[10px] font-medium text-slate-200 transition hover:border-cyan-500/60 hover:text-white"
                  >
                    {episode ? t("Seleccionar episodio") : t("+ Episodio")}
                  </button>
                {/if}
                {#if !isAvchd && (asset.kind === "image" || asset.kind === "video")}
                  <button
                    type="button"
                    onclick={() => setButtonThumbnail(asset)}
                    disabled={!selectedButton}
                    class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-40"
                  >
                    {t("Miniatura")}
                  </button>
                {/if}
                {#if asset.kind === "image"}
                  <button
                    type="button"
                    onclick={() => setBackground(asset)}
                    class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
                  >
                    {t("Fondo")}
                  </button>
                {/if}
                {#if asset.kind === "video"}
                  <button
                    type="button"
                    onclick={() => setVideoBackground(asset)}
                    class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
                  >
                    {t("Fondo vídeo")}
                  </button>
                {/if}
                {#if asset.kind === "audio"}
                  <button
                    type="button"
                    onclick={() => setMusic(asset)}
                    class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
                  >
                    {t("Música menú")}
                  </button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if projectStore.mediaTab === "layouts"}
      <p class="px-1 text-[11px] leading-5 text-slate-500">
        {t("Genera una composición completa a partir de los episodios y ajusta después en el lienzo.")}
      </p>
      <div class="mt-3 space-y-2">
        {#each layouts as layout}
          <button
            type="button"
            onclick={() => applyLayout(layout.kind)}
            disabled={!project || isAvchd}
            class="group w-full rounded-xl border border-slate-800 bg-slate-900/40 p-2.5 text-left transition hover:border-cyan-500/50 hover:bg-slate-900 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <div class="flex items-center gap-2.5">
              <div class="grid h-10 w-14 shrink-0 grid-rows-2 gap-0.5 rounded-md border border-slate-700 bg-slate-950 p-1">
                <div class="grid grid-cols-2 gap-0.5">
                  <span class="rounded-sm bg-cyan-500/25"></span>
                  <span class="rounded-sm bg-slate-700"></span>
                  <span class="rounded-sm bg-slate-700"></span>
                  <span class="rounded-sm bg-cyan-500/15"></span>
                </div>
                <div class={`grid gap-0.5 ${layout.preview}`}>
                  <span class="rounded-sm bg-slate-700/80"></span>
                  <span class="rounded-sm bg-slate-700/50"></span>
                </div>
              </div>
              <div class="min-w-0">
                <p class="text-[11px] font-semibold text-slate-200">{t(layout.title)}</p>
                <p class="mt-0.5 text-[10px] text-slate-500">{t(layout.hint)}</p>
              </div>
            </div>
          </button>
        {/each}
      </div>
      {#if isAvchd}
        <p class="mt-3 rounded-lg border border-cyan-500/20 bg-cyan-500/10 px-2.5 py-2 text-[10px] leading-5 text-cyan-100/80">
          {t("AVCHD usa reproducción continua; las plantillas de menú no aplican.")}
        </p>
      {/if}
    {:else}
      <p class="px-1 text-[11px] leading-5 text-slate-500">
        {selectedButton
          ? t("Aplica un estilo a «{name}».", { name: selectedButton.name })
          : "Selecciona un botón o crea uno nuevo con el estilo elegido."}
      </p>
      <div class="mt-3 space-y-2">
        {#each styles as style}
          <button
            type="button"
            onclick={() => applyStyle(style.kind)}
            disabled={!project || isAvchd}
            class={`w-full rounded-xl border p-2.5 text-left transition disabled:cursor-not-allowed disabled:opacity-40 ${
              selectedButton?.styleKind === style.kind
                ? "border-cyan-500/60 bg-cyan-500/10"
                : "border-slate-800 bg-slate-900/40 hover:border-cyan-500/40 hover:bg-slate-900"
            }`}
          >
            <div class="flex items-center gap-2.5">
              <div class="flex h-9 w-16 shrink-0 items-center justify-center rounded-md border border-slate-700 bg-slate-950">
                {#if style.kind === "thumbnailCard"}
                  <span class="h-6 w-10 rounded-sm border border-slate-600 bg-gradient-to-br from-slate-700 to-slate-900"></span>
                {:else if style.kind === "playBadge"}
                  <span class="flex h-5 w-5 items-center justify-center rounded-full bg-slate-200 text-[9px] text-slate-950">▶</span>
                {:else if style.kind === "glassPill"}
                  <span class="h-4 w-11 rounded-full border border-white/30 bg-white/10"></span>
                {:else if style.kind === "classicFrame"}
                  <span class="h-5 w-12 rounded-sm border-2 border-slate-400/70 bg-slate-800"></span>
                {:else if style.kind === "textOnly"}
                  <span class="text-[9px] font-semibold text-slate-300">{t("Texto")}</span>
                {:else}
                  <span class="h-4 w-12 border-l-2 border-cyan-400/70 bg-slate-900/30"></span>
                {/if}
              </div>
              <div class="min-w-0">
                <p class="text-[11px] font-semibold text-slate-200">{t(style.title)}</p>
                <p class="mt-0.5 text-[10px] text-slate-500">{t(style.hint)}</p>
              </div>
            </div>
          </button>
        {/each}
      </div>
      {#if !selectedButton && project && !isAvchd}
        <button
          type="button"
          onclick={() => {
            if (!projectStore.addButton("classicFrame")) localError = projectStore.error;
          }}
          class="mt-3 w-full rounded-lg border border-slate-700 px-2.5 py-2 text-[11px] font-medium text-slate-200 transition hover:border-cyan-500/60"
        >
          {t("+ Añadir botón clásico")}
        </button>
      {/if}
    {/if}
  </div>
</aside>
