<script lang="ts">
  import type { Component } from "svelte";
  import { t } from "../i18n/index.svelte";

  // Loads a screen on first use so it is not part of the startup bundle.
  interface Props {
    loader: () => Promise<{ default: Component<any> }>;
    props?: Record<string, unknown>;
  }

  let { loader, props = {} }: Props = $props();
</script>

{#await loader()}
  <div class="flex flex-1 items-center justify-center text-[11px] text-slate-500" role="status">
    {t("Cargando…")}
  </div>
{:then module}
  <module.default {...props} />
{:catch error}
  <div class="flex flex-1 items-center justify-center text-[11px] text-rose-300" role="alert">
    {String(error)}
  </div>
{/await}
