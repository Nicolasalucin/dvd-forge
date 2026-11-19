<script lang="ts" module>
  /** Filmstrips already extracted this session, by source path. */
  const filmstripCache = new Map<string, Array<string | null>>();
</script>

<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import * as actions from "../actions";
  import { ui } from "../ui/state.svelte";

  /** Imports videos and adds each one as an episode right away. */
  async function importAndAdd() {
    const before = new Set(projectStore.assets.map((asset) => asset.id));
    await actions.importMedia();
    for (const asset of projectStore.assets) {
      if (!before.has(asset.id) && asset.kind === "video" && !projectStore.episodeForAsset(asset.id)) {
        projectStore.addEpisodeFromAsset(asset.id);
      }
    }
    const first = projectStore.episodes[0];
    if (first) projectStore.selectEpisode(first.id);
  }
  import { t } from "../i18n/index.svelte";
  import PictureFixes from "./PictureFixes.svelte";
  import { onMount } from "svelte";
  import {
    commands,
    type Chapter_Serialize,
    type PlaybackSupport_Serialize,
    type VideoScaleMode,
  } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { loadPlaybackSupport, previewUrlFor } from "../playback";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";

  let videoEl: HTMLVideoElement | undefined = $state();
  let timelineEl: HTMLDivElement | undefined = $state();
  let dragging: "in" | "out" | "playhead" | null = null;
  let previewPath = $state<string | null>(null);
  let videoSrc = $state<string | null>(null);
  let preparing = $state(false);
  let playing = $state(false);
  let videoFullscreen = $state(false);
  let currentTimeMs = $state(0);
  let localError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let playbackSupport = $state<PlaybackSupport_Serialize | null>(null);
  let prepareToken = 0;
  let seekFrame: number | null = null;

  const episode = $derived(projectStore.selectedEpisode);
  const sourcePath = $derived(episode ? projectStore.episodeSourcePath(episode) : null);
  const sourceDuration = $derived(episode ? projectStore.episodeSourceDuration(episode) : 0);

  // Filmstrip: 10 frames across the source, extracted once per file.
  const FILMSTRIP_FRAMES = 10;
  let filmstrip = $state<Array<string | null>>(Array(FILMSTRIP_FRAMES).fill(null));
  $effect(() => {
    const path = sourcePath;
    const duration = sourceDuration;
    filmstrip = Array(FILMSTRIP_FRAMES).fill(null);
    if (!path || duration <= 0) return;
    let cancelled = false;
    const cached = filmstripCache.get(path);
    if (cached) {
      filmstrip = cached;
      return;
    }
    void (async () => {
      const frames: Array<string | null> = Array(FILMSTRIP_FRAMES).fill(null);
      for (let index = 0; index < FILMSTRIP_FRAMES; index += 1) {
        if (cancelled) return;
        const at = Math.round((duration * (index + 0.5)) / FILMSTRIP_FRAMES);
        try {
          const png = await unwrap(commands.mediaExtractFrame(path, at, null));
          frames[index] = await unwrap(commands.readImageDataUrl(png));
        } catch {
          frames[index] = null;
        }
        if (!cancelled) filmstrip = [...frames];
      }
      filmstripCache.set(path, frames);
    })();
    return () => {
      cancelled = true;
    };
  });
  const startMs = $derived(episode?.trimStartMs ?? 0);
  const endMs = $derived(episode?.trimEndMs ?? sourceDuration);
  const effectiveMs = $derived(Math.max(0, endMs - startMs));
  const thumbnail = $derived(episode ? projectStore.episodePreview(episode) : null);
  const scaleModes: Array<{ id: VideoScaleMode; label: string; hint: string }> = [
    { id: "contain", label: "Ajustar", hint: "Barras negras, sin recortar" },
    { id: "cover", label: "Rellenar", hint: "Llena el marco recortando" },
    { id: "stretch", label: "Estirar", hint: "Estira la imagen" },
    { id: "letterbox", label: "Buzón (heredado)", hint: "Compatibilidad" },
    { id: "cropFill", label: "Recorte y relleno (heredado)", hint: "Compatibilidad" },
  ];
  const scaleHint = $derived(
    t(scaleModes.find((mode) => mode.id === episode?.scaleMode)?.hint ?? ""),
  );

  onMount(() => {
    let active = true;
    void loadPlaybackSupport().then((support) => {
      if (active) playbackSupport = support;
    });
    const onFullscreenChange = () => {
      videoFullscreen = Boolean(videoEl) && document.fullscreenElement === videoEl;
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () => {
      active = false;
      document.removeEventListener("fullscreenchange", onFullscreenChange);
    };
  });

  $effect(() => {
    const support = playbackSupport;
    const path = sourcePath;
    const token = ++prepareToken;
    previewPath = null;
    videoSrc = null;
    preparing = false;
    playing = false;
    currentTimeMs = 0;
    if (!path || !support?.supported) return;
    preparing = true;
    void (async () => {
      try {
        const prepared = await unwrap(commands.mediaPreparePreview(path));
        const url = await previewUrlFor(prepared);
        if (token === prepareToken) {
          previewPath = prepared;
          videoSrc = url;
        }
      } catch (cause) {
        if (token === prepareToken) localError = messageOf(cause);
      } finally {
        if (token === prepareToken) preparing = false;
      }
    })();
  });

  $effect(() => {
    const video = videoEl;
    const path = previewPath;
    const target = startMs;
    if (!video || !path) return;
    // Dragging the in-point updates startMs on every pointermove. Coalesce the
    // seeks to one per animation frame instead of hammering the decoder.
    if (seekFrame !== null) cancelAnimationFrame(seekFrame);
    seekFrame = requestAnimationFrame(() => {
      seekFrame = null;
      if (!videoEl || !previewPath) return;
      videoEl.currentTime = target / 1000;
      currentTimeMs = target;
    });
    return () => {
      if (seekFrame !== null) {
        cancelAnimationFrame(seekFrame);
        seekFrame = null;
      }
    };
  });

  function onLoadedMetadata() {
    if (!videoEl) return;
    localError = null;
    videoEl.currentTime = startMs / 1000;
    currentTimeMs = startMs;
  }

  function onVideoError() {
    const error = videoEl?.error;
    localError = t("No se pudo cargar el vídeo (código {code}): {detail}. Origen: {source}", {
      code: error?.code ?? "?",
      detail: error?.message || t("sin detalle de WebKit"),
      source: videoEl?.currentSrc || videoSrc || t("desconocido"),
    });
  }

  function onTimeUpdate() {
    if (!videoEl) return;
    currentTimeMs = Math.round(videoEl.currentTime * 1000);
    if (playing && currentTimeMs >= endMs) {
      videoEl.pause();
      playing = false;
      currentTimeMs = endMs;
    }
  }

  async function togglePlay() {
    if (!videoEl) return;
    if (playing) {
      videoEl.pause();
      return;
    }
    if (currentTimeMs < startMs || currentTimeMs >= endMs) {
      videoEl.currentTime = startMs / 1000;
      currentTimeMs = startMs;
    }
    try {
      await videoEl.play();
    } catch (cause) {
      localError = messageOf(cause);
    }
  }

  async function toggleVideoFullscreen() {
    if (!videoEl) return;
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await videoEl.requestFullscreen();
      }
    } catch (cause) {
      localError = messageOf(cause);
    }
  }

  function seek(ms: number) {
    const target = Math.max(0, Math.min(sourceDuration, ms));
    if (videoEl) videoEl.currentTime = target / 1000;
    currentTimeMs = target;
  }

  function setStart(ms: number) {
    if (!episode) return;
    if (!projectStore.setEpisodeTrimStart(episode.id, ms)) {
      showStoreError();
    } else {
      localError = null;
      notice = t("Inicio en {time}.", { time: formatMs(ms) });
    }
  }

  function setEnd(ms: number) {
    if (!episode) return;
    if (!projectStore.setEpisodeTrimEnd(episode.id, ms)) {
      showStoreError();
    } else {
      localError = null;
      notice = t("Fin en {time}.", { time: formatMs(ms) });
    }
  }

  function resetTrim() {
    if (!episode) return;
    projectStore.resetEpisodeTrim(episode.id);
    localError = null;
    notice = t("Recorte restablecido.");
  }

  function markStart() {
    setStart(currentTimeMs);
  }

  function markEnd() {
    setEnd(currentTimeMs);
  }

  function setScaleMode(mode: VideoScaleMode) {
    if (!episode) return;
    projectStore.setEpisodeScaleMode(episode.id, mode);
  }

  function setAudio(sourceStream: number) {
    if (!episode) return;
    projectStore.setEpisodeAudio(episode.id, sourceStream);
  }

  function addChapter() {
    if (!episode) return;
    if (projectStore.addChapterAt(episode.id, currentTimeMs)) {
      notice = t("Capítulo añadido.");
      localError = null;
    } else {
      localError = t("El capítulo está fuera del rango o demasiado cerca de otro.");
    }
  }

  function autoChapters(mode: "5min" | "10min" | "quarters") {
    if (!episode) return;
    const count = projectStore.autoChapters(episode.id, mode);
    localError = null;
    notice =
      count > 0
        ? t("{count} capítulos generados.", { count })
        : t("El vídeo dura menos de {minutes} min: los capítulos no cambiaron.", {
            minutes: mode === "10min" ? 10 : 5,
          });
  }

  let detectingScenes = $state(false);

  async function detectScenes() {
    if (!episode || detectingScenes) return;
    detectingScenes = true;
    localError = null;
    try {
      const count = await projectStore.sceneChapters(episode.id);
      if (count !== null && count <= 1) notice = t("No se encontraron cambios de escena claros; prueba «Cada 5 min» o añádelos a mano.");
      else if (count !== null) notice = t("{count} capítulos en los cambios de escena.", { count });
      else if (projectStore.error) showStoreError();
    } finally {
      detectingScenes = false;
    }
  }

  function importChapters() {
    if (!episode) return;
    const count = projectStore.importMediaChapters(episode.id);
    if (count > 0) {
      notice = t("Se importaron {count} capítulo(s) del archivo.", { count });
      localError = null;
    } else {
      localError = t("El archivo no contiene capítulos dentro del rango activo.");
    }
  }

  function removeChapter(chapter: Chapter_Serialize) {
    if (!episode) return;
    projectStore.removeChapter(episode.id, chapter.id);
  }

  function chapterSourceTime(chapter: Chapter_Serialize): number {
    return episode ? startMs + projectStore.chapterTimeMs(episode, chapter) : 0;
  }

  async function captureThumbnail() {
    if (!episode || !sourcePath) return;
    localError = null;
    try {
      const framePath = await unwrap(
        commands.mediaExtractFrame(sourcePath, Math.max(0, Math.round(currentTimeMs)), null),
      );
      const dataUrl = await unwrap(commands.readImageDataUrl(framePath));
      projectStore.setEpisodeThumbnail(episode.id, framePath, dataUrl);
      projectStore.updateEpisode(episode.id, {
        thumbnailTimeMs: Math.max(0, Math.round(currentTimeMs)),
      });
      notice = t("Miniatura del episodio actualizada.");
    } catch (cause) {
      localError = messageOf(cause);
    }
  }

  /**
   * Shows the store's last error in this panel's banner instead of the
   * header, so one mistake leaves one message that the next success clears.
   */
  function showStoreError() {
    localError = projectStore.error;
    notice = null;
    projectStore.error = null;
  }

  // Success messages fade on their own; errors stay until the next success
  // or until closed.
  $effect(() => {
    if (!notice || localError) return;
    const shown = notice;
    const timer = setTimeout(() => {
      if (notice === shown) notice = null;
    }, 4000);
    return () => clearTimeout(timer);
  });

  function formatMs(ms: number): string {
    const totalSeconds = Math.max(0, Math.floor(ms / 1000));
    const hours = Math.floor(totalSeconds / 3600);
    const minutes = Math.floor((totalSeconds % 3600) / 60);
    const seconds = totalSeconds % 60;
    const millis = Math.max(0, Math.round(ms % 1000));
    return `${hours > 0 ? `${String(hours).padStart(2, "0")}:` : ""}${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${String(millis).padStart(3, "0")}`;
  }

  function percent(ms: number): number {
    return sourceDuration > 0 ? Math.max(0, Math.min(100, (ms / sourceDuration) * 100)) : 0;
  }

  function timelineMsAt(clientX: number): number {
    if (!timelineEl || sourceDuration <= 0) return 0;
    const rect = timelineEl.getBoundingClientRect();
    const ratio = rect.width > 0 ? (clientX - rect.left) / rect.width : 0;
    return Math.max(0, Math.min(sourceDuration, Math.round(ratio * sourceDuration)));
  }

  function startTimelineDrag(event: PointerEvent): void {
    if (!episode || !timelineEl) return;
    const ms = timelineMsAt(event.clientX);
    const startDistance = Math.abs(percent(ms) - percent(startMs));
    const endDistance = Math.abs(percent(ms) - percent(endMs));
    const playheadDistance = Math.abs(percent(ms) - percent(currentTimeMs));
    if (startDistance <= 4 && startDistance <= endDistance) {
      dragging = "in";
    } else if (endDistance <= 4 && endDistance <= playheadDistance) {
      dragging = "out";
    } else {
      dragging = "playhead";
      seek(ms);
    }
    timelineEl.setPointerCapture(event.pointerId);
  }

  function moveTimelineDrag(event: PointerEvent): void {
    if (!dragging) return;
    const ms = timelineMsAt(event.clientX);
    if (dragging === "in") {
      setStart(Math.min(ms, Math.max(0, endMs - 500)));
    } else if (dragging === "out") {
      setEnd(Math.max(ms, Math.min(sourceDuration, startMs + 500)));
    } else {
      seek(ms);
    }
  }

  function endTimelineDrag(event: PointerEvent): void {
    dragging = null;
    if (timelineEl?.hasPointerCapture(event.pointerId)) {
      timelineEl.releasePointerCapture(event.pointerId);
    }
  }

  function onTimelineKeydown(event: KeyboardEvent): void {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      seek(currentTimeMs - 1_000);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      seek(currentTimeMs + 1_000);
    } else if (event.key === "Home") {
      event.preventDefault();
      seek(startMs);
    } else if (event.key === "End") {
      event.preventDefault();
      seek(endMs);
    }
  }
