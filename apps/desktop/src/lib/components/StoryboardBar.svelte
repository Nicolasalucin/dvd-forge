<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { Episode_Serialize, Menu_Serialize } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";

  const project = $derived(projectStore.project);
  const episodes = $derived(projectStore.episodes);
  const menus = $derived(project?.menus ?? []);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const totalDuration = $derived(
    episodes.reduce((total, episode) => total + projectStore.episodeEffectiveDuration(episode), 0),
  );

  function selectEpisode(episode: Episode_Serialize) {
    projectStore.selectEpisode(episode.id);
  }

  function move(episode: Episode_Serialize, delta: number, event: MouseEvent) {
    event.stopPropagation();
    projectStore.moveEpisode(episode.id, delta);
  }

  function remove(episode: Episode_Serialize, event: MouseEvent) {
    event.stopPropagation();
    if (!window.confirm(t("¿Eliminar el episodio «{episode}» del proyecto?", { episode: episode.title }))) return;
    projectStore.removeEpisode(episode.id);
  }

  function addEpisode() {
    projectStore.mediaTab = "media";
    projectStore.leftCollapsed = false;
    projectStore.studioMode = "trim";
  }

  function menuRole(menu: Menu_Serialize): string {
    const role =
      menu.role === "root"
        ? t("Raíz")
        : menu.role === "chapterSelect"
          ? t("Capítulos")
          : menu.role === "episodeSelect"
            ? t("Episodios")
            : menu.role === "submenu"
              ? t("Submenú")
              : t("Menú");
    return menu.placement.kind === "vtsm"
      ? `VTSM ${menu.placement.titleset} · ${role}`
      : `VMGM · ${role}`;
  }

  function priorityClass(priority: Episode_Serialize["encodingPriority"]): string {
    if (priority === "high") return "bg-emerald-400";
    if (priority === "low") return "bg-amber-400";
    return "bg-cyan-400";
  }

  function openFirstPlay() {
    projectStore.studioMode = "simulator";
  }
</script>

