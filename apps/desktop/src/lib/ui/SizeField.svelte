<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Icon from "./Icon.svelte";

  let {
    value,
    min = 8,
    max = 96,
    label,
    onchange,
  }: { value: number; min?: number; max?: number; label: string; onchange: (value: number) => void } = $props();

  function set(next: number) {
    const clamped = Math.max(min, Math.min(max, Math.round(next)));
    if (Number.isFinite(clamped) && clamped !== value) onchange(clamped);
  }

  const step = "flex h-8 w-8 shrink-0 items-center justify-center text-slate-300 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-40";
</script>

<div>
  <span class="mb-1 block text-[12px] text-slate-400">{label}</span>
  <div class="flex h-8 items-stretch overflow-hidden rounded-md border border-slate-700 bg-slate-900">
    <button type="button" class={step} onclick={() => set(value - 2)} disabled={value <= min} aria-label={t("Más pequeño")}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><path d="M5 12h14" /></svg>
    </button>
    <input
      type="number"
      {min}
      {max}
      class="w-full min-w-0 border-x border-slate-700 bg-transparent text-center text-[13px] tabular-nums text-slate-100 outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none"
      {value}
      aria-label={label}
      onchange={(event) => set(Number(event.currentTarget.value))}
    />
    <button type="button" class={step} onclick={() => set(value + 2)} disabled={value >= max} aria-label={t("Más grande")}>
      <Icon name="add" size={14} />
    </button>
  </div>
  <input
    type="range"
    {min}
    {max}
    {value}
    class="mt-1.5 w-full accent-cyan-500"
    aria-label={label}
    oninput={(event) => set(Number(event.currentTarget.value))}
  />
</div>
