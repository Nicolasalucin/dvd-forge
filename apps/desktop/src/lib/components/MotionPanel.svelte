<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { ButtonShapeMask, MenuButton_Serialize } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { formatDuration } from "../utils/format";

  const project = $derived(projectStore.project);
  const menu = $derived(projectStore.activeMenu);
  const motion = $derived(menu?.motionConfig ?? null);
  const selectedButton = $derived(projectStore.selectedButton);
  const previewUrl = $derived(
    selectedButton?.thumbnail
      ? projectStore.previewForAsset(selectedButton.thumbnail.asset)
      : projectStore.backgroundPreviewUrl,
  );
  const backgroundUrl = $derived(projectStore.backgroundPreviewUrl);
  const duration = $derived(motion?.durationMs ?? 15_000);
  const buttonMotion = $derived(selectedButton?.motionPreview ?? null);
  const transition = $derived(menu?.transition ?? null);
  const videoAssets = $derived(
    (project?.assets ?? []).filter((asset) => asset.kind === "video"),
  );
  const shapeMasks: Array<{ id: ButtonShapeMask; label: string }> = [
    { id: "rectangle", label: "Rectángulo" },
    { id: "roundedRect", label: "Bordes redondeados" },
    { id: "circle", label: "Círculo" },
    { id: "star", label: "Estrella" },
  ];

  function maskLabel(mask: ButtonShapeMask | null | undefined): string {
    const label = shapeMasks.find((shape) => shape.id === mask)?.label;
    return label ? t(label) : t("Sin máscara");
  }

  function enableMotion() {
    projectStore.updateMenuMotion({});
  }

  function setNumber(
    key: "durationMs" | "fadeInMs" | "fadeOutMs" | "audioVolume",
    value: number,
  ) {
    if (!Number.isFinite(value)) return;
    projectStore.updateMenuMotion({ [key]: Math.max(0, value) });
  }

  function setShape(button: MenuButton_Serialize, shapeMask: ButtonShapeMask) {
    projectStore.updateButton(button.id, { shapeMask });
  }

  function enableButtonMotion(button: MenuButton_Serialize, enabled: boolean) {
    projectStore.updateButtonMotion(
      button.id,
      enabled
        ? {
            sourceStartMs:
              button.motionPreview?.sourceStartMs ??
              (button.thumbnail && button.thumbnail.fps.num > 0
                ? Math.round(
                    (button.thumbnail.inFrame * 1000 * button.thumbnail.fps.den) /
                      button.thumbnail.fps.num,
                  )
                : 0),
            loopDurationMs: button.motionPreview?.loopDurationMs ?? 10_000,
          }
        : null,
    );
  }

  function setButtonMotion(
    button: MenuButton_Serialize,
    key: "sourceStartMs" | "loopDurationMs",
    value: number,
  ) {
    if (!Number.isFinite(value)) return;
    projectStore.updateButtonMotion(button.id, {
      [key]: Math.max(0, Math.round(value)),
    });
  }

  function fadePath(kind: "in" | "out", height: number): string {
    const width = 600;
    const fade = Math.min(width / 2, Math.max(6, (motion?.fadeInMs ?? 0) / duration * width));
    if (kind === "in") {
      return `M 0 ${height} L ${fade} 6 L ${width} 6`;
    }
    const fadeOut = Math.min(width / 2, Math.max(6, (motion?.fadeOutMs ?? 0) / duration * width));
    return `M 0 6 L ${width - fadeOut} 6 L ${width} ${height}`;
  }
</script>

