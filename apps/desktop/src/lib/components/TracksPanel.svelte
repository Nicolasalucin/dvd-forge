<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import type { AudioFormat, VideoScaleMode } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import LanguageSelect from "./LanguageSelect.svelte";
  import { normalizeLanguageCode } from "../i18n/languages";
  import { basename, formatDuration } from "../utils/format";

  const episode = $derived(projectStore.selectedEpisode);
  const asset = $derived(episode ? projectStore.assetFor(episode.video) : null);
  const duration = $derived(episode ? projectStore.episodeEffectiveDuration(episode) : 0);
  const availableAudio = $derived(
    asset?.probe.audio.filter(
      (stream) => !episode?.audio.some((track) => track.sourceStream === stream.index),
    ) ?? [],
  );
  const defaultAudio = $derived(episode?.audio.find((track) => track.default) ?? episode?.audio[0]);
  const defaultSubtitle = $derived(episode?.subtitles.find((track) => track.isDefault) ?? null);
  let audioToAdd = $state("");

  const scaleModes: Array<{ id: VideoScaleMode; label: string; hint: string }> = [
    { id: "cover", label: "Rellenar", hint: "Llena el marco recortando" },
    { id: "contain", label: "Ajustar", hint: "Barras negras, sin recortar" },
    { id: "stretch", label: "Estirar", hint: "Estira la imagen" },
    { id: "letterbox", label: "Buzón (heredado)", hint: "Compatibilidad" },
    { id: "cropFill", label: "Recorte y relleno (heredado)", hint: "Compatibilidad" },
  ];

  function scaleLabel(mode: VideoScaleMode): string {
    return t(scaleModes.find((item) => item.id === mode)?.label ?? mode);
  }

  function addAudio() {
    if (!episode) return;
    const stream = audioToAdd
      ? Number(audioToAdd)
      : availableAudio[0]?.index;
    if (stream === undefined) return;
    if (projectStore.addEpisodeAudioTrack(episode.id, stream)) audioToAdd = "";
  }

  function setFormat(index: number, format: AudioFormat) {
    if (!episode) return;
    projectStore.updateEpisodeAudioTrack(episode.id, index, { format });
  }

  function setAudioBitrate(index: number, kbps: number) {
    if (!episode) return;
    projectStore.updateEpisodeAudioTrack(episode.id, index, { ac3Kbps: kbps });
  }

  function setAudioDelay(index: number, value: number) {
    if (!episode || !Number.isFinite(value)) return;
    projectStore.updateEpisodeAudioTrack(episode.id, index, {
      delayMs: Math.max(-10_000, Math.min(10_000, Math.round(value))),
    });
  }

  function setSubtitleDelay(index: number, value: number) {
    if (!episode || !Number.isFinite(value)) return;
    projectStore.updateEpisodeSubtitleTrack(episode.id, index, {
      delayMs: Math.max(-10_000, Math.min(10_000, Math.round(value))),
    });
  }

  function setLanguage(kind: "audio" | "subtitle", index: number, language: string) {
    if (!episode) return;
    const normalized = normalizeLanguageCode(language);
    if (kind === "audio") {
      projectStore.updateEpisodeAudioTrack(episode.id, index, { language: normalized });
    } else {
      projectStore.updateEpisodeSubtitleTrack(episode.id, index, { language: normalized });
    }
  }

  function toggleDefaultAudio(index: number) {
    if (!episode) return;
    projectStore.setDefaultEpisodeAudio(episode.id, index);
  }

  function toggleDefaultSubtitle(index: number) {
    if (!episode) return;
    projectStore.setDefaultEpisodeSubtitle(episode.id, index);
  }

  async function addSubtitleFile() {
    if (!episode) return;
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Subtítulos"), extensions: ["srt", "ass", "ssa", "vtt"] }],
    });
    if (typeof picked !== "string") return;
    const name = basename(picked);
    const language = normalizeLanguageCode(name.match(/\.([a-z]{2,3})\./i)?.[1]);
    projectStore.addEpisodeSubtitleTrack(episode.id, picked, language, name);
  }

  function addEmptySubtitle() {
    if (!episode) return;
    projectStore.addEpisodeSubtitleTrack(episode.id, null, "und");
  }

  function offsetPercent(delayMs: number): number {
    if (duration <= 0) return 0;
    return Math.max(-100, Math.min(100, (delayMs / duration) * 100));
  }

  function shiftedBar(delayMs: number): string {
    const offset = offsetPercent(delayMs);
    if (offset >= 0) return `left: ${offset}%; width: ${Math.max(0, 100 - offset)}%;`;
    return `left: 0%; width: ${Math.max(0, 100 + offset)}%;`;
  }
