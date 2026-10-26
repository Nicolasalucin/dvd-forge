<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { messageOf } from "../api/client";
  import { projectStore } from "../stores/project.svelte";

  interface Props {
    compact?: boolean;
    onOpened?: () => void;
  }

  let { compact = false, onOpened }: Props = $props();
  let opening = $state<string | null>(null);
  let localError = $state<string | null>(null);

  function formatDate(value: string): string {
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return value;
    return date.toLocaleString("es-ES", {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  async function open(path: string) {
    opening = path;
    localError = null;
    try {
      await projectStore.openProject(path);
      onOpened?.();
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      opening = null;
    }
  }
</script>

<div class={compact ? "space-y-1" : "space-y-2"}>
  {#if localError}
    <p class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-2 py-1 text-[10px] text-rose-200">
      {localError}
    </p>
  {/if}
  {#if projectStore.recentProjects.length === 0}
    <p class="px-2 py-1 text-[10px] text-slate-600">
      {projectStore.recentProjectsBusy ? t("Cargando recientes…") : t("Todavía no hay proyectos recientes.")}
    </p>
  {:else}
    {#each projectStore.recentProjects as recent (recent.id)}
      <div class="group flex items-center gap-2 rounded-xl border border-slate-800/80 bg-slate-900/60 px-2 py-1.5">
        <button
          type="button"
          onclick={() => void open(recent.filePath)}
          disabled={opening !== null}
          class="min-w-0 flex-1 text-left disabled:opacity-50"
          title={recent.filePath}
        >
          <span class="block truncate text-[11px] font-semibold text-slate-200">
            {opening === recent.filePath ? "Abriendo…" : recent.name}
          </span>
          <span class="mt-0.5 flex items-center gap-1.5 text-[9px] text-slate-500">
            <span class="rounded border border-slate-700 px-1 py-0.5 text-cyan-200/80">{recent.discKind}</span>
            <span>{formatDate(recent.updatedAt)}</span>
          </span>
          {#if !compact}
            <span class="mt-0.5 block truncate font-mono text-[8px] text-slate-600">{recent.filePath}</span>
          {/if}
        </button>
        <button
          type="button"
          onclick={() => void projectStore.forgetRecentProject(recent.filePath)}
          class="rounded px-1 text-[10px] text-slate-600 opacity-0 transition hover:bg-rose-500/10 hover:text-rose-300 group-hover:opacity-100"
          title={t("Quitar de recientes")}
        >
          ✕
        </button>
      </div>
    {/each}
  {/if}
</div>
