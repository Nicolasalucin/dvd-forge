<script lang="ts">
  import { onMount } from "svelte";
  import { commands, type MenuTemplateInfo } from "../api/bindings";
  import { messageOf } from "../api/client";
  import { projectStore } from "../stores/project.svelte";
  import { t } from "../i18n/index.svelte";
  import Icon from "../ui/Icon.svelte";

  let { onapplied }: { onapplied?: () => void } = $props();

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
    else onapplied?.();
    applying = null;
  }

  const hasChapters = $derived(
    (projectStore.project?.episodes ?? []).some((episode) => episode.chapters.length > 0),
  );

  async function makeScenes() {
    applying = "scenes";
    error = null;
    const ok = await projectStore.applyChapterMenus();
    if (!ok) error = projectStore.error;
    else onapplied?.();
    applying = null;
  }

  function swatch(rgb: [number, number, number]) {
    return "rgb(" + rgb[0] + "," + rgb[1] + "," + rgb[2] + ")";
  }
</script>

<section class="flex min-h-0 flex-col">
  <div class="flex items-center justify-between gap-2 px-3 pb-2 pt-3">
    <div>
      <p class="ui-title">{t("Plantillas de menú")}</p>
      <p class="ui-hint">{t("Aplicación no destructiva con paginación automática.")}</p>
    </div>
    {#if hasChapters}
      <button
        type="button"
        class="ui-btn ui-btn-sm"
        onclick={makeScenes}
        disabled={applying !== null || !projectStore.project}
        title={t("Crea un menú con un botón por capítulo para cada episodio")}
      ><Icon name="layers" size={14} />{t("Menús de escenas")}</button>
    {/if}
  </div>
  {#if error}<p class="mx-3 mb-2 rounded-md bg-rose-500/10 px-2.5 py-1.5 text-[12px] text-rose-200" role="alert">{error}</p>{/if}
  <div class="min-h-0 overflow-y-auto px-3 pb-3">
    {#if loading}
      <p class="ui-hint">{t("Cargando plantillas…")}</p>
    {:else}
      <div class="grid grid-cols-2 gap-2 lg:grid-cols-3">
        {#each templates as template (template.id)}
          <button
            type="button"
            class="rounded-md border border-slate-700 p-2 text-left hover:border-cyan-500 disabled:opacity-50"
            onclick={() => apply(template.id)}
            disabled={applying !== null || !projectStore.project}
          >
            <div
              class="grid h-14 gap-1 rounded p-1.5"
              style={`background:${swatch(template.palette.colors[0]?.rgb ?? [16, 16, 20])}; grid-template-columns: repeat(${Math.min(template.columns, 4)}, 1fr)`}
            >
              {#each Array(Math.min(template.columns * template.rows, 8)) as _, index}
                <span
                  class="rounded-sm"
                  style={`background:${swatch(template.palette.colors[index === 0 ? 2 : 1]?.rgb ?? [90, 90, 100])}; opacity:${index === 0 ? 1 : 0.55}`}
                ></span>
              {/each}
            </div>
            <div class="mt-1.5 flex items-center justify-between gap-2">
              <span class="truncate text-[12px] font-medium text-slate-100">{template.name}</span>
              <span class="ui-chip font-mono">{template.columns}×{template.rows}</span>
            </div>
            <p class="mt-0.5 line-clamp-2 text-[12px] leading-5 text-slate-500">{t(template.description)}</p>
            {#if applying === template.id}<p class="mt-1 text-[11px] text-cyan-300">{t("Aplicando…")}</p>{/if}
          </button>
        {/each}
      </div>
      {#if projectStore.templateDiagnostics.length > 0}
        <ul class="mt-2 space-y-1">
          {#each projectStore.templateDiagnostics as diagnostic}
            <li class="rounded bg-amber-500/10 px-2 py-1 text-[12px] text-amber-200">{t(diagnostic)}</li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</section>
