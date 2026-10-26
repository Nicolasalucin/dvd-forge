<script lang="ts">
  import { t } from "../i18n/index.svelte";
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

  const episode = $derived(projectStore.selectedEpisode);
  const sourcePath = $derived(episode ? projectStore.episodeSourcePath(episode) : null);
  const sourceDuration = $derived(episode ? projectStore.episodeSourceDuration(episode) : 0);
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
    if (videoEl && previewPath) {
      videoEl.currentTime = startMs / 1000;
      currentTimeMs = startMs;
    }
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
      localError = projectStore.error;
    } else {
      localError = null;
      notice = t("Inicio en {time}.", { time: formatMs(ms) });
    }
  }

  function setEnd(ms: number) {
    if (!episode) return;
    if (!projectStore.setEpisodeTrimEnd(episode.id, ms)) {
      localError = projectStore.error;
    } else {
      localError = null;
      notice = t("Fin en {time}.", { time: formatMs(ms) });
    }
  }

  function resetTrim() {
    if (!episode) return;
    projectStore.resetEpisodeTrim(episode.id);
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
    projectStore.autoChapters(episode.id, mode);
    notice = t("Capítulos generados.");
    localError = null;
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

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex min-w-0 items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Edición")}</span>
      {#if episode}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-semibold text-cyan-200">
          E{episode.number}
        </span>
        <span class="truncate text-[11px] font-medium text-slate-200">{episode.title}</span>
      {/if}
    </div>
    {#if episode}
      <div class="flex flex-wrap items-center gap-2 text-[10px] text-slate-500">
        <span>{t("Fuente")} {formatDuration(sourceDuration)}</span>
        <span class="text-slate-700">→</span>
        <span class="font-semibold text-cyan-300">{t("Recorte")} {formatDuration(effectiveMs)}</span>
        <button
          type="button"
          onclick={resetTrim}
          class="rounded-md border border-slate-700 px-2 py-0.5 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
        >
          {t("Restablecer")}
        </button>
      </div>
    {/if}
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

  {#if !episode}
    <div class="flex flex-1 items-center justify-center p-6">
      <div class="max-w-sm rounded-2xl border border-dashed border-slate-800 p-8 text-center">
        <p class="text-sm font-medium text-slate-300">{t("Selecciona un episodio")}</p>
        <p class="mt-1 text-xs leading-5 text-slate-500">
          {t("Añade un vídeo desde la biblioteca de medios o elige un episodio en la secuencia.")}
        </p>
      </div>
    </div>
  {:else}
    <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_260px]">
      <div class="flex min-w-0 flex-col gap-2">
        <div class="relative flex min-h-[180px] flex-1 items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-black">
          {#if playbackSupport === null}
            <div class="p-6 text-center text-xs text-slate-500">
              {t("Comprobando soporte de reproducción…")}
            </div>
          {:else if !playbackSupport.supported}
            <div class="absolute inset-0 flex items-center justify-center">
              {#if thumbnail}
                <img src={thumbnail} alt="" class="max-h-full max-w-full opacity-25" />
              {/if}
              <div class="relative max-w-md rounded-xl border border-amber-500/30 bg-slate-950/90 p-4 text-center shadow-2xl backdrop-blur">
                <p class="text-xs font-semibold text-amber-200">{t("Vista previa de vídeo no disponible")}</p>
                <p class="mt-1 text-[10px] leading-4 text-slate-400">{playbackSupport.detail}</p>
                {#if playbackSupport.installHint}
                  <code class="mt-2 block break-words rounded-md bg-black/40 px-2 py-1 text-left text-[9px] leading-4 text-cyan-200">
                    {playbackSupport.installHint}
                  </code>
                {/if}
                <p class="mt-2 text-[10px] leading-4 text-slate-500">
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
              class="max-h-full max-w-full"
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
              {preparing ? "Preparando vista previa…" : "Sin vista previa disponible"}
            </div>
          {/if}
          {#if playbackSupport?.supported}
            <div class="absolute bottom-2 left-2 flex items-center gap-1.5 rounded-lg bg-slate-950/85 px-2 py-1 text-[10px] text-slate-300 backdrop-blur">
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
                title={t("Capturar fotograma para el menú")}
                class="ml-1 flex h-6 w-6 items-center justify-center rounded-md border border-slate-700 text-[11px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-40"
              >
                📸
              </button>
              <button
                type="button"
                onclick={toggleVideoFullscreen}
                disabled={!videoSrc}
                title={videoFullscreen ? "Salir de pantalla completa" : "Pantalla completa del vídeo"}
                class="ml-1 flex h-6 w-6 items-center justify-center rounded-md border border-slate-700 text-[11px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white disabled:opacity-40"
              >
                {videoFullscreen ? "⤡" : "⛶"}
              </button>
            </div>
          {/if}
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="mb-1.5 flex items-center justify-between text-[10px] text-slate-500">
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
            class="relative h-10 cursor-crosshair touch-none select-none overflow-hidden rounded-lg border border-slate-800 bg-slate-950"
          >
            <div class="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 rounded-full bg-slate-800"></div>
            <div
              class="absolute top-1/2 h-1.5 -translate-y-1/2 bg-cyan-500/45"
              style={`left: ${percent(startMs)}%; width: ${Math.max(0, percent(endMs) - percent(startMs))}%;`}
            ></div>
            <span
              aria-hidden="true"
              class="absolute top-0 z-10 flex h-full w-3 -translate-x-1/2 cursor-ew-resize items-start justify-center rounded-sm border-x-2 border-cyan-400 bg-cyan-400/10 pt-0.5 text-[9px] font-black leading-none text-cyan-200 shadow-[0_0_10px_rgba(34,211,238,0.7)]"
              style={`left: ${percent(startMs)}%;`}
            >
              [
            </span>
            <span
              aria-hidden="true"
              class="absolute top-0 z-10 flex h-full w-3 -translate-x-1/2 cursor-ew-resize items-start justify-center rounded-sm border-x-2 border-cyan-300 bg-cyan-300/10 pt-0.5 text-[9px] font-black leading-none text-cyan-100 shadow-[0_0_10px_rgba(34,211,238,0.7)]"
              style={`left: ${percent(endMs)}%;`}
            >
              ]
            </span>
            <div
              class="pointer-events-none absolute top-0 h-full w-0.5 -translate-x-1/2 bg-white"
              style={`left: ${percent(currentTimeMs)}%;`}
            >
              <span class="absolute -top-0.5 left-1/2 h-0 w-0 -translate-x-1/2 border-x-[4px] border-t-[6px] border-x-transparent border-t-white"></span>
            </div>
            <div class="pointer-events-none absolute inset-x-0 bottom-0 flex justify-between px-1 font-mono text-[8px] text-slate-600">
              <span>00:00</span>
              <span>{formatMs(sourceDuration / 2)}</span>
              <span>{formatMs(sourceDuration)}</span>
            </div>
          </div>
          <div class="mt-2 flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onclick={markStart}
              class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
            >
              {t("Marcar inicio")}
            </button>
            <button
              type="button"
              onclick={markEnd}
              class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
            >
              {t("Marcar fin")}
            </button>
            <span class="ml-auto rounded-md bg-slate-950/70 px-2 py-1 font-mono text-[10px] text-cyan-300">
              Recorte {formatMs(effectiveMs)}
            </span>
          </div>
        </div>

        <div class="grid gap-2 sm:grid-cols-2">
          <label class="rounded-xl border border-slate-800 bg-slate-900/50 p-2.5">
            <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Escalado")}</span>
            <select
              class="w-full rounded-lg border border-slate-700 bg-slate-950 px-2 py-1.5 text-[11px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={episode.scaleMode}
              onchange={(event) => setScaleMode((event.currentTarget as HTMLSelectElement).value as VideoScaleMode)}
            >
              {#each scaleModes as mode}
                <option value={mode.id}>{t(mode.label)}</option>
              {/each}
            </select>
            <span class="mt-1 block text-[9px] text-slate-600">{scaleHint}</span>
          </label>
          <label class="rounded-xl border border-slate-800 bg-slate-900/50 p-2.5">
            <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Pista de audio")}</span>
            <select
              class="w-full rounded-lg border border-slate-700 bg-slate-950 px-2 py-1.5 text-[11px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={episode.audio[0]?.sourceStream ?? 0}
              onchange={(event) => setAudio(Number((event.currentTarget as HTMLSelectElement).value))}
            >
              {#each episode.audio as audio}
                <option value={audio.sourceStream}>{audio.label} · {audio.language}</option>
              {/each}
            </select>
          </label>
        </div>
      </div>

      <aside class="flex min-h-0 flex-col gap-2">
        <div class="flex min-h-0 flex-1 flex-col rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex items-center justify-between">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              {t("Capítulos")} · {episode.chapters.length}
            </h2>
            <button
              type="button"
              onclick={addChapter}
              class="rounded-md bg-cyan-500 px-2 py-1 text-[10px] font-bold text-slate-950 transition hover:bg-cyan-400"
            >
              {t("+ Añadir")}
            </button>
          </div>
          <div class="mt-1.5 flex flex-wrap gap-1">
            <button
              type="button"
              onclick={() => autoChapters("5min")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[9px] text-slate-400 transition hover:border-cyan-500/60 hover:text-slate-200"
            >
              {t("Cada 5 min")}
            </button>
            <button
              type="button"
              onclick={() => autoChapters("10min")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[9px] text-slate-400 transition hover:border-cyan-500/60 hover:text-slate-200"
            >
              {t("Cada 10 min")}
            </button>
            <button
              type="button"
              onclick={() => autoChapters("quarters")}
              class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[9px] text-slate-400 transition hover:border-cyan-500/60 hover:text-slate-200"
            >
              {t("4 partes")}
            </button>
          </div>
          <ul class="mt-2 min-h-0 flex-1 space-y-1 overflow-y-auto pr-0.5">
            {#each episode.chapters as chapter, index}
              <li class="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-950/60 px-2 py-1.5">
                <span class="w-4 text-center font-mono text-[9px] text-slate-600">{index + 1}</span>
                <button
                  type="button"
                  onclick={() => seek(chapterSourceTime(chapter))}
                  class="min-w-0 flex-1 text-left"
                >
                  <span class="block truncate text-[10px] text-slate-300">{chapter.title}</span>
                  <span class="block font-mono text-[9px] text-slate-500">
                    {formatMs(chapterSourceTime(chapter))}
                  </span>
                </button>
                {#if index > 0}
                  <button
                    type="button"
                    onclick={() => removeChapter(chapter)}
                    class="rounded px-1 text-[10px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
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
