<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "../i18n/index.svelte";
  import { projectStore } from "../stores/project.svelte";
  import { MEDIA_EXTENSIONS } from "../actions";
  import Icon from "../ui/Icon.svelte";
  import { basename } from "../utils/format";

  let { onclose }: { onclose: () => void } = $props();

  const VIDEO_EXTENSIONS = MEDIA_EXTENSIONS.slice(0, 12);

  let clips = $state<string[]>([]);
  let title = $state("");
  let working = $state(false);
  let error = $state<string | null>(null);

  const canJoin = $derived(!working && clips.length >= 2);

  async function pickClips() {
    const picked = await open({
      multiple: true,
      filters: [{ name: t("Vídeos"), extensions: VIDEO_EXTENSIONS }],
    });
    if (!picked) return;
    const list = Array.isArray(picked) ? picked : [picked];
    // Picked files come sorted by name: camcorder parts stay in order.
    const added = list.filter((path) => !clips.includes(path)).sort((a, b) => a.localeCompare(b));
    clips = [...clips, ...added];
    if (!title.trim() && clips.length > 0) title = basename(clips[0]).replace(/\.[^.]+$/, "");
  }

  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= clips.length) return;
    const next = [...clips];
    [next[index], next[target]] = [next[target], next[index]];
    clips = next;
  }

  async function join() {
    working = true;
    error = null;
    const ok = await projectStore.createJoin(clips, title.trim() || t("Vídeos unidos"));
    working = false;
    if (ok) onclose();
    else error = projectStore.error;
  }
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && !working && onclose()} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button type="button" aria-label={t("Cerrar")} class="ui-backdrop" onclick={() => !working && onclose()}></button>
  <div class="ui-dialog max-h-[90vh] w-full max-w-xl" role="dialog" aria-modal="true" aria-labelledby="join-title">
    <header class="ui-dialog-header">
      <div>
        <h2 id="join-title" class="ui-title">{t("Unir vídeos")}</h2>
        <p class="ui-hint mt-0.5">
          {t("Varios vídeos se convierten en un solo episodio, con un capítulo al principio de cada uno.")}
        </p>
      </div>
    </header>

    <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
      <label class="block">
        <span class="ui-label">{t("Título del episodio")}</span>
        <input class="ui-input w-full" bind:value={title} placeholder={t("Vídeos unidos")} />
      </label>

      <div class="flex items-center justify-between gap-3">
        <span class="ui-hint">{t("{count} vídeos, en este orden", { count: clips.length })}</span>
        <button type="button" class="ui-btn" onclick={pickClips} disabled={working}>
          <Icon name="add" size={14} />{t("Añadir vídeos…")}
        </button>
      </div>

      {#if clips.length > 0}
        <ol class="max-h-64 overflow-y-auto rounded-md border border-slate-800">
          {#each clips as clip, index (clip)}
            <li class="flex items-center gap-2 border-b border-slate-800 px-2.5 py-1 last:border-0">
              <span class="w-6 text-right font-mono text-[11px] text-slate-500">{index + 1}</span>
              <span class="min-w-0 flex-1 truncate text-[13px] text-slate-200" title={clip}>{basename(clip)}</span>
              <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon" aria-label={t("Subir")} onclick={() => move(index, -1)} disabled={index === 0 || working}>
                <Icon name="undo" size={14} class="rotate-90" />
              </button>
              <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon" aria-label={t("Bajar")} onclick={() => move(index, 1)} disabled={index === clips.length - 1 || working}>
                <Icon name="redo" size={14} class="rotate-90" />
              </button>
              <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon" aria-label={t("Quitar")} onclick={() => (clips = clips.filter((_, i) => i !== index))} disabled={working}>
                <Icon name="close" size={14} />
              </button>
            </li>
          {/each}
        </ol>
      {:else}
        <div class="ui-empty py-6">
          <p class="ui-hint">{t("Por ejemplo, las partes de una filmación o los vídeos del celular de un mismo día.")}</p>
        </div>
      {/if}

      {#if error}
        <p class="rounded-md bg-rose-500/10 px-3 py-2 text-[12px] text-rose-200" role="alert">{error}</p>
      {/if}
    </div>

    <footer class="ui-dialog-footer">
      <button type="button" class="ui-btn ui-btn-ghost" onclick={onclose} disabled={working}>{t("Cancelar")}</button>
      <button type="button" class="ui-btn ui-btn-primary" onclick={join} disabled={!canJoin}>
        {working ? t("Uniendo…") : t("Unir y añadir")}
      </button>
    </footer>
  </div>
</div>
