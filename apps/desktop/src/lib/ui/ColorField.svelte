<script lang="ts">
  import { t } from "../i18n/index.svelte";

  let {
    value,
    label,
    onchange,
  }: { value: string; label: string; onchange: (hex: string) => void } = $props();

  // Colours that read well on TV menus, then the last ones used.
  const PRESETS = [
    "#ffffff", "#d9d9d9", "#8c8c8c", "#262626", "#000000", "#f2c94c",
    "#f2994a", "#eb5757", "#bb6bd9", "#5390ff", "#56ccf2", "#27ae60",
  ];
  const RECENT_KEY = "kinodisc_recent_colors";

  let open = $state(false);
  let draft = $state("");
  let recent = $state<string[]>(readRecent());
  let root: HTMLDivElement | undefined = $state();

  function readRecent(): string[] {
    try {
      const parsed = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? "[]");
      return Array.isArray(parsed) ? parsed.filter((item) => /^#[0-9a-f]{6}$/i.test(item)).slice(0, 6) : [];
    } catch {
      return [];
    }
  }

  function choose(hex: string) {
    const normalized = hex.toLowerCase();
    if (!/^#[0-9a-f]{6}$/.test(normalized)) return;
    onchange(normalized);
    if (!PRESETS.includes(normalized)) {
      recent = [normalized, ...recent.filter((item) => item !== normalized)].slice(0, 6);
      try {
        window.localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
      } catch {
        // Best effort.
      }
    }
  }

  function toggle() {
    open = !open;
    draft = value;
  }

  function commitDraft() {
    let hex = draft.trim();
    if (!hex.startsWith("#")) hex = `#${hex}`;
    if (/^#[0-9a-f]{3}$/i.test(hex)) hex = `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`;
    choose(hex);
  }

  $effect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (root && !root.contains(event.target as Node)) open = false;
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("keydown", escape);
    };
  });
</script>

<div class="relative" bind:this={root}>
  <span class="mb-1 block text-[12px] text-slate-400">{label}</span>
  <button
    type="button"
    class="flex h-8 w-full items-center gap-2 rounded-md border border-slate-700 bg-slate-900 px-1.5 text-left hover:border-slate-600"
    aria-haspopup="dialog"
    aria-expanded={open}
    onclick={toggle}
  >
    <span class="h-5 w-5 shrink-0 rounded border border-black/20" style={`background:${value}`}></span>
    <span class="truncate font-mono text-[12px] uppercase text-slate-300">{value}</span>
  </button>

  {#if open}
    <div class="popover absolute left-0 top-full z-30 mt-1 w-56 rounded-md border border-slate-700 bg-slate-900 p-2.5 shadow-xl" role="dialog" aria-label={label}>
      <div class="grid grid-cols-6 gap-1.5">
        {#each PRESETS as color}
          <button
            type="button"
            class={`h-7 rounded border ${color === value ? "border-cyan-400 ring-2 ring-cyan-500/40" : "border-black/20"}`}
            style={`background:${color}`}
            aria-label={color}
            title={color}
            onclick={() => choose(color)}
          ></button>
        {/each}
      </div>
      {#if recent.length > 0}
        <p class="mt-2.5 text-[11px] text-slate-500">{t("Recientes")}</p>
        <div class="mt-1 grid grid-cols-6 gap-1.5">
          {#each recent as color}
            <button
              type="button"
              class={`h-7 rounded border ${color === value ? "border-cyan-400 ring-2 ring-cyan-500/40" : "border-black/20"}`}
              style={`background:${color}`}
              aria-label={color}
              title={color}
              onclick={() => choose(color)}
            ></button>
          {/each}
        </div>
      {/if}
      <div class="mt-2.5 flex items-center gap-1.5">
        <input
          class="h-7 min-w-0 flex-1 rounded border border-slate-700 bg-slate-950 px-2 font-mono text-[12px] uppercase text-slate-100 outline-none focus:border-cyan-500"
          bind:value={draft}
          aria-label={t("Código hexadecimal")}
          onkeydown={(event) => event.key === "Enter" && commitDraft()}
          onblur={commitDraft}
        />
        <label class="relative flex h-7 cursor-pointer items-center rounded border border-slate-700 px-2 text-[12px] text-slate-300 hover:border-slate-500">
          {t("Más…")}
          <input
            type="color"
            class="absolute inset-0 cursor-pointer opacity-0"
            {value}
            oninput={(event) => onchange(event.currentTarget.value)}
            onchange={(event) => choose(event.currentTarget.value)}
          />
        </label>
      </div>
    </div>
  {/if}
</div>

<style>
  .popover {
    transform-origin: top left;
    animation: pop 150ms cubic-bezier(0.23, 1, 0.32, 1);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.97) translateY(-2px);
    }
  }
</style>
