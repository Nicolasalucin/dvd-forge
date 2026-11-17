<script lang="ts">
  import { icons, type IconName } from "./icons";

  let {
    name,
    size = 16,
    class: className = "",
    label = null,
  }: { name: IconName; size?: number; class?: string; label?: string | null } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  class={`shrink-0 ${className}`}
  role={label ? "img" : undefined}
  aria-label={label ?? undefined}
  aria-hidden={label ? undefined : "true"}
>
  <path d={icons[name]} />
</svg>
