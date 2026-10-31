<script lang="ts">
  import { onMount } from "svelte";
  import { commands, type MenuTemplateInfo } from "../api/bindings";
  import { messageOf } from "../api/client";
  import { projectStore } from "../stores/project.svelte";

  let templates = $state<MenuTemplateInfo[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let applying = $state<string | null>(null);

  onMount(() => {
    void load();
  });

  async function load() {
    loading = true;
    try {
      templates = await commands.menuTemplates();
    } catch (cause) {
      error = messageOf(cause);
    } finally {
      loading = false;
    }
  }

  async function apply(id: string) {
    applying = id;
    error = null;
    const ok = await projectStore.applyMenuTemplate(id as MenuTemplateInfo["id"]);
    if (!ok) error = projectStore.error;
    applying = null;
  }

  function swatch(rgb: [number, number, number]) {
    return "rgb(" + rgb[0] + "," + rgb[1] + "," + rgb[2] + ")";
  }
</script>

<section class="rounded-xl border border-slate-800 bg-slate-900/60 p-3">
  <div class="mb-2 flex items-center justify-between gap-2">
    <div>
      <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">Plantillas de menú</p>
      <p class="mt-0.5 text-[11px] text-slate-400">Aplicación no destructiva con paginación automática.</p>
    </div>
    <button
      type="button"
      class="rounded-md border border-slate-700 px-2 py-1 text-[10px] text-slate-300 hover:border-cyan-500/60 hover:text-cyan-200"
      onclick={load}
      disabled={loading}
    >Recargar</button>
  </div>
  {#if error}<p class="mb-2 rounded bg-rose-500/10 px-2 py-1 text-[10px] text-rose-200">{error}</p>{/if}
  {#if loading}
    <p class="text-[11px] text-slate-500">Cargando plantillas…</p>
  {:else}
    <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
      {#each templates as template (template.id)}
        <button
          type="button"
          class="group rounded-lg border border-slate-800 bg-slate-950/70 p-2 text-left transition hover:border-cyan-500/60"
          onclick={() => apply(template.id)}
          disabled={applying !== null || !projectStore.project}
        >
          <div class="flex items-center justify-between gap-2">
            <span class="text-[11px] font-semibold text-slate-200">{template.name}</span>
            <span class="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[9px] text-slate-400">{template.columns}x{template.rows}</span>
          </div>
          <p class="mt-1 line-clamp-2 text-[10px] leading-4 text-slate-500">{template.description}</p>
          <div class="mt-2 flex items-center gap-1">
            {#each template.palette.colors as color}
              <span class="h-3 w-3 rounded-sm border border-white/10" style={"background:" + swatch(color.rgb)}></span>
            {/each}
            <span class="ml-auto text-[9px] text-slate-500">{template.episodeCapacity} por página</span>
          </div>
          {#if applying === template.id}<p class="mt-1 text-[9px] text-cyan-300">Aplicando…</p>{/if}
        </button>
      {/each}
    </div>
    {#if projectStore.templateDiagnostics.length > 0}
      <ul class="mt-2 space-y-1">
        {#each projectStore.templateDiagnostics as diagnostic}
          <li class="rounded bg-amber-500/10 px-2 py-1 text-[9px] text-amber-200">{diagnostic}</li>
        {/each}
      </ul>
    {/if}
  {/if}
</section>
