<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";
  import { t } from "../i18n/index.svelte";
  import Icon from "../ui/Icon.svelte";

  let { onclose }: { onclose: () => void } = $props();
  let version = $state("");

  /** People who found bugs and suggested improvements (no @: no pings). */
  const testers = ["OhItsStefan", "Infinite_Silver483", "Narrow-Chard4152", "Mirandopacuenca67", "Carllito7"];

  onMount(() => {
    void getVersion()
      .then((value) => (version = value))
      .catch(() => undefined);
  });

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Escape") onclose();
  }
</script>

<svelte:window {onkeydown} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" aria-label={t("Cerrar")} class="absolute inset-0 cursor-default bg-black/50" onclick={onclose}></button>
  <div class="relative z-10 w-full max-w-md rounded-lg border border-slate-700 bg-slate-900 p-6 shadow-xl" role="dialog" aria-modal="true" aria-labelledby="about-title">
    <div class="flex items-center gap-3">
      <Icon name="disc" size={36} class="text-cyan-400" />
      <div>
        <h2 id="about-title" class="text-[16px] font-semibold text-slate-100">Kinodisc</h2>
        <p class="text-[12px] text-slate-400">{t("Versión {version}", { version: version || "—" })}</p>
      </div>
    </div>
    <p class="mt-4 text-[13px] leading-5 text-slate-300">
      {t("Crea DVD, AVCHD y Blu-ray con menús a partir de tus vídeos.")}
    </p>

    <h3 class="mt-5 text-[12px] font-semibold text-slate-200">{t("Agradecimientos")}</h3>
    <p class="mt-1 text-[12px] leading-5 text-slate-400">
      {t("Gracias por encontrar errores y proponer mejoras:")}
    </p>
    <ul class="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-[13px] text-slate-200">
      {#each testers as name}<li>{name}</li>{/each}
    </ul>

    <h3 class="mt-5 text-[12px] font-semibold text-slate-200">{t("Licencias")}</h3>
    <p class="mt-1 text-[12px] leading-5 text-slate-400">
      {t("Kinodisc es software libre. FFmpeg y DVDAuthor se descargan aparte, verificados, con sus propias licencias (LGPL/GPL).")}
    </p>
    <p class="mt-2 text-[12px] text-slate-400">github.com/Nicolasalucin/kinodisc</p>

    <div class="mt-6 flex justify-end">
      <button type="button" class="on-accent rounded-md bg-cyan-500 px-4 py-1.5 text-[13px] font-semibold hover:bg-cyan-400" onclick={onclose}>{t("Cerrar")}</button>
    </div>
  </div>
</div>
