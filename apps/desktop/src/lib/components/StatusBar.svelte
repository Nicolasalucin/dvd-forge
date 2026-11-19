<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import { ui } from "../ui/state.svelte";
  import { openUpdate } from "../ui/updates";
  import { formatBytes, formatDuration } from "../utils/format";

  const project = $derived(projectStore.project);
  const plan = $derived(projectStore.plan);
  const capacity = $derived(
    plan?.capacityBytes ??
      ({ bd25: 25_000_000_000, bd50: 50_000_000_000, dvd9: 8_540_000_000, dvd5: 4_700_000_000 } as const)[
        project?.disc.capacity ?? "dvd5"
      ],
  );
  const fill = $derived(Math.min(100, Math.max(0, projectStore.planPercent)));
  const tooFull = $derived(Boolean(plan && !plan.feasible));
  const missing = $derived(projectStore.toolsReport?.missingRequired ?? []);
  const discLabel = $derived(
    project
      ? project.disc.capacity === "bd25" || project.disc.capacity === "bd50"
        ? "Blu-ray"
        : project.disc.mode === "avchd"
          ? "AVCHD"
          : "DVD-Video"
      : "",
  );
</script>

<footer class="flex h-7 shrink-0 items-center gap-4 border-t border-slate-800 bg-slate-900 px-3 text-[12px] text-slate-400">
  {#if project}
    <span class="shrink-0 text-slate-300">{discLabel} · {project.disc.capacity.toUpperCase().replace("DVD", "DVD-").replace("BD", "BD-")}</span>
    <div class="flex min-w-0 flex-1 items-center gap-2" title={t("Espacio usado en el disco")}>
      <div class="relative h-1.5 w-40 shrink-0 overflow-hidden rounded-full bg-slate-800">
        <div class={`h-full rounded-full ${tooFull ? "bg-rose-500" : "bg-cyan-500"}`} style={`width: ${fill}%`}></div>
      </div>
      <span class="truncate">
        {projectStore.planBusy
          ? t("Calculando…")
          : plan
            ? `${formatBytes(projectStore.planUsedBytes)} ${t("de")} ${formatBytes(capacity)} · ${projectStore.qualityLabel}`
            : project.episodes.length > 0
              ? t("El plan se actualiza automáticamente")
              : t("Añade vídeos para calcular el espacio")}
      </span>
      {#if tooFull}<span class="shrink-0 text-rose-300">{t("No cabe: reduce la calidad o usa varios discos")}</span>{/if}
    </div>
    <span class="shrink-0">
      {t(project.episodes.length === 1 ? "{count} vídeo" : "{count} vídeos", { count: project.episodes.length })} · {t(project.menus.length === 1 ? "{count} menú" : "{count} menús", { count: project.menus.length })}{plan ? ` · ${formatDuration(plan.totalDurationMs)}` : ""}
    </span>
  {:else}
    <span class="flex-1">{t("Sin proyecto abierto")}</span>
  {/if}
  {#if ui.update}
    <button
      type="button"
      class="flex shrink-0 items-center gap-1 text-cyan-300 hover:underline"
      onclick={() => void openUpdate()}
      title={t("Abre la página de la versión nueva en el navegador")}
    >
      <Icon name="import" size={14} />
      {t("Kinodisc {version} disponible", { version: ui.update.version })}
    </button>
  {/if}
  <button
    type="button"
    class={`flex shrink-0 items-center gap-1 ${missing.length > 0 ? "text-amber-300" : "text-slate-400"} hover:text-slate-100`}
    onclick={() => (ui.doctor = true)}
    title={t("Herramientas de disco")}
  >
    <Icon name={missing.length > 0 ? "warning" : "check"} size={14} />
    {missing.length > 0 ? t("Faltan herramientas") : t("Herramientas listas")}
  </button>
</footer>
