<script lang="ts">
  import { i18n, t } from "../i18n/index.svelte";
  import { localizeDiagnostic } from "../i18n/diagnostics";
  import type { EncodingPriority } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { formatBytes, formatDuration } from "../utils/format";

  const project = $derived(projectStore.project);
  const plan = $derived(projectStore.plan);
  const episodes = $derived(projectStore.episodes);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const dvdVideo = $derived(project?.disc.video.kind === "dvd" ? project.disc.video : null);
  const totalMs = $derived(plan?.totalDurationMs ?? 0);
  const capacityBytes = $derived(
    plan?.capacityBytes ??
      (project?.disc.capacity === "bd25"
        ? 25_000_000_000
        : project?.disc.capacity === "bd50"
          ? 50_000_000_000
          : project?.disc.capacity === "dvd9"
            ? 8_540_000_000
            : 4_700_000_000),
  );
  const discLabel = $derived(
    project?.disc.capacity === "bd25"
      ? "BD-25 · 25 GB"
      : project?.disc.capacity === "bd50"
        ? "BD-50 · 50 GB"
        : project?.disc.capacity === "dvd9"
          ? "DVD-9 · 8,5 GB"
          : "DVD-5 · 4,7 GB",
  );
  const layerBreakPercent = $derived(
    plan?.layerBreakSector !== undefined && plan?.layerBreakSector !== null
      ? Math.min(100, (plan.layerBreakSector * 2048 * 100) / Math.max(1, capacityBytes))
      : null,
  );

  const chartWidth = 820;
  const chartHeight = 250;
  const padLeft = 46;
  const padRight = 14;
  const padTop = 18;
  const padBottom = 28;
  const plotWidth = chartWidth - padLeft - padRight;
  const plotHeight = chartHeight - padTop - padBottom;

  const maxKbps = $derived(
    Math.max(
      1_000,
      plan?.videoKbps ?? 0,
      ...(plan?.perEpisode.map((entry) => entry.videoKbps) ?? []),
    ) * 1.18,
  );

  const segments = $derived.by(() => {
    const entries = plan?.perEpisode ?? [];
    let cursor = 0;
    return entries.map((entry) => {
      const start = cursor;
      cursor += entry.durationMs;
      return { ...entry, start, end: cursor };
    });
  });

  function xForTime(ms: number): number {
    return padLeft + (totalMs > 0 ? (ms / totalMs) * plotWidth : 0);
  }

  function yForKbps(kbps: number): number {
    return padTop + (1 - Math.min(1, kbps / maxKbps)) * plotHeight;
  }

  const curvePath = $derived.by(() => {
    if (segments.length === 0) return "";
    let path = `M ${xForTime(0)} ${yForKbps(segments[0].videoKbps)}`;
    for (const segment of segments) {
      path += ` L ${xForTime(segment.start)} ${yForKbps(segment.videoKbps)}`;
      path += ` L ${xForTime(segment.end)} ${yForKbps(segment.videoKbps)}`;
    }
    return path;
  });

  const areaPath = $derived(
    curvePath ? `${curvePath} L ${xForTime(totalMs)} ${padTop + plotHeight} L ${xForTime(0)} ${padTop + plotHeight} Z` : "",
  );

  const targetY = $derived(plan ? yForKbps(plan.videoKbps) : 0);
  const layerBreakX = $derived(
    layerBreakPercent !== null ? padLeft + (layerBreakPercent / 100) * plotWidth : null,
  );

  const timeTicks = $derived(
    totalMs > 0 ? [0, 0.25, 0.5, 0.75, 1].map((fraction) => Math.round(totalMs * fraction)) : [0],
  );
  const kbpsTicks = $derived(
    [0, 0.33, 0.66, 1].map((fraction) => Math.round(maxKbps * fraction)),
  );

  const priorities: Array<{ id: EncodingPriority; label: string; hint: string }> = [
    { id: "low", label: "Baja", hint: "Menos bits, más espacio" },
    { id: "medium", label: "Media", hint: "Equilibrio" },
    { id: "high", label: "Alta", hint: "Más bits, mejor calidad" },
  ];

  function setPriority(episodeId: string, priority: EncodingPriority) {
    projectStore.setEpisodePriority(episodeId, priority);
  }

  function episodeTitle(episodeId: string): string {
    return episodes.find((episode) => episode.id === episodeId)?.title ?? episodeId;
  }

  function priorityOf(episodeId: string): EncodingPriority {
    return episodes.find((episode) => episode.id === episodeId)?.encodingPriority ?? "medium";
  }

  function warningClass(severity: string): string {
    if (severity === "error") return "border-rose-500/30 bg-rose-500/10 text-rose-200";
    if (severity === "warning") return "border-amber-500/25 bg-amber-500/5 text-amber-100";
    return "border-slate-800 bg-slate-900/50 text-slate-400";
  }