<section class="flex min-h-0 flex-1 flex-col bg-[#080d18]">
  <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800/70 px-3 py-2">
    <div class="flex items-center gap-2">
      <span class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Menú animado")}</span>
      {#if menu}
        <span class="rounded-full border border-slate-800 px-2 py-0.5 text-[10px] text-slate-300">
          {menu.name}
        </span>
      {/if}
      {#if motion}
        <span class="rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-[10px] text-cyan-200">
          {formatDuration(duration)} · {t(motion.loopEnabled ? "en bucle" : "una vez")}
        </span>
      {/if}
    </div>
    {#if menu}
      <div class="flex items-center gap-2">
        {#if motion}
          <button
            type="button"
            onclick={() => projectStore.disableMenuMotion()}
            class="rounded-md border border-rose-500/30 px-2 py-1 text-[10px] text-rose-300 transition hover:bg-rose-500/10"
          >
            {t("Desactivar animación")}
          </button>
        {:else}
          <button
            type="button"
            onclick={enableMotion}
            class="rounded-md bg-cyan-500 px-2.5 py-1 text-[10px] font-bold text-slate-950 transition hover:bg-cyan-400"
          >
            {t("Activar menú animado")}
          </button>
        {/if}
      </div>
    {/if}
  </div>

  {#if !menu}
    <div class="flex flex-1 items-center justify-center p-6 text-center">
      <div class="max-w-md rounded-2xl border border-dashed border-slate-800 p-8">
        <p class="text-sm font-medium text-slate-300">{t("Sin menú activo")}</p>
        <p class="mt-1 text-xs leading-5 text-slate-500">
          {t("Crea un menú para configurar su bucle, fundidos y botones de vídeo en DVD o HD.")}
        </p>
      </div>
    </div>
  {:else}
    <div class="grid min-h-0 flex-1 gap-3 overflow-y-auto p-3 xl:grid-cols-[minmax(0,1fr)_300px]">
      <div class="flex min-w-0 flex-col gap-3">
        <div class="relative flex min-h-[230px] flex-1 items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-black">
          {#if backgroundUrl}
            <img src={backgroundUrl} alt="" class="absolute inset-0 h-full w-full object-cover opacity-60" />
          {/if}
          <div class="absolute inset-0 bg-gradient-to-t from-slate-950/80 via-transparent to-slate-950/30"></div>
          {#if previewUrl}
            <div class="relative z-10 flex max-w-[72%] flex-col items-center gap-2">
              <div class="relative overflow-hidden rounded-2xl border-2 border-cyan-400/60 shadow-2xl shadow-cyan-500/20">
                <img src={previewUrl} alt="" class="max-h-[150px] w-auto object-contain" />
                {#if buttonMotion}
                  <div class="absolute inset-0 animate-pulse border-2 border-cyan-300/50"></div>
                  <span class="absolute bottom-1 left-1 rounded bg-slate-950/80 px-1.5 py-0.5 font-mono text-[8px] text-cyan-200">
                    {formatDuration(buttonMotion.loopDurationMs)} en bucle
                  </span>
                {/if}
              </div>
              <span class="rounded-full bg-slate-950/80 px-2 py-1 text-[10px] text-slate-200 backdrop-blur">
                {selectedButton?.name ?? t("Selecciona un botón para ver su vista previa animada")}
              </span>
            </div>
          {:else}
            <p class="relative z-10 text-xs text-slate-500">{t("Sin fondo o miniatura para previsualizar.")}</p>
          {/if}
          <div class="absolute left-2 top-2 flex items-center gap-1 rounded-lg bg-slate-950/80 px-2 py-1 text-[9px] text-slate-300 backdrop-blur">
            <span class="h-1.5 w-1.5 rounded-full bg-cyan-400"></span>
            {t("Vista previa animada")}
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Línea de tiempo del menú")}</h2>
            <span class="font-mono text-[10px] text-cyan-300">{formatDuration(duration)}</span>
          </div>
          <div class="relative mt-2 h-24 overflow-hidden rounded-lg border border-slate-800 bg-slate-950">
            <svg class="absolute inset-0 h-full w-full" viewBox="0 0 600 96" preserveAspectRatio="none">
              <defs>
                <linearGradient id="motion-fade" x1="0" x2="0" y1="0" y2="1">
                  <stop offset="0%" stop-color="#22d3ee" stop-opacity="0.45" />
                  <stop offset="100%" stop-color="#22d3ee" stop-opacity="0.03" />
                </linearGradient>
              </defs>
              <path d="M 0 88 L 600 88" stroke="#1e293b" stroke-width="1" />
              <path d={`${fadePath("in", 88)} L 600 88 L 0 88 Z`} fill="url(#motion-fade)" />
              <path d={fadePath("in", 88)} fill="none" stroke="#22d3ee" stroke-width="2" />
              <path d={fadePath("out", 88)} fill="none" stroke="#a78bfa" stroke-width="1.5" stroke-dasharray="5 4" />
              {#each [0, 150, 300, 450, 600] as tick}
                <line x1={tick} y1="80" x2={tick} y2="90" stroke="#334155" />
              {/each}
            </svg>
            <div class="absolute inset-x-3 bottom-2 flex justify-between font-mono text-[8px] text-slate-600">
              <span>00:00</span>
              <span>{formatDuration(duration / 4)}</span>
              <span>{formatDuration(duration / 2)}</span>
              <span>{formatDuration((duration * 3) / 4)}</span>
              <span>{formatDuration(duration)}</span>
            </div>
            {#if motion}
              <div class="absolute left-2 top-2 rounded bg-slate-950/80 px-1.5 py-0.5 text-[8px] text-cyan-200">
                entrada {formatDuration(motion.fadeInMs)} / salida {formatDuration(motion.fadeOutMs)}
              </div>
            {:else}
              <div class="absolute inset-0 flex items-center justify-center text-[10px] text-slate-500">
                {t("Activa el menú animado para editar el bucle y los fundidos.")}
              </div>
            {/if}
          </div>
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <div class="flex items-center justify-between">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">
              {t("Botones de vídeo")} · {menu.buttons.length}
            </h2>
            <span class="text-[9px] text-slate-500">{t("máscara + fragmento en bucle")}</span>
          </div>
          <div class="mt-2 grid gap-2 md:grid-cols-2">
            {#each menu.buttons as button (button.id)}
              <button
                type="button"
                onclick={() => (projectStore.selectedButtonId = button.id)}
                class={`rounded-lg border p-2 text-left transition ${
                  selectedButton?.id === button.id
                    ? "border-cyan-500/60 bg-cyan-500/10"
                    : "border-slate-800 bg-slate-950/60 hover:border-slate-700"
                }`}
              >
                <div class="flex items-center gap-2">
                  {#if button.thumbnail && projectStore.previewForAsset(button.thumbnail.asset)}
                    <img
                      src={projectStore.previewForAsset(button.thumbnail.asset) ?? ""}
                      alt=""
                      class="h-10 w-16 rounded object-cover"
                      style={button.shapeMask === "circle" ? "border-radius: 9999px;" : undefined}
                    />
                  {:else}
                    <div class="flex h-10 w-16 items-center justify-center rounded bg-slate-900 text-[9px] text-slate-600">
                      {t("sin fotograma")}
                    </div>
                  {/if}
                  <div class="min-w-0">
                    <p class="truncate text-[10px] font-semibold text-slate-200">{button.name}</p>
                    <p class="text-[9px] text-slate-500">
                      {button.motionPreview ? `${formatDuration(button.motionPreview.loopDurationMs)} ${t("en bucle")}` : t("estático")} · {maskLabel(button.shapeMask)}
                    </p>
                  </div>
                </div>
              </button>
            {/each}
            {#if menu.buttons.length === 0}
              <p class="text-[10px] text-slate-500">{t("Añade botones desde la pestaña Menú.")}</p>
            {/if}
          </div>
        </div>
      </div>

      <aside class="flex min-h-0 flex-col gap-2">
        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Bucle del menú")}</h2>
          {#if motion}
            <div class="mt-2 space-y-2">
              <label class="block">
                <span class="mb-0.5 flex justify-between text-[9px] text-slate-500">
                  <span>{t("Duración")}</span>
                  <span class="font-mono">{formatDuration(duration)}</span>
                </span>
                <input
                  type="range"
                  min="1000"
                  max="120000"
                  step="500"
                  value={duration}
                  oninput={(event) => setNumber("durationMs", Number(event.currentTarget.value))}
                  class="w-full accent-cyan-400"
                />
              </label>
              <label class="flex items-center justify-between text-[10px] text-slate-400">
                <span>{t("Repetir en bucle")}</span>
                <input
                  type="checkbox"
                  checked={motion.loopEnabled}
                  onchange={(event) =>
                    projectStore.updateMenuMotion({ loopEnabled: event.currentTarget.checked })}
                  class="accent-cyan-500"
                />
              </label>
              <div class="grid grid-cols-2 gap-2">
                <label class="block">
                  <span class="mb-0.5 block text-[9px] text-slate-500">{t("Entrada (ms)")}</span>
                  <input
                    type="number"
                    min="0"
                    step="50"
                    class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={motion.fadeInMs}
                    onchange={(event) => setNumber("fadeInMs", Number(event.currentTarget.value))}
                  />
                </label>
                <label class="block">
                  <span class="mb-0.5 block text-[9px] text-slate-500">{t("Salida (ms)")}</span>
                  <input
                    type="number"
                    min="0"
                    step="50"
                    class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={motion.fadeOutMs}
                    onchange={(event) => setNumber("fadeOutMs", Number(event.currentTarget.value))}
                  />
                </label>
              </div>
              <label class="block">
                <span class="mb-0.5 flex justify-between text-[9px] text-slate-500">
                  <span>{t("Volumen de música")}</span>
                  <span class="font-mono">{Math.round((motion.audioVolume ?? 1) * 100)}%</span>
                </span>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.05"
                  value={motion.audioVolume ?? 1}
                  oninput={(event) => setNumber("audioVolume", Number(event.currentTarget.value))}
                  class="w-full accent-cyan-400"
                />
              </label>
            </div>
          {:else}
            <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
              {t("El menú estático se muestra indefinidamente. Actívalo para definir un bucle con entrada/salida y música.")}
            </p>
          {/if}
        </div>

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Transición de entrada")}</h2>
          <p class="mt-1 text-[9px] leading-4 text-slate-500">
            {t("Clip pregrabado que se reproduce antes del bucle interactivo del menú raíz.")}
          </p>
          <label class="mt-2 block">
            <span class="mb-0.5 block text-[9px] text-slate-500">{t("Clip")}</span>
            <select
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={transition?.asset ?? ""}
              onchange={(event) => {
                const value = event.currentTarget.value;
                projectStore.setMenuTransition(value || null, transition?.durationMs ?? 1_500);
              }}
            >
              <option value="">{t("Sin transición")}</option>
              {#each videoAssets as asset}
                <option value={asset.id}>{asset.displayName}</option>
              {/each}
            </select>
          </label>
          {#if transition}
            <div class="mt-2 grid grid-cols-2 gap-2">
              <label class="block">
                <span class="mb-0.5 block text-[9px] text-slate-500">{t("Duración (ms)")}</span>
                <input
                  type="number"
                  min="100"
                  step="100"
                  class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  value={transition.durationMs}
                  onchange={(event) =>
                    projectStore.updateMenuTransition({ durationMs: Math.max(100, Number(event.currentTarget.value)) })}
                />
              </label>
              <label class="block">
                <span class="mb-0.5 block text-[9px] text-slate-500">{t("Curva")}</span>
                <select
                  class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  value={transition.easing}
                  onchange={(event) =>
                    projectStore.updateMenuTransition({ easing: event.currentTarget.value as typeof transition.easing })}
                >
                  <option value="linear">Lineal</option>
                  <option value="easeIn">Ease In</option>
                  <option value="easeOut">Ease Out</option>
                  <option value="easeInOut">Ease In Out</option>
                </select>
              </label>
            </div>
          {/if}
        </div>

        {#if selectedButton}
          <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
            <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Botón seleccionado")}</h2>
            <p class="mt-1 truncate text-[11px] font-medium text-slate-200">{selectedButton.name}</p>
            <label class="mt-2 block">
              <span class="mb-0.5 block text-[9px] text-slate-500">{t("Máscara de forma")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={selectedButton.shapeMask}
                onchange={(event) =>
                  setShape(selectedButton, event.currentTarget.value as ButtonShapeMask)}
              >
                {#each shapeMasks as shape}
                  <option value={shape.id}>{t(shape.label)}</option>
                {/each}
              </select>
            </label>
            <label class="mt-2 flex items-center justify-between text-[10px] text-slate-400">
              <span>{t("Vídeo en bucle")}</span>
              <input
                type="checkbox"
                checked={buttonMotion !== null}
                onchange={(event) => enableButtonMotion(selectedButton, event.currentTarget.checked)}
                class="accent-cyan-500"
              />
            </label>
            {#if buttonMotion}
              <div class="mt-2 grid grid-cols-2 gap-2">
                <label class="block">
                  <span class="mb-0.5 block text-[9px] text-slate-500">{t("Inicio (ms)")}</span>
                  <input
                    type="number"
                    min="0"
                    step="100"
                    class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={buttonMotion.sourceStartMs}
                    onchange={(event) =>
                      setButtonMotion(
                        selectedButton,
                        "sourceStartMs",
                        Number(event.currentTarget.value),
                      )}
                  />
                </label>
                <label class="block">
                  <span class="mb-0.5 block text-[9px] text-slate-500">{t("Bucle (ms)")}</span>
                  <input
                    type="number"
                    min="500"
                    step="500"
                    class="w-full rounded-md border border-slate-800 bg-slate-900 px-1.5 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={buttonMotion.loopDurationMs}
                    onchange={(event) =>
                      setButtonMotion(
                        selectedButton,
                        "loopDurationMs",
                        Number(event.currentTarget.value),
                      )}
                  />
                </label>
              </div>
            {/if}
          </div>
        {/if}

        <div class="rounded-xl border border-slate-800 bg-slate-900/50 p-3">
          <h2 class="text-[11px] font-semibold uppercase tracking-wider text-slate-400">{t("Salida")}</h2>
          <p class="mt-1.5 text-[10px] leading-4 text-slate-500">
            {t("La duración del menú se sincroniza con el proyecto. La composición combina el fondo, la música y los fragmentos de vídeo de los botones.")}
          </p>
        </div>
      </aside>
    </div>
  {/if}
</section>
