<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { projectStore } from "../stores/project.svelte";
  import { formatBytes } from "../utils/format";

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

  function statusClass(status: string): string {
    switch (status) {
      case "completed":
        return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
      case "cancelled":
        return "border-amber-500/30 bg-amber-500/10 text-amber-200";
      case "failed":
        return "border-rose-500/30 bg-rose-500/10 text-rose-200";
      default:
        return "border-slate-700 bg-slate-900 text-slate-300";
    }
  }

  function statusLabel(status: string): string {
    return status === "completed"
      ? "Completado"
      : status === "cancelled"
        ? "Cancelado"
        : status === "failed"
          ? "Fallido"
          : status;
  }

  async function openIso(path: string | null) {
    if (!path) return;
    localError = null;
    try {
      await unwrap(commands.openFolder(path));
    } catch (cause) {
      localError = messageOf(cause);
    }
  }
</script>

<div class="space-y-1">
  {#if localError}
    <p class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-2 py-1 text-[10px] text-rose-200">
      {localError}
    </p>
  {/if}
  {#if projectStore.jobHistory.length === 0}
    <p class="px-2 py-1 text-[10px] text-slate-600">
      {projectStore.jobHistoryBusy
        ? t("Cargando historial…")
        : t("Todavía no hay trabajos de producción registrados.")}
    </p>
  {:else}
    {#each projectStore.jobHistory as job (job.id)}
      <div class="rounded-xl border border-slate-800/80 bg-slate-900/60 px-2.5 py-2">
        <div class="flex items-center justify-between gap-2">
          <span class="min-w-0 truncate text-[11px] font-semibold text-slate-200">{job.projectName}</span>
          <span class={`shrink-0 rounded-full border px-1.5 py-0.5 text-[9px] ${statusClass(job.status)}`}>
            {statusLabel(job.status)}
          </span>
        </div>
        <div class="mt-1 flex flex-wrap items-center gap-1.5 text-[9px] text-slate-500">
          <span class="rounded border border-slate-700 px-1 py-0.5 text-cyan-200/80">{job.discKind}</span>
          <span>{formatDate(job.createdAt)}</span>
          {#if job.durationSeconds !== null}
            <span>· {job.durationSeconds.toFixed(1)} s</span>
          {/if}
          {#if job.isoSizeBytes > 0}
            <span>· {formatBytes(job.isoSizeBytes)}</span>
          {/if}
        </div>
        {#if job.logSummary}
          <p class="mt-1 line-clamp-2 text-[9px] leading-4 text-slate-500">{job.logSummary}</p>
        {/if}
        {#if job.isoPath}
          <button
            type="button"
            onclick={() => void openIso(job.isoPath)}
            class="mt-1.5 truncate rounded-md border border-slate-700 px-1.5 py-0.5 text-[9px] text-slate-300 transition hover:border-cyan-500/60 hover:text-white"
            title={job.isoPath}
          >
            {t("Abrir ISO")}
          </button>
        {/if}
      </div>
    {/each}
  {/if}
</div>