</script>

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Calidad VBR")}</span>
      {#if plan}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-semibold text-cyan-200">
          {plan.videoKbps.toLocaleString(i18n.locale)} kbps {t("promedio")}
        </span>
        <span class={`rounded-full border px-2 py-0.5 text-[10px] ${plan.feasible ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-200" : "border-rose-500/30 bg-rose-500/10 text-rose-200"}`}>
          {t(plan.feasible ? "Viable" : "No viable")}
        </span>
      {/if}
    </div>
    <div class="flex items-center gap-2">
      {#if dvdVideo}
        <label class="flex items-center gap-1.5 rounded-md border border-slate-800 bg-slate-900/70 px-2 py-1 text-[10px] text-slate-400">
          <input
            type="checkbox"
            checked={dvdVideo.trellis}
            onchange={(event) => projectStore.setTrellis(event.currentTarget.checked)}
            class="accent-cyan-500"
          />
          {t("Trellis")}
        </label>
      {/if}
      <span class="rounded-md border border-slate-800 px-2 py-1 text-[10px] text-slate-500">
        {projectStore.planBusy ? t("Calculando plan…") : t("Plan automático")}
      </span>
    </div>
  </div>

  <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_310px]">
    <div class="flex min-w-0 flex-col gap-3">
      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
            {t("Curva de asignación")}{isAvchd ? "" : t(" · 2 pasadas")}
          </h2>
          <div class="flex items-center gap-3 text-[9px] text-slate-500">
            <span class="flex items-center gap-1"><span class="h-0.5 w-4 bg-cyan-400"></span> {t("objetivo")}</span>
            <span class="flex items-center gap-1"><span class="h-0.5 w-4 bg-amber-300"></span> {t("corte de capa")}</span>
          </div>
        </div>

        {#if plan && segments.length > 0}
          <svg class="mt-2 w-full" viewBox={`0 0 ${chartWidth} ${chartHeight}`} preserveAspectRatio="none">
            <defs>
              <linearGradient id="vbr-area" x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stop-color="#22d3ee" stop-opacity="0.35" />
                <stop offset="100%" stop-color="#22d3ee" stop-opacity="0.02" />
              </linearGradient>
            </defs>

            {#each kbpsTicks as tick}
              <line
                x1={padLeft}
                y1={yForKbps(tick)}
                x2={chartWidth - padRight}
                y2={yForKbps(tick)}
                stroke="#1e293b"
                stroke-width="1"
              />
              <text
                x={padLeft - 6}
                y={yForKbps(tick) + 3}
                text-anchor="end"
                fill="#64748b"
                font-size="8"
                font-family="monospace">{Math.round(tick / 1000)}M</text>
            {/each}

            <path d={areaPath} fill="url(#vbr-area)" />
            <path d={curvePath} fill="none" stroke="#22d3ee" stroke-width="2.5" stroke-linejoin="round" />
            <line
              x1={padLeft}
              y1={targetY}
              x2={chartWidth - padRight}
              y2={targetY}
              stroke="#67e8f9"
              stroke-width="1"
              stroke-dasharray="6 5"
              opacity="0.75"
            />

            {#if layerBreakX !== null}
              <line
                x1={layerBreakX}
                y1={padTop}
                x2={layerBreakX}
                y2={padTop + plotHeight}
                stroke="#fcd34d"
                stroke-width="1.5"
                stroke-dasharray="4 4"
              />
              <text x={layerBreakX + 4} y={padTop + 9} fill="#fde68a" font-size="8">{t("corte de capa")}</text>
            {/if}

            {#each timeTicks as tick}
              <line
                x1={xForTime(tick)}
                y1={padTop + plotHeight}
                x2={xForTime(tick)}
                y2={padTop + plotHeight + 4}
                stroke="#334155"
              />
              <text
                x={xForTime(tick)}
                y={chartHeight - 8}
                text-anchor="middle"
                fill="#64748b"
                font-size="8"
                font-family="monospace">{formatDuration(tick)}</text>
            {/each}
          </svg>
        {:else}
          <div class="flex h-[250px] flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-slate-800">
            <p class="text-xs text-slate-500">
              {projectStore.planBusy
                ? t("Calculando plan…")
                : episodes.length === 0
                  ? t("Añade episodios para generar la curva VBR.")
                  : t("El plan se actualizará automáticamente al cambiar el proyecto.")}
            </p>
          </div>
        {/if}
      </div>

      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
            {t("Prioridad por episodio")}
          </h2>
          <span class="text-[9px] text-slate-500">
            {t("El promedio global se mantiene; los bits se redistribuyen.")}
          </span>
        </div>
        <div class="mt-2 space-y-1.5">
          {#each episodes as episode (episode.id)}
            {@const planned = plan?.perEpisode.find((entry) => entry.episode === episode.id)}
            {@const priority = priorityOf(episode.id)}
            <div class="flex flex-wrap items-center gap-2 rounded-lg border border-slate-800 bg-slate-950/50 px-2 py-1.5">
              <span class="font-mono text-[9px] text-cyan-300">E{episode.number}</span>
              <span class="min-w-0 flex-1 truncate text-[10px] text-slate-300">{episode.title}</span>
              <span class="font-mono text-[9px] text-slate-500">
                {planned ? `${planned.videoKbps.toLocaleString(i18n.locale)} kbps` : t("sin plan")}
              </span>
              <span class="font-mono text-[9px] text-slate-600">
                {formatDuration(projectStore.episodeEffectiveDuration(episode))}
              </span>
              <div class="flex rounded-md border border-slate-800 bg-slate-900 p-0.5">
                {#each priorities as option}
                  <button
                    type="button"
                    onclick={() => void setPriority(episode.id, option.id)}
                    title={t(option.hint)}
                    class={`rounded px-1.5 py-0.5 text-[9px] font-semibold transition ${
                      priority === option.id
                        ? "bg-cyan-500/20 text-cyan-200"
                        : "text-slate-500 hover:text-slate-200"
                    }`}
                  >
                    {t(option.label)}
                  </button>
                {/each}
              </div>
            </div>
          {/each}
          {#if episodes.length === 0}
            <p class="text-[10px] text-slate-500">{t("Sin episodios.")}</p>
          {/if}
        </div>
      </div>

      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Diagnósticos")}</h2>
        <div class="mt-2 space-y-1.5">
          {#each plan?.warnings ?? [] as warning (warning.code + warning.message)}
            <div class={`rounded-lg border px-2 py-1.5 text-[10px] leading-4 ${warningClass(warning.severity)}`}>
              <span class="font-mono text-[9px] opacity-70">{warning.code}</span>
              <span class="ml-1">{localizeDiagnostic(warning)}</span>
            </div>
          {/each}
          {#if !plan || plan.warnings.length === 0}
            <p class="rounded-lg border border-dashed border-slate-800 px-2 py-3 text-center text-[10px] text-slate-500">
              {t("Sin advertencias del planificador.")}
            </p>
          {/if}
          {#if plan && !plan.feasible}
            <div class="rounded-lg border border-rose-500/30 bg-rose-500/10 px-2 py-1.5 text-[10px] text-rose-200">
              {t("Buffer underflow probable: reduce la prioridad de los episodios largos o cambia a Half-D1 / DVD-9.")}
            </div>
          {/if}
        </div>
      </div>
    </div>

    <aside class="flex min-h-0 flex-col gap-2">
      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Presupuesto")}</h2>
        <dl class="mt-2 space-y-1 text-[10px]">
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Disco")}</dt>
            <dd class="text-slate-300">{discLabel}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Capacidad")}</dt>
            <dd class="font-mono text-slate-300">{formatBytes(capacityBytes)}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Objetivo")}</dt>
            <dd class="font-mono text-cyan-300">{plan ? formatBytes(plan.targetBytes) : "—"}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Llenado")}</dt>
            <dd class={`font-mono ${plan && !plan.feasible ? "text-rose-300" : "text-emerald-300"}`}>
              {plan ? `${(plan.fillPpm / 10_000).toFixed(1)}%` : "—"}
            </dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Vídeo")}</dt>
            <dd class="font-mono text-slate-300">{plan ? formatBytes(plan.videoBudgetBytes) : "—"}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Audio")}</dt>
            <dd class="font-mono text-slate-300">{plan ? formatBytes(plan.episodeAudioBytes) : "—"}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Subtítulos")}</dt>
            <dd class="font-mono text-slate-300">{plan ? formatBytes(plan.subtitleBytes) : "—"}</dd>
          </div>
        </dl>
      </div>

      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Corte de capa")}</h2>
        {#if plan?.layerBreakSector !== null && plan?.layerBreakSector !== undefined}
          <dl class="mt-2 space-y-1 text-[10px]">
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Sector")}</dt>
              <dd class="font-mono text-amber-200">{plan.layerBreakSector}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Posición")}</dt>
              <dd class="font-mono text-slate-300">{layerBreakPercent?.toFixed(1)}%</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Modo")}</dt>
              <dd class="text-slate-300">{plan.layerBreakMode?.toUpperCase()}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Bytes")}</dt>
              <dd class="font-mono text-slate-300">{formatBytes(plan.layerBreakSector * 2048)}</dd>
            </div>
          </dl>
          <p class="mt-2 text-[9px] leading-4 text-slate-500">
            {t("Alineado a bloques ECC de 16 sectores. La autoría ajusta el corte de capa al PGC más cercano durante la grabación.")}
          </p>
        {:else}
          <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
            {t("Selecciona DVD-9 para calcular el punto de cambio de capa PTP/OTP.")}
          </p>
        {/if}
      </div>

      <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
        <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Codificación")}</h2>
        <dl class="mt-2 space-y-1 text-[10px]">
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Perfil")}</dt>
            <dd class="text-slate-300">{isAvchd ? "H.264/AVC" : (plan?.profile ?? "—")}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("2 pasadas")}</dt>
            <dd class="text-emerald-300">{isAvchd ? t("No") : dvdVideo?.twoPass ? t("Sí") : t("No")}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Trellis")}</dt>
            <dd class="text-slate-300">{dvdVideo?.trellis ? t("Activo") : t("Desactivado")}</dd>
          </div>
          <div class="flex justify-between gap-2">
            <dt class="text-slate-500">{t("Escalado")}</dt>
            <dd class="text-slate-300">{isAvchd ? "AVCHD/Blu-ray HD" : "DVD-Video"}</dd>
          </div>
        </dl>
      </div>
    </aside>
  </div>
</section>