</script>

<section class="relative flex min-h-0 min-w-0 flex-1 flex-col bg-slate-950">
  <div class="flex min-h-9 shrink-0 flex-wrap items-center justify-between gap-2 border-b border-slate-800 bg-slate-900 px-3 py-1">
    <div class="flex min-w-0 items-center gap-2">
      <span class="ui-section-title">{t("Edición")}</span>
      {#if episode}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[11px] font-semibold text-cyan-200">
          E{episode.number}
        </span>
        <span class="truncate text-[12px] font-medium text-slate-200">{episode.title}</span>
      {/if}
    </div>
    {#if episode}
      <div class="flex flex-wrap items-center gap-2 text-[11px] text-slate-500">
        <span>{t("Fuente")} {formatDuration(sourceDuration)}</span>
        <span class="text-slate-700">→</span>
        <span class="font-semibold text-cyan-300">{t("Recorte")} {formatDuration(effectiveMs)}</span>
        <button
          type="button"
          onclick={resetTrim}
          class="ui-btn ui-btn-sm"
        >
          {t("Restablecer")}
        </button>
      </div>
    {/if}
  </div>

  {#if localError || notice}
    <!-- Floats over the content so showing or hiding it never moves the layout. -->
    <div
      class={`absolute left-5 top-12 z-30 flex max-w-[min(32rem,calc(100%-1.5rem))] items-center gap-2 rounded-lg border px-3 py-1.5 text-[12px] shadow-lg ${
        localError
          ? "border-rose-500/40 bg-slate-950 text-rose-200"
          : "border-emerald-500/40 bg-slate-950 text-emerald-200"
      }`}
      role={localError ? "alert" : "status"}
    >
      <span class="min-w-0 flex-1">{localError ?? notice}</span>
      <button
        type="button"
        class="shrink-0 opacity-70 hover:opacity-100"
        onclick={() => {
          localError = null;
          notice = null;
        }}
        aria-label={t("Cerrar")}
      >
        <Icon name="close" size={14} />
      </button>
    </div>
  {/if}

  {#if !episode}
    <div class="flex flex-1 items-center justify-center p-6">
      {#if projectStore.episodes.length === 0}
        <div class="max-w-md text-center">
          <Icon name="film" size={40} class="mx-auto text-slate-500" />
          <h2 class="mt-3 text-[18px] font-semibold text-slate-100">{t("Empieza añadiendo tus vídeos")}</h2>
          <p class="mt-1 text-[13px] leading-5 text-slate-400">
            {t("Cada vídeo será un episodio del disco. Después eliges una plantilla de menú y produces el disco.")}
          </p>
          <div class="mt-4 flex justify-center gap-2">
            <button type="button" class="ui-btn ui-btn-primary" onclick={() => importAndAdd()}>
              <Icon name="import" size={15} />{t("Importar vídeos…")}
            </button>
            <button type="button" class="ui-btn" onclick={() => ui.open("slideshow")}>
              <Icon name="photo" size={15} />{t("Presentación de fotos")}
            </button>
            <button type="button" class="ui-btn" onclick={() => ui.open("join")}>
              <Icon name="film" size={15} />{t("Unir vídeos…")}
            </button>
          </div>
          <p class="ui-hint mt-3">{t("También puedes arrastrar los archivos a esta ventana.")}</p>
        </div>
      {:else}
        <div class="max-w-sm text-center">
          <p class="text-[14px] font-medium text-slate-200">{t("Selecciona un episodio")}</p>
          <p class="ui-hint mt-1">{t("Elige un episodio en la secuencia de abajo para recortarlo y marcar capítulos.")}</p>
        </div>
      {/if}
    </div>
  {:else}
    <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_260px]">
      <div class="flex min-w-0 flex-col gap-2">
        <!-- A TV-shaped frame: the video never sets the height, so vertical or
             odd-sized sources show with bars, as the disc will. -->
        <div class="relative mx-auto flex aspect-video max-h-[62vh] min-h-[180px] w-full items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-black">
          {#if playbackSupport === null}
            <div class="p-6 text-center text-xs text-slate-500">
              {t("Comprobando soporte de reproducción…")}
            </div>
          {:else if !playbackSupport.supported}
            <div class="absolute inset-0 flex items-center justify-center">
              {#if thumbnail}
                <img src={thumbnail} alt="" class="max-h-full max-w-full opacity-25" />
              {/if}
              <div class="relative max-w-md rounded-xl border border-amber-500/30 bg-slate-950/90 p-4 text-center shadow-xl">
                <p class="text-xs font-semibold text-amber-200">{t("Vista previa de vídeo no disponible")}</p>
                <p class="mt-1 text-[12px] leading-5 text-slate-400">{playbackSupport.detail}</p>
                {#if playbackSupport.installHint}
                  <code class="mt-2 block break-words rounded-md bg-black/40 px-2 py-1 text-left text-[12px] leading-5 text-cyan-200">
                    {playbackSupport.installHint}
                  </code>
                {/if}
                <p class="mt-2 text-[12px] leading-5 text-slate-500">
                  {t("Puedes recortar, añadir capítulos y capturar fotogramas igualmente.")}
                </p>
              </div>
            </div>
          {:else if videoSrc}
            <video
              bind:this={videoEl}
              src={videoSrc}
              preload="metadata"
              playsinline
              controls={videoFullscreen}
              class="absolute inset-0 h-full w-full object-contain"
              onloadedmetadata={onLoadedMetadata}
              onerror={onVideoError}
              ontimeupdate={onTimeUpdate}
              onplay={() => (playing = true)}
              onpause={() => (playing = false)}
              onended={() => (playing = false)}
            >
              <track kind="captions" />
            </video>
          {:else}
            <div class="p-6 text-center text-xs text-slate-500">
              {preparing ? t("Preparando vista previa…") : t("Sin vista previa disponible")}
            </div>
          {/if}
          {#if playbackSupport?.supported}
            <div class="absolute bottom-2 left-2 flex items-center gap-1.5 rounded-lg bg-slate-950/85 px-2 py-1 text-[11px] text-slate-300">
              <button
                type="button"
                onclick={togglePlay}
                disabled={!videoSrc}
                class="flex h-6 w-6 items-center justify-center rounded-full bg-cyan-500 text-slate-950 transition hover:bg-cyan-400 disabled:opacity-40"
              >
                {playing ? "❚❚" : "▶"}
              </button>
              <span class="font-mono">{formatMs(currentTimeMs)}</span>
              <span class="text-slate-600">/</span>
              <span class="font-mono text-slate-500">{formatMs(sourceDuration)}</span>
              <button
                type="button"
                onclick={captureThumbnail}
                disabled={!sourcePath}
                title={t("Usar este fotograma como miniatura del episodio en los menús")}
                class="ui-btn ui-btn-sm ml-1"
              >
                <Icon name="photo" size={13} />{t("Miniatura")}
              </button>
              <button
                type="button"
                onclick={toggleVideoFullscreen}
                disabled={!videoSrc}
                title={videoFullscreen ? t("Salir de pantalla completa") : t("Pantalla completa del vídeo")}
                class="ui-btn ui-btn-sm ml-1"
              >
                {videoFullscreen ? "⤡" : "⛶"}
              </button>
            </div>
          {/if}
        </div>

        <div class="rounded-lg border border-slate-800 p-3">
          <div class="mb-1.5 flex items-center justify-between text-[11px] text-slate-500">
            <span>{t("Entrada")} <span class="font-mono text-cyan-300">{formatMs(startMs)}</span></span>
            <span class="font-mono text-slate-200">{formatMs(currentTimeMs)}</span>
            <span>{t("Salida")} <span class="font-mono text-cyan-300">{formatMs(endMs)}</span></span>
          </div>
          <div
            bind:this={timelineEl}
            role="slider"
            tabindex="0"
            aria-label={t("Línea de tiempo de recorte")}
            aria-valuemin={0}
            aria-valuemax={sourceDuration}
            aria-valuenow={currentTimeMs}
            onpointerdown={startTimelineDrag}
            onpointermove={moveTimelineDrag}
            onpointerup={endTimelineDrag}
            onpointercancel={endTimelineDrag}
            onkeydown={onTimelineKeydown}
            class="relative h-14 cursor-crosshair touch-none select-none overflow-hidden rounded-md border border-slate-700 bg-black"
          >
            <!-- Filmstrip of the source video. -->
            <div class="pointer-events-none absolute inset-0 flex">
              {#each filmstrip as frame}
                <div class="h-full min-w-0 flex-1 overflow-hidden">
                  {#if frame}<img src={frame} alt="" class="h-full w-full object-cover opacity-80" draggable="false" />{/if}
                </div>
              {/each}
            </div>
            <!-- Outside the trim: dimmed. -->
            <div class="pointer-events-none absolute inset-y-0 left-0 bg-black/65" style={`width: ${percent(startMs)}%;`}></div>
            <div class="pointer-events-none absolute inset-y-0 right-0 bg-black/65" style={`width: ${Math.max(0, 100 - percent(endMs))}%;`}></div>
            <div
              class="pointer-events-none absolute inset-y-0 border-y-2 border-cyan-500"
              style={`left: ${percent(startMs)}%; width: ${Math.max(0, percent(endMs) - percent(startMs))}%;`}
            ></div>
            {#each episode?.chapters ?? [] as chapter}
              <span
                class="pointer-events-none absolute bottom-0 h-3 w-0.5 -translate-x-1/2 bg-amber-400"
                style={`left: ${percent(chapterSourceTime(chapter))}%;`}
                title={chapter.title}
              ></span>
            {/each}
            <span
              aria-hidden="true"
              class="absolute inset-y-0 z-10 flex w-2.5 -translate-x-1/2 cursor-ew-resize items-center justify-center rounded-l-sm bg-cyan-500"
              style={`left: ${percent(startMs)}%;`}
            ><span class="h-4 w-0.5 rounded bg-white/80"></span></span>
            <span
              aria-hidden="true"
              class="absolute inset-y-0 z-10 flex w-2.5 -translate-x-1/2 cursor-ew-resize items-center justify-center rounded-r-sm bg-cyan-500"
              style={`left: ${percent(endMs)}%;`}
            ><span class="h-4 w-0.5 rounded bg-white/80"></span></span>
            <div
              class="pointer-events-none absolute top-0 z-20 h-full w-0.5 -translate-x-1/2 bg-[#fff]"
              style={`left: ${percent(currentTimeMs)}%;`}
            >
              <span class="absolute -top-0.5 left-1/2 h-0 w-0 -translate-x-1/2 border-x-[5px] border-t-[7px] border-x-transparent border-t-[#fff]"></span>
            </div>
          </div>
          <div class="mt-1 flex justify-between px-0.5 font-mono text-[11px] text-slate-500">
            <span>00:00</span>
            <span>{formatMs(sourceDuration / 2)}</span>
            <span>{formatMs(sourceDuration)}</span>
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onclick={markStart}
              class="ui-btn ui-btn-sm"
            >
              {t("Marcar inicio")}
            </button>
            <button
              type="button"
              onclick={markEnd}
              class="ui-btn ui-btn-sm"
            >
              {t("Marcar fin")}
            </button>
            <span class="ml-auto rounded-md bg-slate-950/70 px-2 py-1 font-mono text-[11px] text-cyan-300">
              {t("Recorte")} {formatMs(effectiveMs)}
            </span>
          </div>
        </div>

        <div class="grid gap-2 sm:grid-cols-2">
          <label class="rounded-xl border border-slate-800 p-2.5">
            <span class="ui-section-title mb-1 block">{t("Escalado")}</span>
            <select
              class="ui-input w-full"
              value={episode.scaleMode}
              onchange={(event) => setScaleMode((event.currentTarget as HTMLSelectElement).value as VideoScaleMode)}
            >
              {#each scaleModes.filter((mode) => !mode.label.includes("(heredado)") || projectStore.expertMode || mode.id === episode.scaleMode) as mode}
                <option value={mode.id}>{t(mode.label)}</option>
              {/each}
            </select>
            <span class="mt-1 block text-[11px] text-slate-600">{scaleHint}</span>
          </label>
          <label class="rounded-xl border border-slate-800 p-2.5">
            <span class="ui-section-title mb-1 block">{t("Pista de audio")}</span>
            <select
              class="ui-input w-full"
              value={episode.audio[0]?.sourceStream ?? 0}
              disabled={episode.audio.length === 0}
              onchange={(event) => setAudio(Number((event.currentTarget as HTMLSelectElement).value))}
            >
              {#each episode.audio as audio}
                <option value={audio.sourceStream}>{audio.label} · {audio.language}</option>
              {:else}
                <!-- A video without sound: the disc gets a silent track. -->
                <option value={0}>{t("Sin sonido (el disco lleva silencio)")}</option>
              {/each}
            </select>
          </label>
        </div>

        <PictureFixes {episode} timeMs={currentTimeMs} />
      </div>

      <aside class="flex min-h-0 flex-col gap-2">
        <div class="flex min-h-0 flex-1 flex-col rounded-lg border border-slate-800 p-3">
          <div class="flex items-center justify-between">
            <h2 class="ui-section-title">
              {t("Capítulos")} · {episode.chapters.length}
            </h2>
            <button
              type="button"
              onclick={addChapter}
              class="ui-btn ui-btn-sm ui-btn-primary"
            >
              {t("+ Añadir")}
            </button>
          </div>
          <div class="mt-1.5 flex flex-wrap gap-1">
            <button
              type="button"
              onclick={() => autoChapters("5min")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[11px] text-slate-400 transition hover:border-slate-500 hover:text-slate-200"
            >
              {t("Cada 5 min")}
            </button>
            <button
              type="button"
              onclick={() => autoChapters("10min")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[11px] text-slate-400 transition hover:border-slate-500 hover:text-slate-200"
            >
              {t("Cada 10 min")}
            </button>
            <button
              type="button"
              onclick={() => autoChapters("quarters")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[11px] text-slate-400 transition hover:border-slate-500 hover:text-slate-200"
            >
              {t("4 partes")}
            </button>
            <button
              type="button"
              onclick={detectScenes}
              disabled={detectingScenes}
              title={t("Pone un capítulo en cada cambio de escena")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[11px] text-slate-400 transition hover:border-slate-500 hover:text-slate-200 disabled:opacity-60"
            >
              {detectingScenes ? t("Buscando escenas…") : t("Por escenas")}
            </button>
            <button
              type="button"
              onclick={importChapters}
              class="ui-btn ui-btn-sm"
            >
              {t("Importar del archivo")}
            </button>
          </div>
          <ul class="mt-2 min-h-0 flex-1 space-y-1 overflow-y-auto pr-0.5">
            {#each episode.chapters as chapter, index}
              <li class="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                <span class="w-4 text-center font-mono text-[11px] text-slate-600">{index + 1}</span>
                <button
                  type="button"
                  onclick={() => seek(chapterSourceTime(chapter))}
                  class="min-w-0 flex-1 text-left"
                >
                  <span class="block truncate text-[11px] text-slate-300">{chapter.title}</span>
                  <span class="block font-mono text-[11px] text-slate-500">
                    {formatMs(chapterSourceTime(chapter))}
                  </span>
                </button>
                {#if index > 0}
                  <button
                    type="button"
                    onclick={() => removeChapter(chapter)}
                    class="rounded px-1 text-[11px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
                    title={t("Eliminar capítulo")}
                  >
                    ✕
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      </aside>
    </div>
  {/if}
</section>
