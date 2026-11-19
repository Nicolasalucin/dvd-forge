<script lang="ts">
  import { onMount } from "svelte";
  import { commands, type Episode_Serialize, type QualitySample } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { t, i18n } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";

  interface Props {
    episode: Episode_Serialize;
    /** Offset inside the episode where the 10 s sample starts. */
    atMs?: number;
    onclose: () => void;
  }

  let { episode, atMs, onclose }: Props = $props();

  let sample = $state<QualitySample | null>(null);
  let original = $state<string | null>(null);
  let disc = $state<string | null>(null);
  let error = $state<string | null>(null);
  let split = $state(50);

  const kbps = $derived(
    projectStore.plan?.perEpisode.find((entry) => entry.episode === episode.id)?.videoKbps ??
      projectStore.plan?.videoKbps ??
      0,
  );

  const verdict = $derived.by(() => {
    const psnr = sample?.psnrDb ?? 0;
    if (psnr >= 42) return { label: t("Excelente: la compresión no se nota"), tone: "text-emerald-300" };
    if (psnr >= 38) return { label: t("Muy buena: solo se nota mirando muy de cerca"), tone: "text-emerald-300" };
    if (psnr >= 34) return { label: t("Buena: puede notarse en escenas con mucho movimiento"), tone: "text-amber-300" };
    return {
      label: t("Se notará la compresión: usa un disco más grande o reparte en varios discos"),
      tone: "text-rose-300",
    };
  });

  onMount(async () => {
    const project = projectStore.project;
    if (!project) return;
    if (!kbps) {
      error = t("Primero se tiene que calcular el plan del disco.");
      return;
    }
    const duration = projectStore.episodeEffectiveDuration(episode);
    const start = atMs ?? Math.max(0, Math.round(duration / 2 - 5_000));
    try {
      const result = await unwrap(commands.renderQualitySample(project, episode.id, start, kbps));
      sample = result;
      original = await unwrap(commands.readImageDataUrl(result.originalPng));
      disc = await unwrap(commands.readImageDataUrl(result.discPng));
    } catch (cause) {
      error = messageOf(cause);
    }
  });

  function seconds(ms: number): string {
    const total = Math.round(ms / 1000);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  }
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && onclose()} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" aria-label={t("Cerrar")} class="ui-backdrop" onclick={onclose}></button>
  <div class="ui-dialog w-full max-w-4xl" role="dialog" aria-modal="true" aria-labelledby="quality-title">
    <header class="ui-dialog-header">
      <div>
        <h2 id="quality-title" class="ui-title">{t("Probar calidad")} · {episode.title}</h2>
        <p class="ui-hint mt-0.5">
          {t("10 segundos codificados exactamente como en el disco, a {kbps} kbps.", { kbps: kbps.toLocaleString(i18n.locale) })}
        </p>
      </div>
    </header>

    <div class="px-5 py-4">
      {#if error}
        <p class="rounded-md bg-rose-500/10 px-3 py-2 text-[12px] text-rose-200" role="alert">{error}</p>
      {:else if !sample || !original || !disc}
        <div class="ui-empty py-16">
          <p class="text-[13px] text-slate-300">{t("Codificando la muestra…")}</p>
          <p class="ui-hint mt-1">{t("Tarda unos segundos: usa el mismo codificador que el disco.")}</p>
        </div>
      {:else}
        <div class="relative mx-auto aspect-video w-full select-none overflow-hidden rounded-md border border-slate-800 bg-black">
          <img src={original} alt={t("Original")} class="absolute inset-0 h-full w-full object-contain" />
          <img
            src={disc}
            alt={t("Disco")}
            class="absolute inset-0 h-full w-full object-contain"
            style={`clip-path: inset(0 0 0 ${split}%)`}
          />
          <div class="pointer-events-none absolute inset-y-0 w-px bg-[#fff]" style={`left: ${split}%`}></div>
          <span class="absolute left-2 top-2 rounded bg-black/70 px-1.5 py-0.5 text-[11px] text-[#fff]">{t("Original")}</span>
          <span class="absolute right-2 top-2 rounded bg-black/70 px-1.5 py-0.5 text-[11px] text-[#fff]">{t("Disco")}</span>
        </div>
        <input
          type="range"
          min="0"
          max="100"
          bind:value={split}
          class="mt-2 w-full"
          aria-label={t("Comparar")}
        />
        <div class="mt-2 flex flex-wrap items-baseline justify-between gap-2">
          <p class="text-[14px] font-medium {verdict.tone}">{verdict.label}</p>
          <p class="font-mono text-[12px] text-slate-400">
            PSNR {(sample.psnrDb ?? 0).toFixed(1)} dB · {seconds(sample.startMs ?? 0)}–{seconds((sample.startMs ?? 0) + (sample.seconds ?? 0) * 1000)}
          </p>
        </div>
      {/if}
    </div>

    <footer class="ui-dialog-footer">
      <button type="button" class="ui-btn ui-btn-primary" onclick={onclose}>{t("Cerrar")}</button>
    </footer>
  </div>
</div>
