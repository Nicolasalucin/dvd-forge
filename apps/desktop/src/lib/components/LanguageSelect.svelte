<script lang="ts">
  import { languageLabel, LANGUAGES, normalizeLanguageCode } from "../i18n/languages";
  import { i18n, t } from "../i18n/index.svelte";

  interface Props {
    value: string;
    onchange: (value: string) => void;
    disabled?: boolean;
  }

  let { value, onchange, disabled = false }: Props = $props();
  let open = $state(false);
  let query = $state("");
  let wrapper: HTMLDivElement;
  const current = $derived(normalizeLanguageCode(value));
  const selected = $derived(languageLabel(current, i18n.locale));
  const filtered = $derived(
    LANGUAGES.filter((language) => {
      const label = languageLabel(language.code, i18n.locale).toLowerCase();
      const needle = query.trim().toLowerCase();
      return !needle || label.includes(needle) || language.code.includes(needle);
    }),
  );

  function choose(code: string) {
    onchange(normalizeLanguageCode(code));
    open = false;
    query = "";
  }
</script>

<div
  class="relative min-w-0"
  bind:this={wrapper}
  onfocusout={() => {
    setTimeout(() => {
      if (wrapper && !wrapper.contains(document.activeElement)) open = false;
    }, 120);
  }}
>
  <button
    type="button"
    {disabled}
    onclick={() => {
      open = !open;
      query = "";
    }}
    class="flex w-full items-center justify-between gap-1 rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 text-left text-[10px] text-slate-200 outline-none transition hover:border-cyan-500/50 disabled:opacity-50"
    aria-haspopup="listbox"
    aria-expanded={open}
    title={selected}
  >
    <span class="truncate">{selected}</span>
    <span class="text-[8px] text-slate-500">▼</span>
  </button>
  {#if open}
    <div class="absolute bottom-full left-0 z-50 mb-1 w-64 rounded-lg border border-slate-700 bg-slate-950 p-1.5 shadow-2xl shadow-black/60">
      <input
        class="w-full rounded-md border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-100 outline-none focus:border-cyan-500/60"
        placeholder={t("Buscar idioma…")}
        bind:value={query}
      />
      <div class="mt-1 max-h-48 overflow-y-auto">
        {#each filtered as language (language.code)}
          <button
            type="button"
            onclick={() => choose(language.code)}
            class={`flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-[10px] transition ${
              language.code === current
                ? "bg-cyan-500/15 text-cyan-100"
                : "text-slate-300 hover:bg-slate-800"
            }`}
            role="option"
            aria-selected={language.code === current}
          >
            <span class="truncate">{languageLabel(language.code, i18n.locale)}</span>
          </button>
        {/each}
        {#if filtered.length === 0}
          <p class="px-2 py-1.5 text-[10px] text-slate-500">{t("Sin resultados.")}</p>
        {/if}
      </div>
    </div>
  {/if}
</div>
