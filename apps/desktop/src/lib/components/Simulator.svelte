<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { onDestroy, onMount } from "svelte";
  import {
    commands,
    type Episode_Serialize,
    type NavDirection,
    type NavigationGraph_Serialize,
    type PlaybackSupport_Serialize,
  } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import CanvasStage from "../editor/CanvasStage.svelte";
  import { loadPlaybackSupport, previewUrlFor } from "../playback";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";

  interface PlaybackItem {
    episodeId: string;
    offsetMs?: number;
  }

  let currentMenuId = $state<string | null>(null);
  let focusId = $state<string | null>(null);
  let graph = $state<NavigationGraph_Serialize | null>(null);
  let graphError = $state<string | null>(null);
  let activation = $state<"highlight" | "select">("highlight");
  let notice = $state<string | null>(null);
  let loading = $state(false);
  let isPlaying = $state(false);
  let playerSrc = $state<string | null>(null);
  let playerStartMs = $state(0);
  let playerEndMs = $state(0);
  let currentTimeMs = $state(0);
  let currentEpisode = $state<Episode_Serialize | null>(null);
  let queue = $state<PlaybackItem[]>([]);
  let queueIndex = $state(0);
  let pendingPlay = false;
  let advanceGuard = false;
  let graphToken = 0;
  let graphTimer: ReturnType<typeof setTimeout> | null = null;
  let activationTimer: ReturnType<typeof setTimeout> | null = null;
  let videoEl: HTMLVideoElement | undefined = $state();
  let tvStageDvd: HTMLDivElement | undefined = $state();
  let tvStageAvchd: HTMLDivElement | undefined = $state();
  let tvFullscreen = $state(false);
  let playbackSupport = $state<PlaybackSupport_Serialize | null>(null);
  const previewCache = new Map<string, string>();

  const project = $derived(projectStore.project);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const isBluRay = $derived(
    project?.disc.capacity === "bd25" || project?.disc.capacity === "bd50",
  );
  const discLabel = $derived(isAvchd ? (isBluRay ? "Blu-ray" : "AVCHD") : "DVD");
  const isContinuousAvchd = $derived(isAvchd && (project?.menus.length ?? 0) === 0);
  const episodes = $derived(
    [...(project?.episodes ?? [])].sort((a, b) => a.number - b.number),
  );
  const currentMenu = $derived(
    project?.menus.find((menu) => menu.id === currentMenuId) ??
      project?.menus.find((menu) => menu.id === project.rootMenu) ??
      project?.menus[0] ??
      null,
  );
  const backgroundUrl = $derived(
    currentMenu?.background.kind === "static"
      ? projectStore.previewForAsset(currentMenu.background.asset)
      : null,
  );
  const focusButton = $derived(
    currentMenu?.buttons.find((button) => button.id === focusId) ?? null,
  );
  const warnings = $derived(
    (graph?.warnings ?? []).filter((warning) => warning.severity !== "info"),
  );

  // Only serialise the fields that affect menu.navigation(). Reading the whole
  // project inside a synchronous effect made it rerun for every unrelated edit
  // and reset the focus on each pass.
  const graphSignature = $derived.by(() => {
    const menu = currentMenu;
    if (!menu) return "";
    return JSON.stringify({
      id: menu.id,
      canvas: menu.canvas,
      buttons: menu.buttons.map((button) => ({
        id: button.id,
        rect: button.rect,
        enabled: button.enabled,
        navOverride: button.navOverride,
        action: button.action,
      })),
    });
  });

  $effect(() => {
    const signature = graphSignature;
    if (graphTimer) {
      clearTimeout(graphTimer);
      graphTimer = null;
    }
    if (!signature) {
      graphToken += 1;
      graph = null;
      graphError = null;
      focusId = null;
      return;
    }
    graphTimer = setTimeout(() => {
      graphTimer = null;
      void loadGraph();
    }, 120);
  });

  onDestroy(() => {
    if (graphTimer) clearTimeout(graphTimer);
    if (activationTimer) clearTimeout(activationTimer);
  });

  onMount(() => {
    let active = true;
    void loadPlaybackSupport().then((support) => {
      if (active) playbackSupport = support;
    });
    const onFullscreenChange = () => {
      tvFullscreen = Boolean(document.fullscreenElement);
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () => {
      active = false;
      document.removeEventListener("fullscreenchange", onFullscreenChange);
    };
  });

  onMount(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (event.key === "Escape" && tvFullscreen) {
        // Deja que el navegador salga de fullscreen sin disparar goBack.
        return;
      }
      if (event.key === "Escape" || event.key === "Backspace") {
        event.preventDefault();
        goBack();
        return;
      }
      if (playerSrc) {
        if (event.key === " ") {
          event.preventDefault();
          void togglePause();
        } else if (event.key === "ArrowLeft") {
          event.preventDefault();
          seekPlayer(-5_000);
        } else if (event.key === "ArrowRight") {
          event.preventDefault();
          seekPlayer(5_000);
        }
        return;
      }
      if (!currentMenu) return;
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        activateFocus();
        return;
      }
      const direction = keyDirection(event.key);
      if (direction) {
        event.preventDefault();
        navigate(direction);
      }
    };
    window.addEventListener("keydown", handler);
    return () => {
      window.removeEventListener("keydown", handler);
      if (activationTimer) clearTimeout(activationTimer);
    };
  });

  async function loadGraph() {
    const menu = currentMenu;
    const token = ++graphToken;
    graphError = null;
    if (!menu) {
      graph = null;
      focusId = null;
      return;
    }
    const previousFocus = graph?.menuId === menu.id ? focusId : null;
    try {
      const result = await commands.menuNavigation(menu, null);
      if (token !== graphToken) return;
      graph = result;
      focusId =
        previousFocus && result.nodes.some((node) => node.button === previousFocus)
          ? previousFocus
          : (result.defaultFocus ?? result.nodes[0]?.button ?? null);
      activation = "highlight";
    } catch (cause) {
      if (token === graphToken) graphError = messageOf(cause);
    }
  }

  function refreshGraph() {
    if (graphTimer) {
      clearTimeout(graphTimer);
      graphTimer = null;
    }
    void loadGraph();
  }

  function keyDirection(key: string): NavDirection | null {
    if (key === "ArrowUp") return "up";
    if (key === "ArrowDown") return "down";
    if (key === "ArrowLeft") return "left";
    if (key === "ArrowRight") return "right";
    return null;
  }

  function navigate(direction: NavDirection) {
    if (!graph || !focusId) return;
    const edge = graph.edges.find(
      (candidate) => candidate.from === focusId && candidate.direction === direction,
    );
    if (!edge) {
      notice = t("Sin salida {direction} desde este botón.", { direction: t(directionLabel(direction)) });
      return;
    }
    focusId = edge.to;
    activation = "highlight";
    notice = null;
  }

  function directionLabel(direction: NavDirection): string {
    return t({ up: "arriba", down: "abajo", left: "izquierda", right: "derecha" }[direction]);
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

  function activateFocus() {
    if (focusId) activateButton(focusId);
  }

  function activateButton(id: string) {
    const button = currentMenu?.buttons.find((candidate) => candidate.id === id);
    if (!button || !button.enabled) return;
    activation = "select";
    if (activationTimer) clearTimeout(activationTimer);
    activationTimer = setTimeout(() => {
      activation = "highlight";
    }, 180);
    const action = button.action;
    switch (action.kind) {
      case "playEpisode":
        void startQueue([{ episodeId: action.episode }]);
        break;
      case "playAll":
        void startQueue(episodes.map((episode) => ({ episodeId: episode.id })));
        break;
      case "gotoMenu":
        currentMenuId = action.menu;
        focusId = null;
        break;
      case "playFirstPlay": {
        const first = project?.firstPlay;
        const episode = first
          ? project?.episodes.find((item) => item.video === first.asset)
          : episodes[0];
        if (episode) void startQueue([{ episodeId: episode.id }]);
        else notice = t("No hay contenido para el inicio.");
        break;
      }
      case "jumpTitleChapter": {
        const episode = project?.episodes.find((item) => item.id === action.episode);
        const chapter = episode?.chapters[action.chapter];
        const offset = episode && chapter ? projectStore.chapterTimeMs(episode, chapter) : 0;
        if (episode) {
          void startQueue([{ episodeId: episode.id, offsetMs: offset }]);
        } else {
          notice = t("El capítulo no existe.");
        }
        break;
      }
      case "setAudioTrack":
        notice = t("El reproductor usará la pista {track}.", { track: action.track + 1 });
        break;
      case "resume":
        notice = t("Resume no tiene estado previo en el simulador.");
        break;
      case "noop":
        notice = t("Este botón no tiene acción.");
        break;
    }
  }

  async function startQueue(items: PlaybackItem[]) {
    if (items.length === 0) {
      notice = t("No hay episodios para reproducir.");
      return;
    }
    const support = playbackSupport ?? (await loadPlaybackSupport());
    playbackSupport = support;
    if (!support.supported) {
      notice = `${support.detail}${support.installHint ? ` ${support.installHint}` : ""}`;
      return;
    }
    queue = items;
    queueIndex = 0;
    notice = null;
    await playCurrentItem();
  }

  async function playCurrentItem() {
    const item = queue[queueIndex];
    if (!item || !project) {
      stopPlayback();
      return;
    }
    const episode = project.episodes.find((candidate) => candidate.id === item.episodeId);
    if (!episode) {
      notice = t("El episodio ya no existe.");
      stopPlayback();
      return;
    }
    const path = projectStore.episodeSourcePath(episode);
    if (!path) {
      notice = t("No se puede acceder al archivo del episodio.");
      stopPlayback();
      return;
    }
    loading = true;
    try {
      const prepared =
        previewCache.get(path) ?? (await unwrap(commands.mediaPreparePreview(path)));
      previewCache.set(path, prepared);
      currentEpisode = episode;
      const start = (episode.trimStartMs ?? 0) + (item.offsetMs ?? 0);
      playerStartMs = start;
      playerEndMs = episode.trimEndMs ?? projectStore.episodeSourceDuration(episode);
      currentTimeMs = start;
      const src = await previewUrlFor(prepared);
      if (videoEl && playerSrc === src) {
        videoEl.currentTime = start / 1000;
        await videoEl.play();
        isPlaying = true;
        pendingPlay = false;
      } else {
        playerSrc = src;
        pendingPlay = true;
      }
    } catch (cause) {
      notice = messageOf(cause);
      stopPlayback();
    } finally {
      loading = false;
    }
  }

  function onPlayerLoaded() {
    if (!videoEl) return;
    videoEl.currentTime = playerStartMs / 1000;
    if (pendingPlay) {
      pendingPlay = false;
      void videoEl
        .play()
        .then(() => {
          isPlaying = true;
        })
        .catch((cause) => {
          notice = messageOf(cause);
        });
    }
  }

  function onPlayerTime() {
    if (!videoEl) return;
    currentTimeMs = Math.round(videoEl.currentTime * 1000);
    if (isPlaying && currentTimeMs >= playerEndMs) {
      void advancePlayback();
    }
  }

  async function advancePlayback() {
    if (advanceGuard) return;
    advanceGuard = true;
    videoEl?.pause();
    isPlaying = false;
    if (queueIndex + 1 < queue.length) {
      queueIndex += 1;
      await playCurrentItem();
    } else {
      stopPlayback();
    }
    advanceGuard = false;
  }

  function stopPlayback() {
    videoEl?.pause();
    isPlaying = false;
    playerSrc = null;
    currentEpisode = null;
    queue = [];
    queueIndex = 0;
    currentTimeMs = 0;
    pendingPlay = false;
    loading = false;
  }

  async function togglePause() {
    if (!videoEl) return;
    if (videoEl.paused) {
      if (currentTimeMs >= playerEndMs) videoEl.currentTime = playerStartMs / 1000;
      try {
        await videoEl.play();
        isPlaying = true;
      } catch (cause) {
        notice = messageOf(cause);
      }
    } else {
      videoEl.pause();
      isPlaying = false;
    }
  }

  function seekPlayer(deltaMs: number) {
    if (!videoEl) return;
    const target = Math.max(playerStartMs, Math.min(playerEndMs, currentTimeMs + deltaMs));
    videoEl.currentTime = target / 1000;
    currentTimeMs = target;
  }

  function goBack() {
    if (playerSrc) {
      stopPlayback();
      return;
    }
    if (currentMenu && project && currentMenu.id !== project.rootMenu) {
      currentMenuId = project.rootMenu;
      focusId = null;
      notice = null;
      return;
    }
    notice = t("Ya estás en el menú raíz.");
  }

  function goRoot() {
    if (!project) return;
    stopPlayback();
    currentMenuId = project.rootMenu || project.menus[0]?.id || null;
    focusId = null;
    notice = t("Menú raíz.");
  }

  function goTitle() {
    if (!project) return;
    stopPlayback();
    const titleMenu =
      project.menus.find((menu) => menu.role === "episodeSelect") ??
      project.menus.find((menu) => menu.placement.kind === "vtsm") ??
      null;
    if (!titleMenu) {
      notice = t("Este proyecto no tiene un menú de títulos.");
      return;
    }
    currentMenuId = titleMenu.id;
    focusId = null;
    notice = t("Menú de títulos.");
  }

  async function toggleTvFullscreen() {
    const element = isContinuousAvchd ? tvStageAvchd : tvStageDvd;
    if (!element) return;
    try {
      if (document.fullscreenElement) {
        await document.exitFullscreen();
      } else {
        await element.requestFullscreen();
      }
    } catch (cause) {
      notice = messageOf(cause);
    }
  }

  function episodeTitle(episode: Episode_Serialize): string {
    return `E${episode.number} · ${episode.title}`;
  }

  function progressPercent(): number {
    const duration = playerEndMs - playerStartMs;
    return duration > 0
      ? Math.max(0, Math.min(100, ((currentTimeMs - playerStartMs) / duration) * 100))
      : 0;
  }

  function playEpisodeDirect(episode: Episode_Serialize) {
    void startQueue([{ episodeId: episode.id }]);
  }
</script>

{#snippet playerPanel()}
  <div class="flex min-h-0 flex-1 flex-col gap-2">
    <div class="relative flex min-h-[240px] flex-1 items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-black">
      {#if playerSrc}
        <video
          bind:this={videoEl}
          src={playerSrc}
          preload="metadata"
          playsinline
          class="max-h-full max-w-full"
          onloadedmetadata={onPlayerLoaded}
          ontimeupdate={onPlayerTime}
          onplay={() => (isPlaying = true)}
          onpause={() => (isPlaying = false)}
          onended={() => void advancePlayback()}
        >
          <track kind="captions" />
        </video>
      {:else}
        <div class="p-6 text-center text-xs text-slate-500">
          {loading ? t("Preparando reproducción…") : t("Sin reproducción activa")}
        </div>
      {/if}
      <button
        type="button"
        onclick={goBack}
        class="absolute left-2 top-2 rounded-lg bg-slate-950/80 px-2.5 py-1 text-[10px] font-semibold text-slate-200 backdrop-blur transition hover:bg-slate-900"
      >
        {t("← Menú")}
      </button>
      {#if currentEpisode}
        <span class="absolute right-2 top-2 rounded-lg bg-slate-950/80 px-2 py-1 text-[10px] text-cyan-200 backdrop-blur">
          {episodeTitle(currentEpisode)}
        </span>
      {/if}
    </div>
    <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={togglePause}
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-cyan-500 text-slate-950 transition hover:bg-cyan-400"
        >
          {isPlaying ? "❚❚" : "▶"}
        </button>
        <div class="h-2 flex-1 overflow-hidden rounded-full bg-slate-800">
          <div class="h-full rounded-full bg-cyan-500" style={`width: ${progressPercent()}%`}></div>
        </div>
        <span class="shrink-0 font-mono text-[10px] text-slate-400">
          {formatDuration(Math.max(0, playerEndMs - playerStartMs))}
        </span>
      </div>
      <p class="mt-1.5 text-[10px] text-slate-500">
        {t("Reproduciendo solo el rango recortado")} · {formatDuration(currentTimeMs - playerStartMs)} {t("transcurridos")}
      </p>
    </div>
  </div>
{/snippet}

{#snippet tvRemoteOverlay()}
  <div class="pointer-events-none absolute inset-x-0 bottom-4 z-30 flex justify-center">
    <div class="pointer-events-auto flex items-center gap-2 rounded-2xl border border-white/15 bg-slate-950/55 p-2 shadow-2xl shadow-black/60 backdrop-blur-md">
      <button
        type="button"
        onclick={() => navigate("left")}
        class="flex h-8 w-8 items-center justify-center rounded-full border border-white/15 text-xs text-slate-200 transition hover:bg-white/10"
        title={t("Izquierda")}
      >
        ←
      </button>
      <div class="flex flex-col gap-1">
        <button
          type="button"
          onclick={() => navigate("up")}
          class="flex h-7 w-8 items-center justify-center rounded-full border border-white/15 text-xs text-slate-200 transition hover:bg-white/10"
          title={t("Arriba")}
        >
          ↑
        </button>
        <button
          type="button"
          onclick={() => navigate("down")}
          class="flex h-7 w-8 items-center justify-center rounded-full border border-white/15 text-xs text-slate-200 transition hover:bg-white/10"
          title={t("Abajo")}
        >
          ↓
        </button>
      </div>
      <button
        type="button"
        onclick={activateFocus}
        class="flex h-10 w-10 items-center justify-center rounded-full bg-cyan-500 text-[10px] font-black text-slate-950 transition hover:bg-cyan-400"
        title={t("OK")}
      >
        {t("OK")}
      </button>
      <button
        type="button"
        onclick={() => navigate("right")}
        class="flex h-8 w-8 items-center justify-center rounded-full border border-white/15 text-xs text-slate-200 transition hover:bg-white/10"
        title={t("Derecha")}
      >
        →
      </button>
      <div class="mx-1 h-8 w-px bg-white/10"></div>
      <button
        type="button"
        onclick={goRoot}
        class="rounded-lg px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:bg-white/10"
      >
        {t("Menú raíz")}
      </button>
      <button
        type="button"
        onclick={goTitle}
        class="rounded-lg px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:bg-white/10"
      >
        {t("Título")}
      </button>
      <button
        type="button"
        onclick={goBack}
        class="rounded-lg px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:bg-white/10"
      >
        {t("Volver")}
      </button>
      <button
        type="button"
        onclick={toggleTvFullscreen}
        class="rounded-lg border border-white/15 px-2 py-1 text-[10px] text-slate-300 transition hover:bg-white/10"
        title={t("Salir de pantalla completa")}
      >
        ⤡
      </button>
    </div>
  </div>
{/snippet}

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Simulador")}</span>
      {#if currentMenu}
        <span class="rounded-full border border-slate-800 px-2 py-0.5 text-[10px] text-slate-300">
          {currentMenu.name}
        </span>
      {/if}
      {#if isAvchd}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] text-cyan-200">
          {t(isContinuousAvchd ? "AVCHD continuo" : "Blu-ray/AVCHD HD")}
        </span>
      {/if}
    </div>
    <div class="flex items-center gap-2">
      {#if focusButton}
        <span class="hidden text-[10px] text-slate-500 sm:inline">
          {t("Foco:")} <span class="text-slate-300">{focusButton.name}</span>
        </span>
      {/if}
      <button
        type="button"
        onclick={toggleTvFullscreen}
        class="rounded-md border border-slate-800 px-2 py-1 text-[10px] text-slate-400 transition hover:border-cyan-500/60 hover:text-slate-200"
        title={t("Televisor a pantalla completa")}
      >
        {tvFullscreen ? "⤡ TV" : "⛶ TV"}
      </button>
      <button
        type="button"
        onclick={refreshGraph}
        class="rounded-md border border-slate-800 px-2 py-1 text-[10px] text-slate-400 transition hover:border-cyan-500/60 hover:text-slate-200"
      >
        {t("Refrescar grafo")}
      </button>
    </div>
  </div>

  {#if notice || graphError}
    <div
      class={`mx-3 mt-2 rounded-lg border px-3 py-1.5 text-[11px] ${
        graphError
          ? "border-rose-500/30 bg-rose-500/10 text-rose-200"
          : "border-slate-700 bg-slate-900/60 text-slate-300"
      }`}
    >
      {graphError ?? notice}
    </div>
  {/if}

  {#if playbackSupport && !playbackSupport.supported}
    <div class="mx-3 mt-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-1.5 text-[11px] text-amber-100">
      <p class="font-semibold">{t("Reproducción de vídeo no disponible")}</p>
      <p class="mt-0.5 text-[10px] leading-4 text-amber-100/80">{playbackSupport.detail}</p>
      {#if playbackSupport.installHint}
        <code class="mt-1 block break-words text-[9px] leading-4 text-cyan-200">{playbackSupport.installHint}</code>
      {/if}
      <p class="mt-1 text-[10px] text-amber-100/60">
        {t("Los menús, el foco y la navegación siguen funcionando.")}
      </p>
    </div>
  {/if}

  {#if isContinuousAvchd}
    <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_300px]">
      <div bind:this={tvStageAvchd} class="relative flex min-h-0 flex-1 flex-col gap-2 bg-[#05070d]">
        <div class="relative flex min-h-0 flex-1 flex-col items-center justify-center overflow-hidden rounded-[1.75rem] border-[10px] border-slate-800 bg-black p-2 shadow-[0_25px_60px_rgba(0,0,0,0.72)] ring-1 ring-black">
          {#if playerSrc}
            {@render playerPanel()}
          {:else}
            <div class="flex flex-1 items-center justify-center rounded-xl border border-slate-800 bg-slate-950/50 p-6 text-center">
              <div class="max-w-md">
                <p class="text-sm font-semibold text-cyan-100">{t("Reproducción continua AVCHD")}</p>
                <p class="mt-2 text-xs leading-5 text-slate-500">
                  {t("Los episodios se reproducen en orden. No hay menú interactivo; usa la lista para empezar y los capítulos se respetan dentro de cada episodio.")}
                </p>
                <button
                  type="button"
                  onclick={() => void startQueue(episodes.map((episode) => ({ episodeId: episode.id })))}
                  disabled={episodes.length === 0 || playbackSupport?.supported === false}
                  class="mt-4 rounded-xl bg-cyan-500 px-4 py-2 text-xs font-bold text-slate-950 transition hover:bg-cyan-400 disabled:opacity-40"
                >
                  {t("Reproducir todo")}
                </button>
              </div>
            </div>
          {/if}
        </div>
        {#if tvFullscreen}{@render tvRemoteOverlay()}{/if}
      </div>
      <aside class="flex min-h-0 flex-col rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
          {t("Episodios")} · {episodes.length}
        </h2>
        <ul class="mt-2 min-h-0 flex-1 space-y-1 overflow-y-auto">
          {#each episodes as episode}
            <li>
              <button
                type="button"
                onclick={() => playEpisodeDirect(episode)}
                class={`w-full rounded-lg border px-2 py-1.5 text-left transition ${
                  currentEpisode?.id === episode.id
                    ? "border-cyan-500/50 bg-cyan-500/10"
                    : "border-slate-800 bg-slate-950/50 hover:border-slate-700"
                }`}
              >
                <span class="block truncate text-[10px] font-medium text-slate-200">
                  {episodeTitle(episode)}
                </span>
                <span class="mt-0.5 block text-[9px] text-slate-500">
                  {formatDuration(projectStore.episodeEffectiveDuration(episode))} · {episode.chapters.length}
                  {t(episode.chapters.length === 1 ? "capítulo" : "capítulos")}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      </aside>
    </div>
  {:else if currentMenu}
    <div class="grid min-h-0 flex-1 gap-3 overflow-hidden p-3 xl:grid-cols-[minmax(0,1fr)_260px]">
      <div bind:this={tvStageDvd} class="relative flex min-h-0 flex-1 flex-col gap-2 bg-[#05070d]">
        <div class="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-[1.75rem] border-[10px] border-slate-800 bg-black shadow-[0_25px_60px_rgba(0,0,0,0.72)] ring-1 ring-black">
          {#if playerSrc}
            {@render playerPanel()}
          {:else}
            <CanvasStage
              menu={currentMenu}
              backgroundColor={projectStore.menuBackgroundColor}
              backgroundUrl={backgroundUrl}
              thumbnails={projectStore.thumbnailPreviewUrls}
              selectedButtonId={focusId}
              focusButtonId={focusId}
              previewState={activation}
              editable={false}
              showSafeAreas={false}
              showNavigation={projectStore.showNavigation}
              navigationGraph={graph}
              onActivate={(id) => activateButton(id)}
            />
          {/if}
        </div>
        {#if !tvFullscreen}
          <div class="flex flex-wrap items-center justify-between gap-2 px-1 text-[10px] text-slate-500">
            <span>{t("Flechas para navegar · Enter para activar · Retroceso para volver.")}</span>
            <span>
              {graph?.nodes.length ?? 0} {t("nodos")} · {graph?.edges.length ?? 0} {t("aristas")}
            </span>
          </div>
        {/if}
        {#if tvFullscreen}{@render tvRemoteOverlay()}{/if}
      </div>

      <aside class="flex min-h-0 flex-col gap-2 overflow-y-auto pr-0.5">
        <div class="rounded-[1.75rem] border border-slate-700/80 bg-gradient-to-b from-slate-800 via-slate-900 to-slate-950 p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.08),0_18px_40px_rgba(0,0,0,0.55)]">
          <div class="flex items-center justify-between">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-300">{t("Mando")}</h2>
            <span class="rounded-full border border-slate-700 px-2 py-0.5 text-[8px] uppercase tracking-wider text-slate-500">
              {discLabel}
            </span>
          </div>
          <div class="relative mx-auto mt-4 h-40 w-40">
            <div class="absolute inset-0 rounded-full border border-slate-600/80 bg-[radial-gradient(circle_at_30%_20%,#334155_0%,#0f172a_55%,#020617_100%)] shadow-[inset_0_2px_12px_rgba(255,255,255,0.08),0_12px_24px_rgba(0,0,0,0.5)]"></div>
            <button
              type="button"
              onclick={() => navigate("up")}
              class="absolute left-1/2 top-2 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full text-sm text-slate-200 transition hover:bg-white/10"
              aria-label={t("Arriba")}
            >
              ▲
            </button>
            <button
              type="button"
              onclick={() => navigate("left")}
              class="absolute left-2 top-1/2 flex h-9 w-9 -translate-y-1/2 items-center justify-center rounded-full text-sm text-slate-200 transition hover:bg-white/10"
              aria-label={t("Izquierda")}
            >
              ◀
            </button>
            <button
              type="button"
              onclick={() => navigate("right")}
              class="absolute right-2 top-1/2 flex h-9 w-9 -translate-y-1/2 items-center justify-center rounded-full text-sm text-slate-200 transition hover:bg-white/10"
              aria-label={t("Derecha")}
            >
              ▶
            </button>
            <button
              type="button"
              onclick={() => navigate("down")}
              class="absolute bottom-2 left-1/2 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full text-sm text-slate-200 transition hover:bg-white/10"
              aria-label={t("Abajo")}
            >
              ▼
            </button>
            <button
              type="button"
              onclick={activateFocus}
              class="absolute left-1/2 top-1/2 flex h-14 w-14 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-cyan-500 text-[11px] font-black text-slate-950 shadow-[0_0_0_6px_rgba(6,182,212,0.12),0_10px_20px_rgba(6,182,212,0.25)] transition hover:bg-cyan-400"
            >
              {t("OK")}
            </button>
          </div>
          <div class="mt-4 flex flex-wrap justify-center gap-1">
            <button
              type="button"
              onclick={goRoot}
              class="rounded-full border border-slate-600/70 bg-slate-950/60 px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
              title={t("Ir al menú raíz")}
            >
              {t("Menú raíz")}
            </button>
            <button
              type="button"
              onclick={goTitle}
              class="rounded-full border border-slate-600/70 bg-slate-950/60 px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
              title={t("Ir al menú de títulos")}
            >
              {t("Título")}
            </button>
            <button
              type="button"
              onclick={goBack}
              class="rounded-full border border-slate-600/70 bg-slate-950/60 px-2 py-1 text-[9px] font-semibold text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
              title={t("Volver")}
            >
              {t("Volver")}
            </button>
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Foco")}</h2>
          {#if focusButton}
            <p class="mt-1.5 truncate text-xs font-medium text-slate-200">{focusButton.name}</p>
            <p class="mt-0.5 text-[10px] text-slate-500">{actionLabel(focusButton.action.kind)}</p>
          {:else}
            <p class="mt-1.5 text-[10px] text-slate-500">{t("Sin botón enfocado")}</p>
          {/if}
          {#if graph}
            <p class="mt-2 text-[10px] text-slate-500">
              {graph.nodes.length} {t("botones")} · {graph.edges.length} {t("direcciones")}
            </p>
          {/if}
        </div>

        {#if graph?.routeHash}
          <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold text-slate-500">{t("Huella de rutas D-Pad")}</p>
            <p class="mt-1 truncate font-mono text-[10px] text-cyan-300">{graph.routeHash.slice(0, 24)}…</p>
          </div>
        {/if}

        {#if warnings.length > 0}
          <div class="rounded-xl border border-amber-500/25 bg-amber-500/5 p-2.5">
            <p class="text-[10px] font-semibold text-amber-200">{t("Avisos de navegación")}</p>
            <ul class="mt-1 space-y-1">
              {#each warnings as warning}
                <li class="text-[9px] leading-4 text-amber-100/70">{warning.message}</li>
              {/each}
            </ul>
          </div>
        {/if}
      </aside>
    </div>
  {:else}
    <div class="flex flex-1 items-center justify-center p-6 text-center">
      <div class="max-w-sm rounded-2xl border border-dashed border-slate-800 p-8">
        <p class="text-sm font-medium text-slate-300">{t("Sin menú para simular")}</p>
        <p class="mt-1 text-xs text-slate-500">
          {t("Crea un proyecto DVD-Video o Blu-ray/AVCHD para probar la navegación interactiva.")}
        </p>
      </div>
    </div>
  {/if}
</section>
