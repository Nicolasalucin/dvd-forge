<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { Episode_Serialize, Menu_Serialize } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";
  import Icon from "../ui/Icon.svelte";

  let dragFrom = $state<number | null>(null);
  let dropAt = $state<number | null>(null);

  function menuRoleShort(menu: Menu_Serialize): string {
    return menu.role === "root"
      ? t("Raíz")
      : menu.role === "chapterSelect"
        ? t("Capítulos")
        : menu.role === "episodeSelect"
          ? t("Episodios")
          : t("Submenú");
  }

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

  function addMenu() {
    projectStore.addMenu();
  }

  function renameMenu(menu: Menu_Serialize) {
    const name = window.prompt(t("Nombre de la pantalla de menú"), menu.name);
    if (name !== null) projectStore.renameMenu(menu.id, name);
  }

  function removeMenu(menu: Menu_Serialize, event: MouseEvent) {
    event.stopPropagation();
    if (!window.confirm(t("¿Eliminar la pantalla de menú «{menu}»? Los botones que la abrían volverán al menú principal.", { menu: menu.name }))) return;
    projectStore.removeMenu(menu.id);
  }

  function openFirstPlay() {
    projectStore.studioMode = "simulator";
  }
</script>

<footer class="flex h-[132px] shrink-0 border-t border-slate-800 bg-slate-900">
  <div class="flex w-[232px] shrink-0 flex-col border-r border-slate-800">
    <div class="flex h-8 shrink-0 items-center justify-between px-3">
      <span class="ui-section-title">{t("Menús")}</span>
      {#if project}
        <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost" onclick={addMenu} title={t("Añadir otra pantalla de menú (submenú) enlazada desde la actual")}>
          <Icon name="add" size={13} />{t("Menú")}
        </button>
      {/if}
    </div>
    <ul class="min-h-0 flex-1 space-y-0.5 overflow-y-auto px-1.5 pb-1.5">
      {#if project?.firstPlay}
        <li>
          <button type="button" class="ui-row py-1" onclick={openFirstPlay} title={t("Inicio / Intro")}>
            <Icon name="play" size={14} class="text-amber-300" />
            <span class="truncate">{t("Inicio")}</span>
            <span class="ui-chip ml-auto">{t("Intro")}</span>
          </button>
        </li>
      {/if}
      {#each menus as menu (menu.id)}
        <li class="group/menu relative">
          <button
            type="button"
            class="ui-row py-1 pr-7"
            aria-current={projectStore.selectedMenuId === menu.id}
            onclick={() => projectStore.selectMenu(menu.id)}
            ondblclick={() => renameMenu(menu)}
            title={t("{menu} · doble clic para renombrar", { menu: menu.name })}
          >
            <Icon name="menu" size={14} class="text-slate-400" />
            <span class="truncate">{menu.name}</span>
            <span class="ml-auto shrink-0 text-[11px] text-slate-500">{menuRoleShort(menu)}</span>
          </button>
          {#if menu.id !== project?.rootMenu}
            <button
              type="button"
              onclick={(event) => removeMenu(menu, event)}
              class="absolute right-1 top-1/2 hidden h-6 w-6 -translate-y-1/2 items-center justify-center rounded text-slate-400 hover:bg-rose-500/15 hover:text-rose-300 group-hover/menu:flex"
              title={t("Eliminar pantalla de menú")}
              aria-label={t("Eliminar pantalla de menú")}
            ><Icon name="close" size={13} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  </div>

  <div class="flex min-w-0 flex-1 flex-col">
    <div class="flex h-8 shrink-0 items-center gap-3 px-3">
      <span class="ui-section-title">{t("Episodios")}</span>
      <span class="text-[12px] text-slate-500">
        {episodes.length} · {formatDuration(totalDuration)}
        {#if projectStore.plan}
          · <span class={projectStore.plan.feasible ? "text-slate-400" : "text-rose-300"}>{projectStore.planPercent.toFixed(1)}% {t("del disco")}</span>
        {/if}
      </span>
      {#if episodes.length > 1}<span class="ui-hint ml-auto hidden lg:inline">{t("Arrastra para cambiar el orden")}</span>{/if}
    </div>
    <div class="flex min-h-0 flex-1 items-stretch gap-2 overflow-x-auto px-3 pb-2" role="list">
      {#if episodes.length === 0}
        <button
          type="button"
          onclick={addEpisode}
          class="flex min-w-[280px] flex-1 items-center justify-center gap-2 rounded-md border border-dashed border-slate-700 text-[13px] text-slate-400 hover:border-slate-500 hover:text-slate-100"
        >
          <Icon name="add" size={16} />{t("La secuencia está vacía. Importa un vídeo y añádelo como episodio.")}
        </button>
      {:else}
        {#each episodes as episode, index (episode.id)}
          {@const selected = projectStore.selectedEpisode?.id === episode.id}
          {@const preview = projectStore.episodePreview(episode)}
          {@const duration = projectStore.episodeEffectiveDuration(episode)}
          <div
            role="listitem"
            draggable="true"
            ondragstart={(event) => {
              dragFrom = index;
              event.dataTransfer?.setData("text/plain", episode.id);
              if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
            }}
            ondragover={(event) => {
              if (dragFrom === null) return;
              event.preventDefault();
              dropAt = index;
            }}
            ondragleave={() => (dropAt = dropAt === index ? null : dropAt)}
            ondrop={(event) => {
              event.preventDefault();
              if (dragFrom !== null && dragFrom !== index) projectStore.moveEpisode(episodes[dragFrom].id, index - dragFrom);
              dragFrom = null;
              dropAt = null;
            }}
            ondragend={() => {
              dragFrom = null;
              dropAt = null;
            }}
            class={`group relative flex w-[150px] shrink-0 flex-col overflow-hidden rounded-md border ${
              selected ? "border-cyan-500" : "border-slate-700 hover:border-slate-500"
            } ${dropAt === index && dragFrom !== index ? "ring-2 ring-cyan-500/60" : ""} ${dragFrom === index ? "opacity-50" : ""}`}
          >
            <button type="button" class="flex min-h-0 flex-1 flex-col text-left" onclick={() => selectEpisode(episode)}>
              <div class="relative aspect-video w-full overflow-hidden bg-black">
                {#if preview}
                  <img src={preview} alt="" class="h-full w-full bg-black object-contain" draggable="false" />
                {:else}
                  <div class="flex h-full items-center justify-center text-slate-600"><Icon name="film" size={18} /></div>
                {/if}
                <span class="absolute left-1 top-1 rounded bg-black/70 px-1 font-mono text-[11px] text-[#fff]">{episode.number}</span>
                <span class="absolute bottom-1 right-1 rounded bg-black/70 px-1 font-mono text-[11px] text-[#fff]">{formatDuration(duration)}</span>
                {#if episode.chapters.length > 0}
                  <span class="absolute bottom-1 left-1 rounded bg-black/70 px-1 text-[11px] text-[#fff]">{t("{count} cap.", { count: episode.chapters.length })}</span>
                {/if}
              </div>
              <p class="truncate px-1.5 py-1 text-[12px] text-slate-100" title={episode.title}>{episode.title}</p>
            </button>
            <button
              type="button"
              onclick={(event) => remove(episode, event)}
              class="absolute right-1 top-1 hidden h-6 w-6 items-center justify-center rounded bg-black/70 text-[#fff] hover:bg-rose-600 group-hover:flex"
              title={t("Eliminar episodio")}
              aria-label={t("Eliminar episodio")}
            ><Icon name="close" size={13} /></button>
          </div>
        {/each}
        <button
          type="button"
          onclick={addEpisode}
          class="flex w-[72px] shrink-0 flex-col items-center justify-center gap-1 rounded-md border border-dashed border-slate-700 text-[12px] text-slate-400 hover:border-slate-500 hover:text-slate-100"
          title={t("Añadir episodio desde la biblioteca")}
        >
          <Icon name="add" size={16} />{t("Añadir")}
        </button>
      {/if}
    </div>
  </div>
</footer>
