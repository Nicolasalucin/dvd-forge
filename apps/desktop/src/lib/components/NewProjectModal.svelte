<script lang="ts">
  import Icon from "../ui/Icon.svelte";
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
        { key: "23.976", label: "23.976 fps (cine)", rate: FPS_23_976 },
        { key: "24", label: "24 fps (cine)", rate: FPS_24 },
        { key: "25", label: "25 fps (TV PAL · PsF)", rate: FPS_25 },
        { key: "29.97", label: "29.97 fps (TV NTSC · PsF)", rate: FPS_29_97 },
        { key: "50", label: "50 fps (AVCHD 2.0)", rate: FPS_50 },
        { key: "59.94", label: "59.94 fps (AVCHD 2.0)", rate: FPS_59_94 },
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
  // PAL or NTSC from the system region (es-AR, es-ES, pt-PT... use PAL).
  const PAL_REGIONS = new Set([
    "AR", "UY", "PY", "ES", "PT", "GB", "IE", "FR", "DE", "IT", "NL", "BE", "LU", "AT", "CH",
    "DK", "SE", "NO", "FI", "IS", "PL", "CZ", "SK", "HU", "RO", "BG", "GR", "HR", "SI", "RS",
    "TR", "RU", "UA", "AU", "NZ", "ZA", "IN", "CN", "HK", "SG", "MY", "ID", "IL", "EG", "SA", "AE",
  ]);
  function defaultStandard(): TvStandard {
    const tags = [...(navigator.languages ?? []), navigator.language].filter(Boolean);
    for (const tag of tags) {
      const region = tag.split(/[-_]/)[1]?.toUpperCase();
      if (region) return PAL_REGIONS.has(region) ? "pal" : "ntsc";
    }
    return "ntsc";
  }

  let standard = $state<TvStandard>(defaultStandard());
  let aspect = $state<AspectRatio>("sixteenNine");
  let resolution = $state<AvchdResolution>("720p");
  let fpsKey = $state("23.976");
  let busy = $state(false);
  let localError = $state<string | null>(null);

  const fpsOptions = $derived(fpsOptionsFor(resolution));
  // The selected fps is derived from the available combinations. Keeping the
  // stale `fpsKey` in a state effect forced an extra render on every
  // resolution change.
  const effectiveFpsKey = $derived(
    fpsOptions.some((option) => option.key === fpsKey)
      ? fpsKey
      : (fpsOptions[0]?.key ?? "23.976"),
  );
  const selectedFps = $derived(
    fpsOptions.find((option) => option.key === effectiveFpsKey)?.rate ?? FPS_23_976,
  );
  const avchdStandard = $derived<TvStandard>(
    resolution === "480i"
      ? "ntsc"
      : resolution === "576i" ||
          effectiveFpsKey === "25" ||
          effectiveFpsKey === "50"
        ? "pal"
        : "ntsc",
  );
  const avchdAspect = $derived<AspectRatio>(
    resolution === "480i" || resolution === "576i" ? aspect : "sixteenNine",
  );

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
      filters: [{ name: t("Proyecto de Kinodisc"), extensions: ["json"] }],
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

<svelte:window onkeydown={(event) => { if (event.key === "Escape" && !busy) onClose(); }} />

