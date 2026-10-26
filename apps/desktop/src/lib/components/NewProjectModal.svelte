<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import type {
    AspectRatio,
    AuthoringMode,
    AvchdResolution,
    FrameRate,
    TvStandard,
  } from "../api/bindings";
  import { messageOf } from "../api/client";
  import { projectStore } from "../stores/project.svelte";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  interface FpsOption {
    key: string;
    label: string;
    rate: FrameRate;
  }

  const FPS_23_976: FrameRate = { num: 24_000, den: 1_001 };
  const FPS_24: FrameRate = { num: 24, den: 1 };
  const FPS_25: FrameRate = { num: 25, den: 1 };
  const FPS_29_97: FrameRate = { num: 30_000, den: 1_001 };
  const FPS_50: FrameRate = { num: 50, den: 1 };
  const FPS_59_94: FrameRate = { num: 60_000, den: 1_001 };

  function fpsOptionsFor(resolution: AvchdResolution): FpsOption[] {
    if (resolution === "1080p") {
      return [
        { key: "23.976", label: "23.976 fps", rate: FPS_23_976 },
        { key: "24", label: "24 fps", rate: FPS_24 },
      ];
    }
    if (resolution === "1080i") {
      return [
        { key: "29.97", label: "29.97 fps", rate: FPS_29_97 },
        { key: "25", label: "25 fps", rate: FPS_25 },
      ];
    }
    if (resolution === "720p") {
      return [
        { key: "23.976", label: "23.976 fps", rate: FPS_23_976 },
        { key: "50", label: "50 fps", rate: FPS_50 },
        { key: "59.94", label: "59.94 fps", rate: FPS_59_94 },
      ];
    }
    if (resolution === "480i") {
      return [{ key: "29.97", label: "29.97 fps", rate: FPS_29_97 }];
    }
    return [{ key: "25", label: "25 fps", rate: FPS_25 }];
  }

  let name = $state(t("Mi DVD"));
  let mode = $state<AuthoringMode>("dvdVideo");
  let standard = $state<TvStandard>("ntsc");
  let aspect = $state<AspectRatio>("fourThree");
  let resolution = $state<AvchdResolution>("720p");
  let fpsKey = $state("23.976");
  let busy = $state(false);
  let localError = $state<string | null>(null);

  const fpsOptions = $derived(fpsOptionsFor(resolution));
  const selectedFps = $derived(
    fpsOptions.find((option) => option.key === fpsKey)?.rate ?? FPS_23_976,
  );
  const avchdStandard = $derived<TvStandard>(
    resolution === "480i"
      ? "ntsc"
      : resolution === "576i" || fpsKey === "25" || fpsKey === "50"
        ? "pal"
        : "ntsc",
  );
  const avchdAspect = $derived<AspectRatio>(
    resolution === "480i" || resolution === "576i" ? aspect : "sixteenNine",
  );

  $effect(() => {
    if (!fpsOptions.some((option) => option.key === fpsKey)) {
      fpsKey = fpsOptions[0]?.key ?? "23.976";
    }
  });

  async function createProject() {
    busy = true;
    localError = null;
    try {
      await projectStore.newProject(
        name.trim() || t("Mi DVD"),
        mode === "avchd" ? avchdStandard : standard,
        mode === "avchd" ? avchdAspect : aspect,
        mode,
        mode === "avchd" ? resolution : null,
        mode === "avchd" ? selectedFps : null,
      );
      onClose();
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }

  async function openProject() {
    const picked = await open({
      multiple: false,
      filters: [{ name: t("Proyecto de DVD Forge"), extensions: ["json"] }],
    });
    if (typeof picked !== "string") return;
    busy = true;
    localError = null;
    try {
      await projectStore.openProject(picked);
      onClose();
    } catch (cause) {
      localError = messageOf(cause);
    } finally {
      busy = false;
    }
  }
</script>

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar")}
    onclick={onClose}
    disabled={busy}
    class="absolute inset-0 cursor-default bg-slate-950/80 backdrop-blur-sm"
  ></button>

  <div class="relative z-10 flex max-h-[92vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950 shadow-2xl shadow-black/60">
    <header class="flex shrink-0 items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
      <div>
        <p class="text-[10px] font-semibold uppercase tracking-[0.22em] text-cyan-400">{t("DVD Forge")}</p>
        <h2 class="mt-1 text-lg font-semibold text-white">{t("Nuevo proyecto")}</h2>
        <p class="mt-0.5 text-[11px] text-slate-500">
          {t("DVD-Video universal o AVCHD/Blu-ray HD con menús HDMV interactivos.")}
        </p>
      </div>
      <button
        type="button"
        onclick={onClose}
        disabled={busy}
        class="rounded-lg border border-slate-800 px-2 py-1 text-xs text-slate-400 transition hover:border-slate-700 hover:text-slate-200 disabled:opacity-40"
      >
        ✕
      </button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if localError}
        <div class="mb-3 rounded-xl border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-[11px] text-rose-200">
          {localError}
        </div>
      {/if}

      <div class="grid gap-2 sm:grid-cols-2">
        <button
          type="button"
          onclick={() => (mode = "dvdVideo")}
          disabled={busy}
          class={`rounded-xl border p-3 text-left transition ${
            mode === "dvdVideo"
              ? "border-cyan-500/60 bg-cyan-500/10"
              : "border-slate-800 bg-slate-900/40 hover:border-slate-700"
          }`}
        >
          <span class="text-sm font-semibold text-white">{t("DVD-Video universal")}</span>
          <span class="mt-0.5 block text-[10px] leading-4 text-slate-500">
            {t("Menús Konva, botones, capítulos y máxima compatibilidad.")}
          </span>
        </button>
        <button
          type="button"
          onclick={() => (mode = "avchd")}
          disabled={busy}
          class={`rounded-xl border p-3 text-left transition ${
            mode === "avchd"
              ? "border-cyan-500/60 bg-cyan-500/10"
              : "border-slate-800 bg-slate-900/40 hover:border-slate-700"
          }`}
        >
          <span class="text-sm font-semibold text-white">{t("AVCHD alta definición")}</span>
          <span class="mt-0.5 block text-[10px] leading-4 text-slate-500">
            {t("HD/SD continuo con capítulos para PS4, PS5 y Blu-ray.")}
          </span>
        </button>
      </div>

      <div class="mt-4 space-y-3">
        <label class="block">
          <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Nombre")}</span>
          <input
            class="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-sm text-white outline-none focus:border-cyan-500/60"
            bind:value={name}
            placeholder={t("Mi DVD")}
            disabled={busy}
          />
        </label>

        {#if mode === "dvdVideo"}
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="block">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Estándar")}</span>
              <select
                class="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-[11px] text-white outline-none focus:border-cyan-500/60"
                bind:value={standard}
                disabled={busy}
              >
                <option value="ntsc">{t("NTSC · 720×480")}</option>
                <option value="pal">{t("PAL · 720×576")}</option>
              </select>
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Aspecto")}</span>
              <select
                class="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-[11px] text-white outline-none focus:border-cyan-500/60"
                bind:value={aspect}
                disabled={busy}
              >
                <option value="fourThree">4:3</option>
                <option value="sixteenNine">16:9</option>
              </select>
            </label>
          </div>
        {:else}
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="block">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Resolución")}</span>
              <select
                class="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-[11px] text-white outline-none focus:border-cyan-500/60"
                bind:value={resolution}
                disabled={busy}
              >
                <option value="1080p">{t("1080p · 1920×1080")}</option>
                <option value="1080i">{t("1080i · 1920×1080 TFF")}</option>
                <option value="720p">{t("720p · 1280×720")}</option>
                <option value="480i">{t("480i · 720×480")}</option>
                <option value="576i">{t("576i · 720×576")}</option>
              </select>
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Velocidad")}</span>
              <select
                class="w-full rounded-xl border border-slate-800 bg-slate-900 px-3 py-2 text-[11px] text-white outline-none focus:border-cyan-500/60"
                bind:value={fpsKey}
                disabled={busy}
              >
                {#each fpsOptions as option}
                  <option value={option.key}>{option.label}</option>
                {/each}
              </select>
            </label>
          </div>
          <p class="rounded-lg border border-cyan-500/20 bg-cyan-500/10 px-3 py-2 text-[10px] leading-4 text-cyan-100/80">
            {t("AVCHD/Blu-ray usa H.264, AC-3 y menús HDMV Interactive Graphics. Puedes elegir BD-25 o BD-50 en la cabecera para habilitar el authoring Blu-ray nativo.")}
          </p>
        {/if}
      </div>
    </div>

    <footer class="flex shrink-0 flex-wrap items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
      <button
        type="button"
        onclick={openProject}
        disabled={busy}
        class="rounded-lg border border-slate-800 px-3 py-1.5 text-[11px] text-slate-300 transition hover:border-cyan-500/60 disabled:opacity-40"
      >
        {t("Abrir proyecto…")}
      </button>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={onClose}
          disabled={busy}
          class="rounded-lg border border-slate-800 px-3 py-1.5 text-[11px] text-slate-300 transition hover:border-slate-700 disabled:opacity-40"
        >
          {t("Cancelar")}
        </button>
        <button
          type="button"
          onclick={createProject}
          disabled={busy}
          class="rounded-lg bg-cyan-500 px-4 py-1.5 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400 disabled:cursor-wait disabled:opacity-60"
        >
          {busy ? t("Creando…") : t("Crear proyecto")}
        </button>
      </div>
    </footer>
  </div>
</div>
