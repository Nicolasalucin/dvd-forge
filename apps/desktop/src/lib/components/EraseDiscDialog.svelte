<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { commands } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import Icon from "../ui/Icon.svelte";
  import { ui } from "../ui/state.svelte";
  import { confirmDelete } from "../utils/confirm";
  import BurnErrorBox from "./BurnErrorBox.svelte";

  let { onclose }: { onclose: () => void } = $props();

  let selected = $state("");
  let full = $state(false);
  let working = $state(false);
  let error = $state<string | null>(null);
  let done = $state<{ seconds: number; full: boolean } | null>(null);
  let startedAt = $state(0);
  let now = $state(Date.now());
  let timer: ReturnType<typeof setInterval> | null = null;

  const devices = $derived(projectStore.devices);
  const device = $derived(devices.some((item) => item.id === selected) ? selected : (devices[0]?.id ?? ""));
  const elapsed = $derived(working && startedAt ? Math.max(0, Math.floor((now - startedAt) / 1000)) : 0);

  function clock(seconds: number): string {
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  }

  onMount(() => {
    void projectStore.refreshDevices();
  });

  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  async function erase() {
    if (!device || working) return;
    const question = full
      ? t("¿Borrar todo el contenido del disco que está en {drive}? El borrado completo puede tardar más de 15 minutos y no se puede cancelar.", { drive: device })
      : t("¿Borrar todo el contenido del disco que está en {drive}? No se puede deshacer.", { drive: device });
    if (!(await confirmDelete(question))) return;
    working = true;
    ui.locked = true;
    error = null;
    done = null;
    startedAt = Date.now();
    now = startedAt;
    timer = setInterval(() => (now = Date.now()), 1000);
    try {
      const report = await unwrap(commands.burnErase(device, full));
      done = { seconds: Math.round(Number(report.elapsedMs ?? 0) / 1000), full: report.full };
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      if (timer) clearInterval(timer);
      timer = null;
      working = false;
      ui.locked = false;
    }
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" class="ui-backdrop" onclick={() => !working && onclose()} aria-label={t("Cerrar")}></button>
  <div class="ui-dialog max-w-lg" role="dialog" aria-modal="true" aria-labelledby="erase-disc-title">
    <header class="flex items-start gap-3 border-b border-slate-800 px-5 py-4">
      <Icon name="disc" size={20} class="mt-0.5 text-cyan-400" />
      <div>
        <h2 id="erase-disc-title" class="text-[15px] font-semibold text-slate-100">{t("Borrar disco regrabable")}</h2>
        <p class="ui-hint mt-0.5">
          {t("Deja un DVD-RW, DVD+RW o BD-RE listo para volver a grabarlo. Los DVD-R y DVD+R no se pueden borrar.")}
        </p>
      </div>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
      <div>
        <div class="flex items-center justify-between gap-2">
          <span class="ui-label">{t("Grabadora")}</span>
          <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost" onclick={() => projectStore.refreshDevices()} disabled={working}>
            {t("Actualizar")}
          </button>
        </div>
        {#if devices.length === 0}
          <p class="mt-1 rounded-md bg-amber-500/10 px-2.5 py-1.5 text-[12px] leading-5 text-amber-200/80">
            {t("No se detectaron unidades ópticas. Conecta la grabadora y pulsa «Actualizar».")}
          </p>
        {:else}
          <select class="ui-input mt-1 w-full" value={device} onchange={(event) => (selected = event.currentTarget.value)} disabled={working}>
            {#each devices as item}
              <option value={item.id}>{item.id} · {item.vendor} {item.model}</option>
            {/each}
          </select>
        {/if}
      </div>

      <fieldset class="space-y-1.5" disabled={working}>
        <legend class="ui-label">{t("Tipo de borrado")}</legend>
        <label class="flex items-start gap-2 text-[12px] text-slate-300">
          <input type="radio" name="erase-mode" class="mt-0.5 accent-cyan-500" checked={!full} onchange={() => (full = false)} />
          <span>{t("Rápido")}<span class="ui-hint block">{t("Unos minutos. Alcanza para volver a grabar el disco.")}</span></span>
        </label>
        <label class="flex items-start gap-2 text-[12px] text-slate-300">
          <input type="radio" name="erase-mode" class="mt-0.5 accent-cyan-500" checked={full} onchange={() => (full = true)} />
          <span>{t("Completo")}<span class="ui-hint block">{t("Reescribe todo el disco. Más lento; úsalo si un disco da errores.")}</span></span>
        </label>
      </fieldset>

      {#if working}
        <p class="rounded-md bg-cyan-500/10 px-3 py-2 text-[12px] text-cyan-200" role="status">
          {t("Borrando el disco… {time}", { time: clock(elapsed) })}
          <span class="block text-cyan-100/70">{t("No saques el disco ni desconectes la grabadora.")}</span>
        </p>
      {/if}
      {#if done}
        <p class="rounded-md border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-[12px] text-emerald-200" role="status">
          {t("Disco borrado en {time}. Ya puedes grabarlo.", { time: clock(done.seconds) })}
        </p>
      {/if}
      {#if error}
        <BurnErrorBox message={error} action={t("Borrar disco")} device={device} />
      {/if}
    </div>

    <footer class="ui-dialog-footer">
      <button type="button" class="ui-btn ui-btn-ghost" onclick={onclose} disabled={working}>{t("Cerrar")}</button>
      <button type="button" class="ui-btn ui-btn-primary" onclick={erase} disabled={working || !device}>
        {working ? t("Borrando…") : t("Borrar disco")}
      </button>
    </footer>
  </div>
</div>
