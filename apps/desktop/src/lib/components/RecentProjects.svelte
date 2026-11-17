<script lang="ts">
  import { i18n, t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";

  interface Props {
    compact?: boolean;
    onOpened?: () => void;
  }

  let { compact = false, onOpened }: Props = $props();
  let opening = $state<string | null>(null);

  function formatDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString(i18n.locale, {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  async function open(path: string) {
    opening = path;
    try {
      // The store shows the (localized) error in the header banner.
      await projectStore.openProject(path);
      onOpened?.();
    } catch {
      await projectStore.refreshRecentProjects();
    } finally {
      opening = null;
    }
  }
</script>

<div class="space-y-1">
  {#if projectStore.recentProjectsError}
    <p class="rounded-md border border-amber-500/30 bg-amber-500/10 px-2 py-1 text-[12px] text-amber-200">
      {t("No se pudieron cargar los proyectos recientes: {error}", { error: projectStore.recentProjectsError })}
    </p>
  {/if}
  {#if projectStore.recentProjects.length === 0}
    <p class="ui-hint px-2 py-1">
      {projectStore.recentProjectsBusy ? t("Cargando recientes…") : t("Todavía no hay proyectos recientes.")}
    </p>
  {:else}
    {#each projectStore.recentProjects as recent (recent.id)}
      <div class="group flex items-center gap-1 rounded-md border border-slate-800 bg-slate-950 pr-1 hover:border-slate-700">
        <button
          type="button"
          onclick={() => void open(recent.filePath)}
          disabled={opening !== null || recent.missing}
          class="min-w-0 flex-1 px-2 py-1.5 text-left disabled:cursor-default"
          title={recent.missing ? t("No se encuentra el archivo: {path}", { path: recent.filePath }) : recent.filePath}
        >
          <span class="block truncate text-[13px] font-semibold {recent.missing ? 'text-slate-500' : 'text-slate-100'}">
            {opening === recent.filePath ? t("Abriendo…") : recent.name}
          </span>
          <span class="mt-0.5 flex items-center gap-1.5 text-[11px] text-slate-500">
            {#if recent.missing}
              <span class="ui-chip text-amber-300">{t("No se encuentra")}</span>
            {:else}
              <span class="ui-chip">{recent.discKind}</span>
            {/if}
            <span>{formatDate(recent.updatedAt)}</span>
          </span>
          {#if !compact}
            <span class="mt-0.5 block truncate font-mono text-[11px] text-slate-500">{recent.filePath}</span>
          {/if}
        </button>
        <button
          type="button"
          onclick={() => void projectStore.forgetRecentProject(recent.filePath)}
          class="ui-btn-icon {recent.missing ? '' : 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}"
          title={t("Quitar de recientes")}
          aria-label={t("Quitar de recientes")}
        >
          <Icon name="close" size={14} />
        </button>
      </div>
    {/each}
  {/if}
</div>
