<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";

  let {
    value,
    recommended,
    installed,
    onchange,
    label,
  }: {
    value: string;
    recommended: string[];
    installed: string[];
    onchange: (family: string) => void;
    label: string;
  } = $props();

  let open = $state(false);
  let query = $state("");
  let active = $state(0);
  let root: HTMLDivElement | undefined = $state();
  let search: HTMLInputElement | undefined = $state();

  const others = $derived(installed.filter((family) => !recommended.includes(family)));
  const filtered = $derived.by(() => {
    const needle = query.trim().toLowerCase();
    const match = (family: string) => !needle || family.toLowerCase().includes(needle);
    return [
      ...recommended.filter(match).map((family) => ({ family, group: "recommended" as const })),
      ...others.filter(match).map((family) => ({ family, group: "installed" as const })),
    ];
  });

  function css(family: string): string {
    return `font-family: "${family.replaceAll('"', "")}", system-ui, sans-serif`;
  }

  function toggle() {
    open = !open;
    if (open) {
      query = "";
      active = Math.max(0, filtered.findIndex((item) => item.family === value));
      queueMicrotask(() => search?.focus());
    }
  }

  function choose(family: string) {
    onchange(family);
    open = false;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown") {
      active = Math.min(filtered.length - 1, active + 1);
      event.preventDefault();
    } else if (event.key === "ArrowUp") {
      active = Math.max(0, active - 1);
      event.preventDefault();
    } else if (event.key === "Enter") {
      const item = filtered[active];
      if (item) choose(item.family);
      else if (query.trim()) choose(query.trim());
      event.preventDefault();
    } else if (event.key === "Escape") {
      open = false;
      event.stopPropagation();
    }
  }

  $effect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (root && !root.contains(event.target as Node)) open = false;
    };
    window.addEventListener("pointerdown", close, true);
    return () => window.removeEventListener("pointerdown", close, true);
  });

  $effect(() => {
    // Keep the highlighted row in view while moving with the keyboard.
    void active;
    root?.querySelector<HTMLElement>(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  });
</script>

<div class="relative" bind:this={root}>
  <span class="mb-1 block text-[12px] text-slate-400">{label}</span>
  <button
    type="button"
    class="flex h-8 w-full items-center justify-between gap-2 rounded-md border border-slate-700 bg-slate-900 px-2.5 text-left text-[14px] text-slate-100 hover:border-slate-600"
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={toggle}
  >
    <span class="truncate" style={css(value)}>{value}</span>
    <Icon name="chevronDown" size={14} class="text-slate-400" />
  </button>

  {#if open}
    <div class="popover absolute left-0 right-0 top-full z-30 mt-1 overflow-hidden rounded-md border border-slate-700 bg-slate-900 shadow-xl">
      <div class="flex items-center gap-2 border-b border-slate-800 px-2.5">
        <Icon name="search" size={14} class="text-slate-500" />
        <input
          bind:this={search}
          bind:value={query}
          {onkeydown}
          oninput={() => (active = 0)}
          class="h-8 w-full bg-transparent text-[13px] text-slate-100 outline-none placeholder:text-slate-500"
          placeholder={t("Buscar fuente…")}
          aria-label={t("Buscar fuente…")}
        />
      </div>
      <ul class="max-h-64 overflow-y-auto py-1" role="listbox" aria-label={label}>
        {#each filtered as item, index (item.group + item.family)}
          {#if index === 0 || filtered[index - 1].group !== item.group}
            <li class="px-2.5 pb-1 pt-2 text-[11px] font-semibold text-slate-500" role="presentation">
              {item.group === "recommended" ? t("Recomendadas para menús") : t("Instaladas en este equipo")}
            </li>
          {/if}
          <li role="option" aria-selected={item.family === value} data-index={index}>
            <button
              type="button"
              class={`flex w-full items-center justify-between gap-2 px-2.5 py-1.5 text-left text-[15px] ${
                index === active ? "bg-slate-800 text-slate-50" : "text-slate-200"
              }`}
              onpointerenter={() => (active = index)}
              onclick={() => choose(item.family)}
            >
              <span class="truncate" style={css(item.family)}>{item.family}</span>
              {#if item.family === value}<Icon name="check" size={14} class="text-cyan-400" />{/if}
            </button>
          </li>
        {:else}
          <li class="px-2.5 py-3 text-[12px] text-slate-500">{t("No hay fuentes con ese nombre.")}</li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .popover {
    transform-origin: top center;
    animation: pop 150ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97) translateY(-2px);
    }
  }
</style>
