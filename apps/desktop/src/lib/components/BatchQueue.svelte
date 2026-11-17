<script lang="ts">
  import { Channel } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { commands, type JobEvent_Deserialize } from "../api/bindings";
  import { messageOf, unwrap } from "../api/client";
  import { localizeError } from "../i18n/diagnostics";
  import { t } from "../i18n/index.svelte";
  import { compositeProjectMenus } from "../production";
  import { projectStore } from "../stores/project.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  type Status = "pending" | "running" | "done" | "failed" | "cancelled";
  interface Item {
    path: string;
    name: string;
    status: Status;
    iso?: string;
    error?: string;
  }

  const STORAGE_KEY = "kinodisc_batch_queue";

  function load(): Item[] {
    try {
      const parsed = JSON.parse(window.localStorage.getItem(STORAGE_KEY) ?? "[]") as Item[];
      // A queue interrupted by closing the app resumes from its pending items.
      return parsed.map((item) => (item.status === "running" ? { ...item, status: "pending" } : item));
    } catch {
      return [];
    }
  }

  let items = $state<Item[]>(load());
  let running = $state(false);
  let stopRequested = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(items));
    } catch {
      // Best effort.
    }
  });

  const basename = (path: string) => path.replace(/^.*[\\/]/, "");
  const folder = (path: string) => path.replace(/[\\/][^\\/]*$/, "");
  const pending = $derived(items.filter((item) => item.status === "pending").length);

  function add(path: string) {
    if (items.some((item) => item.path === path)) return;
    items = [...items, { path, name: basename(path).replace(/\.json$/i, ""), status: "pending" }];
  }

  function addCurrent() {
    if (projectStore.projectPath) add(projectStore.projectPath);
    else error = t("Guarda el proyecto antes de añadirlo a la cola.");
  }

  async function addFiles() {
    const picked = await open({
      multiple: true,
      filters: [{ name: t("Proyecto de Kinodisc"), extensions: ["json"] }],
    });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    for (const path of paths) add(path);
  }

  function move(index: number, delta: number) {
    const target = index + delta;
    if (target < 0 || target >= items.length) return;
    const next = [...items];
    [next[index], next[target]] = [next[target], next[index]];
    items = next;
  }

  function remove(index: number) {
    items = items.filter((_, position) => position !== index);
  }

  function retry(index: number) {
    items[index] = { ...items[index], status: "pending", error: undefined };
  }

  function clearFinished() {
    items = items.filter((item) => item.status === "pending" || item.status === "running");
  }

  async function runQueue() {
    running = true;
    stopRequested = false;
    error = null;
    try {
      for (let index = 0; index < items.length; index += 1) {
        if (stopRequested) break;
        if (items[index].status !== "pending") continue;
        items[index] = { ...items[index], status: "running" };
        try {
          await projectStore.openProject(items[index].path);
          await projectStore.hydrateProjectPreviews();
          const project = projectStore.project;
          if (!project) throw new Error(t("No hay un proyecto abierto."));
          projectStore.resetRender();
          projectStore.rendering = true;
          await compositeProjectMenus(project, {
            backgroundColor: projectStore.menuBackgroundColor,
            previewUrls: projectStore.previewUrls,
            framePaths: projectStore.framePaths,
          });
          const channel = new Channel<JobEvent_Deserialize>();
          channel.onmessage = (event) => projectStore.applyJobEvent(event);
          const output = `${folder(items[index].path)}/kinodisc-${items[index].name}`;
          const outcome = await unwrap(commands.renderStart(project, output, "auto", channel));
          items[index] = { ...items[index], status: "done", iso: outcome.isoPath };
        } catch (cause) {
          items[index] = {
            ...items[index],
            status: stopRequested ? "cancelled" : "failed",
            error: localizeError(messageOf(cause)),
          };
        } finally {
          projectStore.rendering = false;
        }
      }
    } finally {
      running = false;
    }
  }

  async function stop() {
    stopRequested = true;
    try {
      await unwrap(commands.renderCancel());
    } catch {
      // Nothing running.
    }
  }

  function statusLabel(status: Status): string {
    switch (status) {
      case "pending":
        return t("En espera");
      case "running":
        return t("Generando…");
      case "done":
        return t("Listo");
      case "failed":
        return t("Error");
      default:
        return t("Cancelado");
    }
  }

  const statusClass: Record<Status, string> = {
    pending: "text-slate-400",
    running: "text-cyan-300",
    done: "text-emerald-300",
    failed: "text-rose-300",
    cancelled: "text-amber-300",
  };
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape" && !running) onClose(); }} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar")}
    onclick={() => !running && onClose()}
    class="absolute inset-0 cursor-default bg-slate-950/80"
  ></button>
  <div
    class="relative z-10 flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-lg border border-slate-700/80 bg-slate-950 shadow-xl"
    role="dialog"
    aria-modal="true"
    aria-labelledby="batch-title"
  >
    <header class="flex items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
      <div>
        <h2 id="batch-title" class="text-lg font-semibold text-white">{t("Cola de discos")}</h2>
        <p class="mt-0.5 text-[12px] text-slate-500">
          {t("Genera varios discos seguidos, uno detrás de otro. Si se corta, lo que quedó en espera continúa después.")}
        </p>
      </div>
      <button
        type="button"
        onclick={onClose}
        disabled={running}
        class="rounded-lg border border-slate-800 px-2 py-1 text-xs text-slate-400 transition hover:text-slate-200 disabled:opacity-40"
        aria-label={t("Cerrar")}
      >
        ✕
      </button>
    </header>

    <div class="flex flex-wrap gap-2 border-b border-slate-800 px-5 py-3">
      <button
        type="button"
        onclick={addCurrent}
        disabled={running || !projectStore.project}
        class="rounded-lg border border-slate-700 px-3 py-1.5 text-[12px] text-slate-200 transition hover:border-cyan-500/60 disabled:opacity-40"
      >
        {t("+ Proyecto actual")}
      </button>
      <button
        type="button"
        onclick={addFiles}
        disabled={running}
        class="rounded-lg border border-slate-700 px-3 py-1.5 text-[12px] text-slate-200 transition hover:border-cyan-500/60 disabled:opacity-40"
      >
        {t("+ Añadir proyectos…")}
      </button>
      <button
        type="button"
        onclick={clearFinished}
        disabled={running}
        class="rounded-lg px-3 py-1.5 text-[12px] text-slate-400 transition hover:text-slate-200 disabled:opacity-40"
      >
        {t("Quitar terminados")}
      </button>
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-3">
      {#if items.length === 0}
        <p class="py-8 text-center text-[12px] text-slate-500">
          {t("La cola está vacía. Añade proyectos guardados para generarlos uno detrás de otro.")}
        </p>
      {:else}
        <ol class="space-y-1.5">
          {#each items as item, index (item.path)}
            <li class="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/50 px-3 py-2">
              <span class="w-5 text-right font-mono text-[11px] text-slate-500">{index + 1}</span>
              <div class="min-w-0 flex-1">
                <p class="truncate text-[12px] font-medium text-slate-200" title={item.path}>{item.name}</p>
                <p class={`truncate text-[11px] ${statusClass[item.status]}`} title={item.error ?? item.iso ?? ""}>
                  {statusLabel(item.status)}{item.iso ? ` · ${item.iso}` : ""}{item.error ? ` · ${item.error}` : ""}
                </p>
              </div>
              {#if !running}
                {#if item.status === "failed" || item.status === "cancelled"}
                  <button type="button" onclick={() => retry(index)} class="rounded px-1.5 text-[12px] text-cyan-300 hover:bg-slate-800" title={t("Reintentar")} aria-label={t("Reintentar")}>↻</button>
                {/if}
                <button type="button" onclick={() => move(index, -1)} disabled={index === 0} class="rounded px-1.5 text-[12px] text-slate-400 hover:bg-slate-800 disabled:opacity-25" aria-label={t("Subir")}>↑</button>
                <button type="button" onclick={() => move(index, 1)} disabled={index === items.length - 1} class="rounded px-1.5 text-[12px] text-slate-400 hover:bg-slate-800 disabled:opacity-25" aria-label={t("Bajar")}>↓</button>
                <button type="button" onclick={() => remove(index)} class="rounded px-1.5 text-[12px] text-slate-400 hover:bg-rose-500/10 hover:text-rose-300" aria-label={t("Quitar de la cola")}>✕</button>
              {/if}
            </li>
          {/each}
        </ol>
      {/if}
      {#if error}
        <p class="mt-3 text-[12px] text-rose-300">{error}</p>
      {/if}
      {#if running}
        <p class="mt-3 text-[12px] text-cyan-200">
          {projectStore.renderStage} · {projectStore.renderPercent.toFixed(0)}%
        </p>
      {/if}
    </div>

    <footer class="flex items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <span class="text-[12px] text-slate-500">{t("{count} en espera", { count: pending })}</span>
      {#if running}
        <button
          type="button"
          onclick={stop}
          class="rounded-lg border border-rose-500/50 px-3 py-1.5 text-[12px] font-semibold text-rose-200 transition hover:bg-rose-500/10"
        >
          {t("Detener")}
        </button>
      {:else}
        <button
          type="button"
          onclick={runQueue}
          disabled={pending === 0}
          class="rounded-lg bg-cyan-500 px-4 py-1.5 text-[12px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
        >
          {t("Generar todo")}
        </button>
      {/if}
    </footer>
  </div>
</div>