<footer class="flex h-[148px] shrink-0 border-t border-slate-800/80 bg-slate-950/85">
  <div class="flex w-[218px] shrink-0 flex-col justify-between border-r border-slate-800/70 px-3 py-2">
    <div>
      <p class="text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-500">
        {t("Secuencia")}
      </p>
      <p class="mt-0.5 text-[11px] font-medium text-slate-300">
        {episodes.length} {t(episodes.length === 1 ? "episodio" : "episodios")} · {formatDuration(totalDuration)}
      </p>
      <div class="mt-1.5 flex flex-wrap gap-1">
        {#if project?.firstPlay}
          <button
            type="button"
            onclick={openFirstPlay}
            class="max-w-full rounded-md border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-left text-[9px] text-amber-200 transition hover:border-amber-400/60"
            title={t("Inicio / Intro")}
          >
            <span class="block truncate font-semibold">{t("Inicio")}</span>
            <span class="block text-[8px] text-amber-200/60">{t("Intro")}</span>
          </button>
        {/if}
        {#each menus as menu}
          <button
            type="button"
            onclick={() => projectStore.selectMenu(menu.id)}
            class={`max-w-full rounded-md border px-1.5 py-0.5 text-left text-[9px] transition ${
              projectStore.selectedMenuId === menu.id
                ? "border-cyan-500/50 bg-cyan-500/10 text-cyan-200"
                : "border-slate-800 text-slate-500 hover:border-slate-700 hover:text-slate-300"
            }`}
            title={menu.name}
          >
            <span class="block max-w-[190px] truncate font-semibold">{menu.name}</span>
            <span class="block max-w-[190px] truncate text-[8px] opacity-60">{menuRole(menu)}</span>
          </button>
        {/each}
        {#if isAvchd}
          <span class="rounded-md border border-cyan-500/25 bg-cyan-500/5 px-1.5 py-0.5 text-[9px] text-cyan-200/70">
            {t(menus.length > 0 ? "Blu-ray/AVCHD HD" : "AVCHD continuo")}
          </span>
        {/if}
      </div>
    </div>
    <div class="flex items-center justify-between gap-2 text-[10px] text-slate-500">
      <span>
        {#if projectStore.plan}
          <span class={projectStore.plan.feasible ? "text-emerald-300" : "text-rose-300"}>
            {projectStore.planPercent.toFixed(1)}% {t("del disco")}
          </span>
        {:else}
          {t("plan sin calcular")}
        {/if}
      </span>
      <button
        type="button"
        onclick={addEpisode}
        class="rounded-md border border-slate-700 px-1.5 py-0.5 text-[10px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
      >
        {t("+ Añadir")}
      </button>
    </div>
  </div>

  <div class="flex min-w-0 flex-1 items-stretch gap-2 overflow-x-auto px-3 py-2">
    {#if episodes.length === 0}
      <div class="flex min-w-[260px] flex-1 items-center justify-center rounded-xl border border-dashed border-slate-800 px-4">
        <p class="text-center text-[11px] leading-5 text-slate-500">
          {t("La secuencia está vacía. Importa un vídeo y añádelo como episodio.")}
        </p>
      </div>
    {:else}
      {#each episodes as episode, index (episode.id)}
        {@const selected = projectStore.selectedEpisode?.id === episode.id}
        {@const preview = projectStore.episodePreview(episode)}
        {@const duration = projectStore.episodeEffectiveDuration(episode)}
        <div
          role="button"
          tabindex="0"
          onclick={() => selectEpisode(episode)}
          onkeydown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              selectEpisode(episode);
            }
          }}
          class={`group relative flex w-[184px] shrink-0 cursor-pointer flex-col overflow-hidden rounded-xl border text-left transition ${
            selected
              ? "border-cyan-500/60 bg-cyan-500/10 shadow-lg shadow-cyan-500/5"
              : "border-slate-800 bg-slate-900/50 hover:border-slate-700"
          }`}
        >
          <div class="relative h-[64px] w-full overflow-hidden bg-slate-950">
            {#if preview}
              <img src={preview} alt="" class="h-full w-full object-cover" />
            {:else}
              <div class="flex h-full items-center justify-center text-lg text-slate-700">▷</div>
            {/if}
            <span class="absolute left-1 top-1 rounded bg-slate-950/85 px-1.5 py-0.5 font-mono text-[9px] text-cyan-200">
              E{episode.number}
            </span>
            {#if episode.chapters.length > 0}
              <span class="absolute bottom-1 right-1 rounded bg-slate-950/85 px-1.5 py-0.5 text-[9px] text-slate-300">
                {episode.chapters.length} cap.
              </span>
            {/if}
            <span class="absolute bottom-1 left-1 flex items-center gap-1 rounded bg-slate-950/85 px-1 py-0.5 text-[8px] text-slate-300">
              <span class={`h-1.5 w-1.5 rounded-full ${priorityClass(episode.encodingPriority)}`}></span>
              <span>A{episode.audio.length}</span>
              <span>S{episode.subtitles.length}</span>
            </span>
          </div>
          <div class="flex min-h-0 flex-1 flex-col justify-between p-1.5">
            <p class="truncate text-[10px] font-semibold text-slate-200" title={episode.title}>
              {episode.title}
            </p>
            <div class="mt-0.5 flex items-center justify-between gap-1">
              <span class="font-mono text-[9px] text-slate-500">{formatDuration(duration)}</span>
              <div class="flex items-center gap-0.5 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100">
                <button
                  type="button"
                  onclick={(event) => move(episode, -1, event)}
                  disabled={index === 0}
                  class="rounded px-1 text-[10px] text-slate-500 transition hover:bg-slate-800 hover:text-slate-200 disabled:opacity-25"
                  title={t("Mover a la izquierda")}
                >
                  ‹
                </button>
                <button
                  type="button"
                  onclick={(event) => move(episode, 1, event)}
                  disabled={index === episodes.length - 1}
                  class="rounded px-1 text-[10px] text-slate-500 transition hover:bg-slate-800 hover:text-slate-200 disabled:opacity-25"
                  title={t("Mover a la derecha")}
                >
                  ›
                </button>
                <button
                  type="button"
                  onclick={(event) => remove(episode, event)}
                  class="rounded px-1 text-[10px] text-slate-500 transition hover:bg-rose-500/10 hover:text-rose-300"
                  title={t("Eliminar episodio")}
                >
                  ✕
                </button>
              </div>
            </div>
          </div>
        </div>
      {/each}
      <button
        type="button"
        onclick={addEpisode}
        class="flex w-[74px] shrink-0 flex-col items-center justify-center gap-1 rounded-xl border border-dashed border-slate-800 text-slate-600 transition hover:border-cyan-500/40 hover:text-cyan-300"
        title={t("Añadir episodio desde la biblioteca")}
      >
        <span class="text-lg leading-none">+</span>
        <span class="text-[9px]">{t("Añadir")}</span>
      </button>
    {/if}
  </div>
</footer>