<div class="fixed inset-0 z-50 flex items-center justify-center p-4">
  <button
    type="button"
    aria-label={t("Cerrar")}
    onclick={onClose}
    disabled={busy}
    class="absolute inset-0 cursor-default bg-slate-950/80"
  ></button>

  <div class="ui-dialog max-h-[92vh] w-full max-w-2xl">
    <header class="ui-dialog-header">
      <div>
        <h2 class="text-[16px] font-semibold text-slate-50">{t("Nuevo proyecto")}</h2>
        <p class="ui-hint mt-0.5">
          {t("Elige el tipo de disco. Si no estás seguro, DVD-Video funciona en casi cualquier reproductor.")}
        </p>
      </div>
      <button
        type="button"
        onclick={onClose}
        disabled={busy}
        class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon"
        aria-label={t("Cerrar")}
      ><Icon name="close" size={15} /></button>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if localError}
        <div class="mb-3 rounded-md bg-rose-500/10 px-3 py-2 text-[12px] text-rose-200" role="alert">
          {localError}
        </div>
      {/if}

      <div class="grid gap-2 sm:grid-cols-2">
        <button
          type="button"
          onclick={() => (mode = "dvdVideo")}
          disabled={busy}
          aria-pressed={mode === "dvdVideo"}
          class={`flex gap-3 rounded-md border p-3 text-left ${
            mode === "dvdVideo" ? "border-cyan-500 bg-cyan-500/10" : "border-slate-700 hover:border-slate-500"
          }`}
        >
          <Icon name="disc" size={22} class={mode === "dvdVideo" ? "text-cyan-400" : "text-slate-400"} />
          <span class="min-w-0">
          <span class="block text-[14px] font-semibold text-slate-50">{t("DVD-Video universal")}</span>
          <span class="mt-0.5 block text-[12px] leading-5 text-slate-400">
            {t("Funciona en casi todos los reproductores, consolas y ordenadores. Calidad estándar.")}
          </span>
          </span>
        </button>
        <button
          type="button"
          onclick={() => (mode = "avchd")}
          disabled={busy}
          aria-pressed={mode === "avchd"}
          class={`flex gap-3 rounded-md border p-3 text-left ${
            mode === "avchd" ? "border-cyan-500 bg-cyan-500/10" : "border-slate-700 hover:border-slate-500"
          }`}
        >
          <Icon name="film" size={22} class={mode === "avchd" ? "text-cyan-400" : "text-slate-400"} />
          <span class="min-w-0">
          <span class="block text-[14px] font-semibold text-slate-50">{t("Blu-ray / AVCHD (alta definición)")}</span>
          <span class="mt-0.5 block text-[12px] leading-5 text-slate-400">
            {t("Imagen HD con menús. Necesita reproductor Blu-ray, PS3/PS4/PS5 o similar.")}
          </span>
          </span>
        </button>
      </div>

      <div class="mt-4 space-y-3">
        <label class="ui-label">
          <span class="mb-1 block">{t("Nombre")}</span>
          <input
            class="ui-input w-full"
            bind:value={name}
            placeholder={t("Mi DVD")}
            disabled={busy}
          />
        </label>

        {#if mode === "dvdVideo"}
          <div class="grid gap-3 sm:grid-cols-2">
            <label class="ui-label">
              <span class="mb-1 block">{t("Estándar")}</span>
              <select
                class="ui-input w-full"
                bind:value={standard}
                disabled={busy}
              >
                <option value="ntsc">{t("NTSC · 720×480 (América, Japón)")}</option>
                <option value="pal">{t("PAL · 720×576 (Europa, Argentina, Australia…)")}</option>
              </select>
            </label>
            <label class="ui-label">
              <span class="mb-1 block">{t("Aspecto")}</span>
              <select
                class="ui-input w-full"
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
            <label class="ui-label">
              <span class="mb-1 block">{t("Resolución")}</span>
              <select
                class="ui-input w-full"
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
            <label class="ui-label">
              <span class="mb-1 block">{t("Velocidad")}</span>
              <select
                class="ui-input w-full"
                value={effectiveFpsKey}
                onchange={(event) => (fpsKey = event.currentTarget.value)}
                disabled={busy}
              >
                {#each fpsOptions as option}
                  <option value={option.key}>{t(option.label)}</option>
                {/each}
              </select>
            </label>
          </div>
          {#if resolution === "1080p" && (effectiveFpsKey === "25" || effectiveFpsKey === "29.97")}
            <p class="flex gap-2 text-[12px] leading-5 text-slate-400"><Icon name="check" size={15} class="mt-0.5 text-emerald-400" />
              {t("Ideal para TV y video casero. Se graba como PsF (imagen progresiva en señal 1080i), compatible con todos los reproductores Blu-ray.")}
            </p>
          {:else if resolution === "1080p" && (effectiveFpsKey === "50" || effectiveFpsKey === "59.94")}
            <p class="flex gap-2 text-[12px] leading-5 text-slate-400"><Icon name="warning" size={15} class="mt-0.5 text-amber-400" />
              {t("Máxima fluidez (deportes, videojuegos, cámaras modernas). Es AVCHD 2.0: funciona en PS3/PS4/PS5 y reproductores actuales, pero algunos Blu-ray antiguos no lo reproducen.")}
            </p>
          {/if}
          <p class="flex gap-2 text-[12px] leading-5 text-slate-400">
            <Icon name="info" size={15} class="mt-0.5 text-cyan-400" />
            {t("Para un Blu-ray elige BD-25 o BD-50 en la barra del disco después de crear el proyecto.")}
          </p>
        {/if}
      </div>
    </div>

    <footer class="ui-dialog-footer justify-between">
      <button
        type="button"
        onclick={openProject}
        disabled={busy}
        class="ui-btn"
      >
        {t("Abrir proyecto…")}
      </button>
      <div class="flex items-center gap-2">
        <button
          type="button"
          onclick={onClose}
          disabled={busy}
          class="ui-btn"
        >
          {t("Cancelar")}
        </button>
        <button
          type="button"
          onclick={createProject}
          disabled={busy}
          class="ui-btn ui-btn-primary"
        >
          {busy ? t("Creando…") : t("Crear proyecto")}
        </button>
      </div>
    </footer>
  </div>
</div>