</script>

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Pistas del episodio")}</span>
      {#if episode}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-semibold text-cyan-200">
          E{episode.number} · {episode.title}
        </span>
      {/if}
    </div>
    <p class="text-[10px] text-slate-500">
      {episode?.audio.length ?? 0} {t("audio")} · {episode?.subtitles.length ?? 0} {t((episode?.subtitles.length ?? 0) === 1 ? "subtítulo" : "subtítulos")} · {t("desfases en ms")}
    </p>
  </div>

  {#if !episode}
    <div class="flex flex-1 items-center justify-center p-6">
      <div class="max-w-sm rounded-2xl border border-dashed border-slate-800 p-8 text-center">
        <p class="text-sm font-medium text-slate-300">{t("Selecciona un episodio")}</p>
        <p class="mt-1 text-xs leading-5 text-slate-500">
          {t("Elige un episodio en la secuencia para editar sus pistas de audio y subtítulos.")}
        </p>
      </div>
    </div>
  {:else}
    <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_250px]">
      <div class="flex min-w-0 flex-col gap-3">
        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Línea de tiempo")}</h2>
            <div class="flex items-center gap-2 text-[10px] text-slate-500">
              <span class="rounded border border-slate-800 px-1.5 py-0.5">E{episode.number}</span>
              <span class="font-mono text-cyan-300">{formatDuration(duration)}</span>
            </div>
          </div>

          <div class="mt-3 space-y-1.5">
            <div class="grid grid-cols-[94px_minmax(0,1fr)] items-center gap-2">
              <span class="truncate text-[10px] font-medium text-slate-300">{t("Vídeo")}</span>
              <div class="relative h-7 overflow-hidden rounded-md border border-slate-800 bg-slate-950">
                <div class="absolute inset-y-0 left-0 w-full bg-gradient-to-r from-cyan-500/35 via-sky-500/30 to-emerald-500/25"></div>
                <span class="absolute left-2 top-1/2 -translate-y-1/2 text-[9px] font-semibold text-cyan-100">
                  H.264 / MPEG-2 · {scaleLabel(episode.scaleMode)}
                </span>
              </div>
            </div>

            {#each episode.audio as track, index (index)}
              <div class="grid grid-cols-[94px_minmax(0,1fr)] items-center gap-2">
                <span class="truncate text-[10px] text-slate-400">
                  Audio {index + 1}{track.default ? " · DEF" : ""}
                </span>
                <div class="relative h-6 overflow-hidden rounded-md border border-slate-800 bg-slate-950">
                  <div class="absolute inset-y-0 left-0 w-full bg-amber-500/10"></div>
                  <div class="absolute inset-y-0 rounded-sm bg-amber-400/30" style={shiftedBar(track.delayMs)}></div>
                  <span class="absolute left-2 top-1/2 -translate-y-1/2 text-[9px] text-amber-100/80">
                    {track.language.toUpperCase()} · {track.format === "ac3Surround51" ? "5.1" : "2.0"} · {track.ac3Kbps} kbps
                  </span>
                  {#if track.delayMs !== 0}
                    <span class="absolute right-1 top-1/2 -translate-y-1/2 rounded bg-slate-950/80 px-1 text-[8px] font-mono text-amber-200">
                      {track.delayMs > 0 ? "+" : ""}{track.delayMs} ms
                    </span>
                  {/if}
                </div>
              </div>
            {/each}

            {#each episode.subtitles as track, index (index)}
              <div class="grid grid-cols-[94px_minmax(0,1fr)] items-center gap-2">
                <span class="truncate text-[10px] text-slate-400">
                  Sub {index + 1}{track.isDefault ? " · DEF" : ""}{track.isForced ? " · FORZ" : ""}{track.bitmap ? " · BITMAP" : ""}
                </span>
                <div class="relative h-6 overflow-hidden rounded-md border border-slate-800 bg-slate-950">
                  <div class="absolute inset-y-0 left-0 w-full bg-violet-500/10"></div>
                  <div class="absolute inset-y-0 rounded-sm bg-violet-400/25" style={shiftedBar(track.delayMs)}></div>
                  <span class="absolute left-2 top-1/2 -translate-y-1/2 text-[9px] text-violet-100/80">
                    {track.language.toUpperCase()} · {track.sourcePath ? basename(track.sourcePath) : `stream ${track.sourceStream}`}
                  </span>
                </div>
              </div>
            {/each}
          </div>

          <div class="mt-2 flex justify-between border-t border-slate-800 pt-1 font-mono text-[8px] text-slate-600">
            <span>00:00</span>
            <span>25%</span>
            <span>50%</span>
            <span>75%</span>
            <span>{formatDuration(duration)}</span>
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex items-center justify-between gap-2">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              {t("Audio")} · {episode.audio.length}
            </h2>
            <div class="flex items-center gap-1">
              <select
                class="rounded-md border border-slate-800 bg-slate-950 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
                bind:value={audioToAdd}
                disabled={availableAudio.length === 0}
              >
                <option value="">{t("Flujo…")}</option>
                {#each availableAudio as stream}
                  <option value={String(stream.index)}>
                    #{stream.index} · {stream.language ?? "und"} · {stream.codec}
                  </option>
                {/each}
              </select>
              <button
                type="button"
                onclick={addAudio}
                disabled={availableAudio.length === 0}
                class="rounded-md bg-cyan-500 px-2 py-1 text-[10px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:opacity-40"
              >
                {t("+ Audio")}
              </button>
            </div>
          </div>

          <div class="mt-2 space-y-2">
            {#each episode.audio as track, index (index)}
              <div class="rounded-lg border border-slate-800 bg-slate-950/60 p-2">
                <div class="flex flex-wrap items-center gap-1.5">
                  <span class="rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[9px] text-amber-200">
                    A{index + 1}
                  </span>
                  {#if track.default}
                    <span class="rounded bg-cyan-500/15 px-1 py-0.5 text-[8px] font-bold text-cyan-200">DEF</span>
                  {/if}
                  {#if track.commentary}
                    <span class="rounded bg-fuchsia-500/15 px-1 py-0.5 text-[8px] font-bold text-fuchsia-200">COM</span>
                  {/if}
                  <input
                    class="min-w-0 flex-1 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={track.label}
                    aria-label={t("Etiqueta de audio")}
                    oninput={(event) =>
                      projectStore.updateEpisodeAudioTrack(episode.id, index, {
                        label: event.currentTarget.value,
                      })}
                  />
                  <div class="w-40">
                    <LanguageSelect
                      value={track.language}
                      onchange={(language: string) => setLanguage("audio", index, language)}
                    />
                  </div>
                  <select
                    class="rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
                    value={track.format}
                    onchange={(event) => setFormat(index, event.currentTarget.value as AudioFormat)}
                  >
                    <option value="ac3Stereo">{t("AC-3 2.0")}</option>
                    <option value="ac3Surround51">{t("AC-3 5.1")}</option>
                  </select>
                  <select
                    class="rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-300 outline-none focus:border-cyan-500/60"
                    value={track.ac3Kbps}
                    onchange={(event) => setAudioBitrate(index, Number(event.currentTarget.value))}
                  >
                    {#each [128, 192, 224, 256, 320, 384, 448] as kbps}
                      <option value={kbps}>{kbps} kbps</option>
                    {/each}
                  </select>
                  <label class="flex items-center gap-1 text-[9px] text-slate-400">
                    <input
                      type="checkbox"
                      checked={track.default}
                      onchange={() => toggleDefaultAudio(index)}
                      class="accent-cyan-500"
                    />
                    {t("Defecto")}
                  </label>
                  <button
                    type="button"
                    onclick={() => projectStore.removeEpisodeAudioTrack(episode.id, index)}
                    class="ml-auto rounded px-1 text-[10px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
                    title={t("Eliminar pista")}
                  >
                    ✕
                  </button>
                </div>
                <div class="mt-1.5 grid gap-2 sm:grid-cols-[minmax(0,1fr)_82px]">
                  <label class="block">
                    <span class="mb-0.5 flex justify-between text-[9px] text-slate-500">
                      <span>{t("Desfase")}</span>
                      <span class="font-mono">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                    </span>
                    <input
                      type="range"
                      min="-10000"
                      max="10000"
                      step="10"
                      value={track.delayMs}
                      oninput={(event) => setAudioDelay(index, Number(event.currentTarget.value))}
                      class="w-full accent-cyan-400"
                    />
                  </label>
                  <label class="block">
                    <span class="mb-0.5 block text-[9px] text-slate-500">{t("Exacto")}</span>
                    <input
                      type="number"
                      step="10"
                      class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                      value={track.delayMs}
                      onchange={(event) => setAudioDelay(index, Number(event.currentTarget.value))}
                    />
                  </label>
                </div>
              </div>
            {/each}
            {#if episode.audio.length === 0}
              <p class="rounded-lg border border-dashed border-slate-800 px-2 py-3 text-center text-[10px] text-slate-500">
                {t("Sin pistas de audio. La composición usará el audio por defecto del proyecto.")}
              </p>
            {/if}
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex items-center justify-between gap-2">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              {t("Subtítulos")} · {episode.subtitles.length}
            </h2>
            <div class="flex items-center gap-1">
              <button
                type="button"
                onclick={addSubtitleFile}
                class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
              >
                {t("+ .srt")}
              </button>
              <button
                type="button"
                onclick={addEmptySubtitle}
                class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 transition hover:border-cyan-500/60"
              >
                {t("+ Pista")}
              </button>
            </div>
          </div>
          <div class="mt-2 space-y-2">
            {#each episode.subtitles as track, index (index)}
              <div class="rounded-lg border border-slate-800 bg-slate-950/60 p-2">
                <div class="flex flex-wrap items-center gap-1.5">
                  <span class="rounded bg-violet-500/15 px-1.5 py-0.5 font-mono text-[9px] text-violet-200">
                    S{index + 1}
                  </span>
                  <input
                    class="min-w-0 flex-1 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={track.label}
                    aria-label={t("Etiqueta de subtítulo")}
                    oninput={(event) =>
                      projectStore.updateEpisodeSubtitleTrack(episode.id, index, {
                        label: event.currentTarget.value,
                      })}
                  />
                  <div class="w-40">
                    <LanguageSelect
                      value={track.language}
                      onchange={(language: string) => setLanguage("subtitle", index, language)}
                    />
                  </div>
                  <label class="flex items-center gap-1 text-[9px] text-slate-400">
                    <input
                      type="checkbox"
                      checked={track.isForced}
                      onchange={(event) =>
                        projectStore.updateEpisodeSubtitleTrack(episode.id, index, {
                          isForced: event.currentTarget.checked,
                        })}
                      class="accent-violet-500"
                    />
                    {t("Forzado")}
                  </label>
                  <label class="flex items-center gap-1 text-[9px] text-slate-400">
                    <input
                      type="checkbox"
                      checked={track.isDefault}
                      onchange={() => toggleDefaultSubtitle(index)}
                      class="accent-violet-500"
                    />
                    {t("Defecto")}
                  </label>
                  <button
                    type="button"
                    onclick={() => projectStore.removeEpisodeSubtitleTrack(episode.id, index)}
                    class="ml-auto rounded px-1 text-[10px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
                    title={t("Eliminar subtítulo")}
                  >
                    ✕
                  </button>
                </div>
                <p class="mt-1 truncate font-mono text-[9px] text-slate-600" title={track.sourcePath ?? ""}>
                  {track.sourcePath ? basename(track.sourcePath) : `stream interno ${track.sourceStream}`}
                </p>
                <div class="mt-1.5 grid gap-2 sm:grid-cols-[minmax(0,1fr)_82px]">
                  <label class="block">
                    <span class="mb-0.5 flex justify-between text-[9px] text-slate-500">
                      <span>{t("Desfase")}</span>
                      <span class="font-mono">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                    </span>
                    <input
                      type="range"
                      min="-10000"
                      max="10000"
                      step="10"
                      value={track.delayMs}
                      oninput={(event) => setSubtitleDelay(index, Number(event.currentTarget.value))}
                      class="w-full accent-violet-400"
                    />
                  </label>
                  <label class="block">
                    <span class="mb-0.5 block text-[9px] text-slate-500">{t("Exacto")}</span>
                    <input
                      type="number"
                      step="10"
                      class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-violet-500/60"
                      value={track.delayMs}
                      onchange={(event) => setSubtitleDelay(index, Number(event.currentTarget.value))}
                    />
                  </label>
                </div>
              </div>
            {/each}
            {#if episode.subtitles.length === 0}
              <p class="rounded-lg border border-dashed border-slate-800 px-2 py-3 text-center text-[10px] text-slate-500">
                {t("Sin subtítulos. Añade un archivo .srt o una pista interna.")}
              </p>
            {/if}
          </div>
        </div>
      </div>

      <aside class="flex min-h-0 flex-col gap-2">
        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Resumen")}</h2>
          <dl class="mt-2 space-y-1 text-[10px]">
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Episodio")}</dt>
              <dd class="truncate text-slate-300">{episode.title}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Duración")}</dt>
              <dd class="font-mono text-slate-300">{formatDuration(duration)}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Audio por defecto")}</dt>
              <dd class="truncate text-slate-300">{defaultAudio?.label ?? "—"}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Subtítulo por defecto")}</dt>
              <dd class="truncate text-slate-300">{defaultSubtitle?.label ?? "—"}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Escalado")}</dt>
              <dd class="text-slate-300">{scaleLabel(episode.scaleMode)}</dd>
            </div>
          </dl>
          <label class="mt-2 block">
            <span class="mb-1 block text-[9px] text-slate-500">{t("Escalado del vídeo")}</span>
            <select
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={episode.scaleMode}
              onchange={(event) =>
                projectStore.setEpisodeScaleMode(
                  episode.id,
                  event.currentTarget.value as VideoScaleMode,
                )}
            >
              {#each scaleModes as mode}
                <option value={mode.id}>{t(mode.label)}</option>
              {/each}
            </select>
          </label>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Compatibilidad")}</h2>
          <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
            {t("DVD-Video admite hasta 8 pistas de audio y 32 subimágenes; AVCHD y Blu-ray priorizan la primera pista AC-3. Mantén una pista como predeterminada y usa desfases de ±10 s para corregir desincronizaciones.")}
          </p>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Rutas de origen")}</h2>
          <ul class="mt-1.5 space-y-1 text-[9px] text-slate-500">
            <li class="truncate" title={asset?.displayName ?? ""}>Vídeo: {asset?.displayName ?? "—"}</li>
            {#each episode.subtitles as track, index (index)}
              <li class="truncate" title={track.sourcePath ?? ""}>
                S{index + 1}: {track.sourcePath ? basename(track.sourcePath) : `stream ${track.sourceStream}`}
              </li>
            {/each}
          </ul>
        </div>
      </aside>
    </div>
  {/if}
</section>
