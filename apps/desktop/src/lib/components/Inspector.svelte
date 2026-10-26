<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type {
    ButtonAction,
    ButtonStyleKind,
    Episode_Serialize,
    VideoScaleMode,
  } from "../api/bindings";
  import { normalizeRect } from "../editor/exportMenu";
  import { projectStore } from "../stores/project.svelte";
  import { basename, formatBytes, formatDuration } from "../utils/format";

  let localError = $state<string | null>(null);

  const project = $derived(projectStore.project);
  const menu = $derived(projectStore.activeMenu);
  const button = $derived(projectStore.selectedButton);
  const episode = $derived(projectStore.selectedEpisode);
  const isAvchd = $derived(project?.disc.mode === "avchd");
  const episodes = $derived(projectStore.episodes);
  const menus = $derived(project?.menus ?? []);
  const videoAssets = $derived((project?.assets ?? []).filter((asset) => asset.kind === "video"));
  const menuVideoAsset = $derived(
    menu?.background.kind === "videoLoop" ? menu.background.asset : "",
  );
  const rect = $derived(
    menu && button
      ? normalizeRect(button.rect, menu.canvas.width, menu.canvas.height)
      : { x0: 0, y0: 0, x1: 0, y1: 0 },
  );
  const thumbnailAsset = $derived(
    button?.thumbnail ? projectStore.assetFor(button.thumbnail.asset) : null,
  );
  const actionKind = $derived(button?.action.kind ?? "playAll");

  const styles: Array<{ kind: ButtonStyleKind; label: string }> = [
    { kind: "thumbnailCard", label: "Miniatura" },
    { kind: "classicFrame", label: "Clásico" },
    { kind: "glassPill", label: "Cristal" },
    { kind: "playBadge", label: "Insignia Play" },
    { kind: "textOnly", label: "Solo texto" },
    { kind: "hotspot", label: "Zona activa" },
  ];
  const scaleModes: Array<{ id: VideoScaleMode; label: string }> = [
    { id: "contain", label: "Ajustar" },
    { id: "cover", label: "Rellenar" },
    { id: "stretch", label: "Estirar" },
    { id: "letterbox", label: "Buzón (heredado)" },
    { id: "cropFill", label: "Recorte y relleno (heredado)" },
  ];
  const menuScaleModes: Array<{ id: "cover" | "contain" | "stretch"; label: string }> = [
    { id: "cover", label: "Rellenar" },
    { id: "contain", label: "Ajustar" },
    { id: "stretch", label: "Estirar" },
  ];
  function maskLabel(mask: string | null | undefined): string {
    if (mask === "circle") return t("Círculo");
    if (mask === "roundedRect") return t("Bordes redondeados");
    if (mask === "star") return t("Estrella");
    if (mask === "rectangle") return t("Rectángulo");
    return t("Sin máscara");
  }

  const fontFamilies = ["Inter", "system-ui", "Arial", "Georgia", "Courier New", "Verdana"];
  const actionKinds: Array<{ id: ButtonAction["kind"]; label: string }> = [
    { id: "playEpisode", label: "Reproducir episodio" },
    { id: "playAll", label: "Reproducir todo" },
    { id: "gotoMenu", label: "Ir a menú" },
    { id: "jumpTitleChapter", label: "Ir a capítulo" },
    { id: "playFirstPlay", label: "Inicio" },
    { id: "setAudioTrack", label: "Pista de audio" },
    { id: "resume", label: "Reanudar" },
    { id: "noop", label: "Sin acción" },
  ];

  function touchMenuName(value: string) {
    if (!menu) return;
    menu.name = value;
    projectStore.touch();
  }

  function commitRect(
    field: "x" | "y" | "w" | "h",
    value: number,
    input: HTMLInputElement,
  ) {
    if (!button) return;
    const current = normalizeRect(button.rect, menu?.canvas.width ?? 720, menu?.canvas.height ?? 480);
    const next = {
      x: field === "x" ? value : current.x0,
      y: field === "y" ? value : current.y0,
      w: field === "w" ? value : current.x1 - current.x0,
      h: field === "h" ? value : current.y1 - current.y0,
    };
    if (!projectStore.updateButtonRect(button.id, next)) {
      localError = t("El botón se solapa o no respeta la separación mínima de 4 px.");
      input.value = String(field === "x" ? current.x0 : field === "y" ? current.y0 : field === "w" ? current.x1 - current.x0 : current.y1 - current.y0);
      return;
    }
    localError = null;
  }

  function setActionKind(kind: ButtonAction["kind"]) {
    if (!button) return;
    const fallbackEpisode = episode ?? episodes[0];
    if ((kind === "playEpisode" || kind === "jumpTitleChapter") && !fallbackEpisode) {
      localError = t("Añade un episodio antes de asignar esta acción.");
      return;
    }
    const targetMenu = menu?.id ?? project?.rootMenu ?? menus[0]?.id;
    if (kind === "gotoMenu" && !targetMenu) {
      localError = t("El proyecto no tiene ningún menú de destino.");
      return;
    }
    let action: ButtonAction;
    switch (kind) {
      case "playEpisode":
        action = { kind, episode: fallbackEpisode!.id };
        break;
      case "gotoMenu":
        action = { kind, menu: targetMenu! };
        break;
      case "jumpTitleChapter":
        action = { kind, episode: fallbackEpisode!.id, chapter: 0 };
        break;
      case "setAudioTrack":
        action = { kind, track: 0 };
        break;
      default:
        action = { kind };
        break;
    }
    projectStore.updateButtonAction(button.id, action);
  }

  function patchAction(patch: Record<string, string | number>) {
    if (!button) return;
    if (
      (button.action.kind === "playEpisode" || button.action.kind === "jumpTitleChapter") &&
      patch.episode === ""
    ) {
      localError = t("Selecciona un episodio válido.");
      return;
    }
    if (button.action.kind === "gotoMenu" && patch.menu === "") {
      localError = t("Selecciona un menú válido.");
      return;
    }
    projectStore.updateButtonAction(button.id, {
      ...button.action,
      ...patch,
    } as ButtonAction);
  }

  function setEnabled(enabled: boolean) {
    if (!button) return;
    projectStore.updateButton(button.id, { enabled });
  }

  function removeThumbnail() {
    if (!button) return;
    projectStore.updateButton(button.id, { thumbnail: null });
  }

  function setEpisodeTitle(value: string) {
    if (!episode) return;
    projectStore.updateEpisode(episode.id, { title: value });
  }

  function setTrim(field: "start" | "end", value: string) {
    if (!episode) return;
    const ms = value.trim() === "" ? null : Math.round(Number(value) * 1000);
    if (ms !== null && !Number.isFinite(ms)) return;
    const accepted =
      field === "start"
        ? projectStore.setEpisodeTrimStart(episode.id, ms)
        : projectStore.setEpisodeTrimEnd(episode.id, ms);
    localError = accepted ? null : projectStore.error;
  }

  function setEpisodeScale(mode: VideoScaleMode) {
    if (!episode) return;
    projectStore.setEpisodeScaleMode(episode.id, mode);
  }

  function setPostAction(value: Episode_Serialize["postAction"]) {
    if (!episode) return;
    projectStore.updateEpisode(episode.id, { postAction: value });
  }

  function setAudioBitrate(kbps: number) {
    if (!episode) return;
    const audio = episode.audio.map((track, index) => ({
      ...track,
      ac3Kbps: index === 0 ? kbps : track.ac3Kbps,
    }));
    projectStore.updateEpisode(episode.id, { audio });
  }

  function setThumbnailTime(value: string) {
    if (!episode) return;
    const ms = value.trim() === "" ? null : Math.round(Number(value) * 1000);
    if (ms !== null && !Number.isFinite(ms)) return;
    projectStore.updateEpisode(episode.id, { thumbnailTimeMs: ms });
  }

  function removeEpisode() {
    if (!episode) return;
    if (!window.confirm(`¿Eliminar el episodio «${episode.title}»?`)) return;
    projectStore.removeEpisode(episode.id);
  }

  function chapterTime(chapter: Episode_Serialize["chapters"][number]): number {
    return episode ? projectStore.chapterTimeMs(episode, chapter) : 0;
  }

  function removeChapter(chapterId: string) {
    if (!episode) return;
    projectStore.removeChapter(episode.id, chapterId);
  }

  function actionEpisodeId(action: ButtonAction): string {
    return action.kind === "playEpisode" || action.kind === "jumpTitleChapter"
      ? action.episode
      : "";
  }

  function chaptersForAction(): Episode_Serialize | null {
    if (!button) return null;
    const id = actionEpisodeId(button.action);
    return episodes.find((item) => item.id === id) ?? null;
  }
</script>

<aside class="flex min-h-0 w-[318px] shrink-0 flex-col border-l border-slate-800/80 bg-slate-950/70">
  <div class="flex shrink-0 items-center justify-between border-b border-slate-800/80 px-3 py-2">
    <div>
      <p class="text-[10px] font-semibold uppercase tracking-[0.2em] text-slate-500">{t("Inspector")}</p>
      <p class="mt-0.5 text-[11px] font-medium text-slate-300">
        {projectStore.studioMode === "menu"
          ? button
            ? t("Botón de menú")
            : t("Propiedades del menú")
          : projectStore.studioMode === "trim"
            ? t("Episodio")
            : projectStore.studioMode === "simulator"
              ? t("Simulador")
              : projectStore.studioMode === "vmMap"
                ? t("Mapa de la VM")
                : projectStore.studioMode === "tracks"
                  ? t("Audio y subtítulos")
                  : projectStore.studioMode === "motion"
                    ? t("Menú animado")
                    : t("Calidad VBR")}
      </p>
    </div>
    <button
      type="button"
      onclick={() => (projectStore.rightCollapsed = true)}
      class="rounded-md border border-slate-800 px-1.5 py-0.5 text-[10px] text-slate-500 transition hover:border-slate-700 hover:text-slate-200"
      title={t("Ocultar inspector")}
    >
      »
    </button>
  </div>

  <div class="min-h-0 flex-1 overflow-y-auto p-3">
    {#if localError || projectStore.error}
      <div class="mb-3 rounded-lg border border-rose-500/30 bg-rose-500/10 px-2.5 py-2 text-[11px] leading-5 text-rose-200">
        {localError ?? projectStore.error}
        <button
          type="button"
          onclick={() => {
            localError = null;
            projectStore.error = null;
          }}
          class="ml-1 text-rose-300/70 hover:text-rose-100"
        >
          ✕
        </button>
      </div>
    {/if}

    {#if projectStore.studioMode === "menu"}
      {#if !menu}
        <p class="px-1 text-[11px] leading-5 text-slate-500">
          {t("Este proyecto no tiene menús. Cambia a DVD-Video para diseñar uno.")}
        </p>
      {:else if button}
        <section class="space-y-3">
          <label class="block">
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Etiqueta")}</span>
            <input
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1.5 text-[11px] text-slate-100 outline-none focus:border-cyan-500/60"
              value={button.name}
              oninput={(event) => projectStore.updateButtonName(button.id, event.currentTarget.value)}
            />
          </label>

          <div>
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Estilo")}</span>
            <div class="grid grid-cols-3 gap-1">
              {#each styles as style}
                <button
                  type="button"
                  onclick={() => projectStore.updateButton(button.id, { styleKind: style.kind })}
                  class={`rounded-lg border px-1 py-1.5 text-[10px] font-medium transition ${
                    button.styleKind === style.kind
                      ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-200"
                      : "border-slate-800 bg-slate-900/50 text-slate-400 hover:border-slate-700 hover:text-slate-200"
                  }`}
                >
                  {t(style.label)}
                </button>
              {/each}
            </div>
          </div>

          <div>
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Geometría")}</span>
            <div class="grid grid-cols-4 gap-1">
              {#each [
                { field: "x" as const, label: "X", value: rect.x0 },
                { field: "y" as const, label: "Y", value: rect.y0 },
                { field: "w" as const, label: "W", value: rect.x1 - rect.x0 },
                { field: "h" as const, label: "H", value: rect.y1 - rect.y0 },
              ] as item}
                <label class="block">
                  <span class="mb-0.5 block text-center text-[9px] text-slate-600">{item.label}</span>
                  <input
                    type="number"
                    step="2"
                    class="w-full rounded-md border border-slate-800 bg-slate-900 px-1 py-1 text-center font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                    value={item.value}
                    onchange={(event) =>
                      commitRect(item.field, Number(event.currentTarget.value), event.currentTarget)}
                  />
                </label>
              {/each}
            </div>
          </div>

          <div class="grid grid-cols-2 gap-1.5">
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Fuente")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.fontFamily}
                onchange={(event) =>
                  projectStore.updateButton(button.id, { fontFamily: event.currentTarget.value })}
              >
                {#each fontFamilies as family}
                  <option value={family}>{family}</option>
                {/each}
              </select>
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Peso")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-1.5 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.fontWeight}
                onchange={(event) =>
                  projectStore.updateButton(button.id, { fontWeight: event.currentTarget.value })}
              >
                <option value="normal">{t("Normal")}</option>
                <option value="bold">{t("Negrita")}</option>
              </select>
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Tamaño")}</span>
              <input
                type="number"
                min="8"
                max="96"
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.fontSize}
                onchange={(event) =>
                  projectStore.updateButton(button.id, {
                    fontSize: Math.max(8, Math.min(96, Number(event.currentTarget.value))),
                  })}
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Color texto")}</span>
              <input
                type="color"
                class="h-[26px] w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                value={button.textColor}
                oninput={(event) =>
                  projectStore.updateButton(button.id, { textColor: event.currentTarget.value })}
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Relleno")}</span>
              <input
                type="color"
                class="h-[26px] w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                value={button.fillColor}
                oninput={(event) =>
                  projectStore.updateButton(button.id, { fillColor: event.currentTarget.value })}
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Sombra")}</span>
              <input
                type="color"
                class="h-[26px] w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                value={button.shadowColor}
                oninput={(event) =>
                  projectStore.updateButton(button.id, { shadowColor: event.currentTarget.value })}
              />
            </label>
          </div>

          <div>
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Acción")}</span>
            <select
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1.5 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={actionKind}
              onchange={(event) =>
                setActionKind(event.currentTarget.value as ButtonAction["kind"])}
            >
              {#each actionKinds as action}
                <option value={action.id}>{t(action.label)}</option>
              {/each}
            </select>

            {#if button.action.kind === "playEpisode"}
              <select
                class="mt-1.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.action.episode}
                onchange={(event) => patchAction({ episode: event.currentTarget.value })}
              >
                {#each episodes as item}
                  <option value={item.id}>E{item.number} · {item.title}</option>
                {/each}
              </select>
            {:else if button.action.kind === "gotoMenu"}
              <select
                class="mt-1.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.action.menu}
                onchange={(event) => patchAction({ menu: event.currentTarget.value })}
              >
                {#each menus as item}
                  <option value={item.id}>{item.name}</option>
                {/each}
              </select>
            {:else if button.action.kind === "jumpTitleChapter"}
              <div class="mt-1.5 grid grid-cols-[1fr_64px] gap-1.5">
                <select
                  class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  value={button.action.episode}
                  onchange={(event) =>
                    patchAction({ episode: event.currentTarget.value, chapter: 0 })}
                >
                  {#each episodes as item}
                    <option value={item.id}>E{item.number} · {item.title}</option>
                  {/each}
                </select>
                <select
                  class="w-full rounded-lg border border-slate-800 bg-slate-900 px-1 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                  value={button.action.chapter}
                  onchange={(event) =>
                    patchAction({ chapter: Number(event.currentTarget.value) })}
                >
                  {#each chaptersForAction()?.chapters ?? [] as chapter, index}
                    <option value={index}>{index + 1}</option>
                  {/each}
                </select>
              </div>
            {:else if button.action.kind === "setAudioTrack"}
              <input
                type="number"
                min="0"
                max="7"
                class="mt-1.5 w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={button.action.track}
                onchange={(event) =>
                  patchAction({ track: Math.max(0, Number(event.currentTarget.value)) })}
              />
            {:else}
              <p class="mt-1 text-[10px] leading-4 text-slate-600">
                {actionKind === "playAll"
                  ? t("Reproduce todos los episodios en orden.")
                  : actionKind === "playFirstPlay"
                    ? t("Reproduce el contenido de inicio.")
                    : actionKind === "resume"
                      ? t("Reanuda desde el último punto (sin estado en el simulador).")
                      : t("Este botón no ejecuta ninguna acción.")}
              </p>
            {/if}
          </div>

          <div>
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Miniatura")}</span>
            {#if button.thumbnail}
              <div class="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/60 p-2">
                {#if projectStore.previewForAsset(button.thumbnail.asset)}
                  <img
                    src={projectStore.previewForAsset(button.thumbnail.asset)}
                    alt=""
                    class="h-10 w-14 rounded border border-slate-800 object-cover"
                  />
                {/if}
                <div class="min-w-0 flex-1">
                  <p class="truncate text-[10px] text-slate-300">
                    {thumbnailAsset ? basename(thumbnailAsset.displayName) : "Medio"}
                  </p>
                  <p class="text-[9px] text-slate-600">Fotograma {button.thumbnail.inFrame}</p>
                </div>
                <button
                  type="button"
                  onclick={removeThumbnail}
                  class="rounded px-1 text-[10px] text-slate-500 transition hover:bg-rose-500/10 hover:text-rose-300"
                >
                  ✕
                </button>
              </div>
            {:else}
              <p class="rounded-lg border border-dashed border-slate-800 px-2 py-2 text-[10px] leading-4 text-slate-600">
                {t("Selecciona un medio y pulsa «Miniatura» en la biblioteca.")}
              </p>
            {/if}
          </div>

          <div>
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Navegación")}</span>
            <div class="grid grid-cols-2 gap-1.5">
              {#each [
                { key: "up" as const, label: "↑" },
                { key: "down" as const, label: "↓" },
                { key: "left" as const, label: "←" },
                { key: "right" as const, label: "→" },
              ] as direction}
                <label class="flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-1.5 py-1">
                  <span class="w-3 text-center text-[10px] text-slate-500">{direction.label}</span>
                  <select
                    class="min-w-0 flex-1 bg-transparent text-[9px] text-slate-300 outline-none"
                    value={button.navOverride[direction.key] ?? ""}
                    onchange={(event) =>
                      projectStore.setButtonNavOverride(
                        button.id,
                        direction.key,
                        event.currentTarget.value || null,
                      )}
                  >
                    <option value="">{t("Automático")}</option>
                    {#each menu.buttons.filter((item) => item.id !== button.id) as target}
                      <option value={target.id}>{target.name}</option>
                    {/each}
                  </select>
                </label>
              {/each}
            </div>
          </div>

          <div class="flex items-center justify-between gap-2 border-t border-slate-800 pt-2">
            <label class="flex items-center gap-1.5 text-[10px] text-slate-400">
              <input
                type="checkbox"
                checked={button.enabled}
                onchange={(event) => setEnabled(event.currentTarget.checked)}
                class="accent-cyan-500"
              />
              {t("Activo")}
            </label>
            <button
              type="button"
              onclick={() => projectStore.deleteSelectedButton()}
              class="rounded-md border border-rose-500/30 px-2 py-1 text-[10px] text-rose-300 transition hover:bg-rose-500/10"
            >
              {t("Eliminar botón")}
            </button>
          </div>
        </section>
      {:else}
        <section class="space-y-3">
          <label class="block">
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Nombre del menú")}</span>
            <input
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1.5 text-[11px] text-slate-100 outline-none focus:border-cyan-500/60"
              value={menu.name}
              oninput={(event) => touchMenuName(event.currentTarget.value)}
            />
          </label>
          <p class="rounded-lg border border-dashed border-slate-800 px-2.5 py-2 text-[10px] leading-4 text-slate-500">
            {t("Selecciona un botón en el lienzo para editar su estilo, acción y navegación.")}
          </p>
          <button
            type="button"
            onclick={() => {
              if (!projectStore.addButton("classicFrame")) localError = projectStore.error;
            }}
            class="w-full rounded-lg bg-cyan-500 px-2 py-1.5 text-[11px] font-bold text-slate-950 transition hover:bg-cyan-400"
          >
            {t("+ Añadir botón")}
          </button>
        </section>
      {/if}

      {#if menu}
        <details class="mt-4 border-t border-slate-800 pt-3" open={!button}>
          <summary class="cursor-pointer select-none text-[10px] font-semibold uppercase tracking-wider text-slate-500">
            {t("Fondo y menú")}
          </summary>
          <div class="mt-2 space-y-3">
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Fondo animado")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={menuVideoAsset}
                onchange={(event) => {
                  const assetId = event.currentTarget.value || null;
                  projectStore.setMenuBackgroundVideo(assetId);
                  if (assetId) void projectStore.ensureVideoThumbnail(assetId);
                }}
              >
                <option value="">{t("Imagen estática")}</option>
                {#each videoAssets as asset (asset.id)}
                  <option value={asset.id}>{asset.displayName}</option>
                {/each}
              </select>
              {#if videoAssets.length === 0}
                <span class="mt-1 block text-[9px] leading-4 text-slate-600">
                  {t("Importa un vídeo desde la biblioteca para usarlo como fondo en bucle.")}
                </span>
              {:else if menu.background.kind === "videoLoop"}
                <span class="mt-1 block text-[9px] leading-4 text-cyan-300/70">
                  {t("El clip se repite en bucle y se codifica en el menú HD o DVD animado.")}
                </span>
              {/if}
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Ajuste del fondo")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={menu.scaleMode}
                onchange={(event) =>
                  projectStore.setMenuScaleMode(
                    event.currentTarget.value as "cover" | "contain" | "stretch",
                  )}
              >
                {#each menuScaleModes as mode}
                  <option value={mode.id}>{t(mode.label)}</option>
                {/each}
              </select>
            </label>
            <label class="block">
              <span class="mb-1 flex items-center justify-between text-[10px] text-slate-500">
                <span>{t("Oscurecer")}</span><span class="font-mono text-cyan-300">{menu.dimPercent}%</span>
              </span>
              <input
                type="range"
                min="0"
                max="80"
                step="1"
                value={menu.dimPercent}
                oninput={(event) => projectStore.setMenuDim(Number(event.currentTarget.value))}
                class="w-full accent-cyan-400"
              />
            </label>
            <label class="block">
              <span class="mb-1 flex items-center justify-between text-[10px] text-slate-500">
                <span>{t("Desenfoque")}</span><span class="font-mono text-cyan-300">{menu.blurPx} px</span>
              </span>
              <input
                type="range"
                min="0"
                max="20"
                step="1"
                value={menu.blurPx}
                oninput={(event) => projectStore.setMenuBlur(Number(event.currentTarget.value))}
                class="w-full accent-cyan-400"
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Color base del lienzo")}</span>
              <input
                type="color"
                class="h-7 w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                bind:value={projectStore.menuBackgroundColor}
              />
            </label>

            <div class="grid grid-cols-2 gap-1.5">
              <label class="block">
                <span class="mb-1 block text-[10px] text-slate-500">{t("Resaltado")}</span>
                <input
                  type="color"
                  class="h-7 w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                  value={projectStore.paletteHex(1)}
                  oninput={(event) => projectStore.setSubpictureColor(1, event.currentTarget.value)}
                />
              </label>
              <label class="block">
                <span class="mb-1 block text-[10px] text-slate-500">{t("Selección")}</span>
                <input
                  type="color"
                  class="h-7 w-full cursor-pointer rounded-lg border border-slate-800 bg-slate-900 p-0.5"
                  value={projectStore.paletteHex(2)}
                  oninput={(event) => projectStore.setSubpictureColor(2, event.currentTarget.value)}
                />
              </label>
            </div>

            {#if menu.audio}
              <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2">
                <div class="flex items-center justify-between">
                  <span class="text-[10px] font-medium text-slate-400">{t("Música del menú")}</span>
                  <button
                    type="button"
                    onclick={() => projectStore.setMenuAudio(null)}
                    class="text-[9px] text-slate-500 hover:text-rose-300"
                  >
                    {t("Quitar")}
                  </button>
                </div>
                <label class="mt-1.5 block">
                  <span class="mb-0.5 flex justify-between text-[9px] text-slate-500">
                    <span>{t("Ganancia")}</span><span class="font-mono">{menu.audio.gainDb ?? 0} dB</span>
                  </span>
                  <input
                    type="range"
                    min="-30"
                    max="10"
                    step="1"
                    value={menu.audio.gainDb ?? 0}
                    oninput={(event) =>
                      projectStore.setMenuAudioOption("gainDb", Number(event.currentTarget.value))}
                    class="w-full accent-cyan-400"
                  />
                </label>
                <label class="mt-1 flex items-center gap-1.5 text-[10px] text-slate-400">
                  <input
                    type="checkbox"
                    checked={menu.audio.loopAudio}
                    onchange={(event) =>
                      projectStore.setMenuAudioOption("loopAudio", event.currentTarget.checked)}
                    class="accent-cyan-500"
                  />
                  {t("Repetir en bucle")}
                </label>
              </div>
            {/if}
          </div>
        </details>
      {/if}
    {:else if projectStore.studioMode === "trim"}
      {#if !episode}
        <p class="px-1 text-[11px] leading-5 text-slate-500">
          {t("Selecciona un episodio en la secuencia o añade un vídeo desde la biblioteca.")}
        </p>
      {:else}
        <section class="space-y-3">
          <label class="block">
            <span class="mb-1 block text-[10px] font-medium uppercase tracking-wider text-slate-500">{t("Título")}</span>
            <input
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1.5 text-[11px] text-slate-100 outline-none focus:border-cyan-500/60"
              value={episode.title}
              oninput={(event) => setEpisodeTitle(event.currentTarget.value)}
            />
          </label>

          <div class="grid grid-cols-2 gap-1.5">
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Inicio (s)")}</span>
              <input
                type="number"
                min="0"
                step="0.1"
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={episode.trimStartMs === null ? "" : (episode.trimStartMs / 1000).toFixed(1)}
                placeholder="0.0"
                onchange={(event) => setTrim("start", event.currentTarget.value)}
              />
            </label>
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Fin (s)")}</span>
              <input
                type="number"
                min="0"
                step="0.1"
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={episode.trimEndMs === null ? "" : (episode.trimEndMs / 1000).toFixed(1)}
                placeholder={t("fin del vídeo")}
                onchange={(event) => setTrim("end", event.currentTarget.value)}
              />
            </label>
          </div>
          <div class="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900/50 px-2 py-1.5 text-[10px]">
            <span class="text-slate-500">{t("Duración efectiva")}</span>
            <span class="font-mono text-cyan-300">{formatDuration(projectStore.episodeEffectiveDuration(episode))}</span>
          </div>

          <label class="block">
            <span class="mb-1 block text-[10px] text-slate-500">{t("Escalado")}</span>
            <select
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={episode.scaleMode}
              onchange={(event) => setEpisodeScale(event.currentTarget.value as VideoScaleMode)}
            >
              {#each scaleModes as mode}
                <option value={mode.id}>{t(mode.label)}</option>
              {/each}
            </select>
          </label>

          <label class="block">
            <span class="mb-1 block text-[10px] text-slate-500">{t("Al terminar")}</span>
            <select
              class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
              value={episode.postAction}
              onchange={(event) =>
                setPostAction(event.currentTarget.value as Episode_Serialize["postAction"])}
            >
              <option value="returnToMenu">{t("Volver al menú")}</option>
              <option value="nextEpisode">{t("Siguiente episodio")}</option>
              <option value="stop">{t("Detener")}</option>
            </select>
          </label>

          {#if episode.audio.length > 0}
            <label class="block">
              <span class="mb-1 block text-[10px] text-slate-500">{t("Tasa de bits AC-3")}</span>
              <select
                class="w-full rounded-lg border border-slate-800 bg-slate-900 px-2 py-1 text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={episode.audio[0].ac3Kbps}
                onchange={(event) => setAudioBitrate(Number(event.currentTarget.value))}
              >
                {#each [128, 192, 224, 256, 320, 384, 448] as kbps}
                  <option value={kbps}>{kbps} kbps</option>
                {/each}
              </select>
            </label>
          {/if}

          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2">
            <div class="flex items-center justify-between">
              <span class="text-[10px] font-medium text-slate-400">{t("Miniatura")}</span>
              <button
                type="button"
                onclick={() => projectStore.updateEpisode(episode.id, { thumbnailPath: null })}
                class="text-[9px] text-slate-500 hover:text-rose-300"
              >
                {t("Quitar ruta")}
              </button>
            </div>
            {#if projectStore.episodePreview(episode)}
              <img
                src={projectStore.episodePreview(episode)}
                alt=""
                class="mt-1.5 aspect-video w-full rounded border border-slate-800 object-cover"
              />
            {/if}
            <label class="mt-1.5 block">
              <span class="mb-0.5 block text-[9px] text-slate-500">{t("Tiempo del fotograma (s)")}</span>
              <input
                type="number"
                min="0"
                step="0.1"
                class="w-full rounded-md border border-slate-800 bg-slate-900 px-2 py-1 font-mono text-[10px] text-slate-200 outline-none focus:border-cyan-500/60"
                value={episode.thumbnailTimeMs === null ? "" : (episode.thumbnailTimeMs / 1000).toFixed(1)}
                placeholder={t("automático")}
                onchange={(event) => setThumbnailTime(event.currentTarget.value)}
              />
            </label>
            <p class="mt-1 truncate font-mono text-[9px] text-slate-600" title={episode.thumbnailPath ?? ""}>
              {episode.thumbnailPath ? basename(episode.thumbnailPath) : "sin captura"}
            </p>
          </div>

          <div>
            <div class="mb-1 flex items-center justify-between">
              <span class="text-[10px] font-medium uppercase tracking-wider text-slate-500">
                {t("Capítulos")} · {episode.chapters.length}
              </span>
            </div>
            <ul class="space-y-1">
              {#each episode.chapters as chapter, index}
                <li class="flex items-center gap-1.5 rounded-md border border-slate-800 bg-slate-900/50 px-2 py-1">
                  <span class="w-3 text-center font-mono text-[9px] text-slate-600">{index + 1}</span>
                  <span class="min-w-0 flex-1 truncate text-[10px] text-slate-300">{chapter.title}</span>
                  <span class="font-mono text-[9px] text-slate-500">{formatDuration(chapterTime(chapter))}</span>
                  {#if index > 0}
                    <button
                      type="button"
                      onclick={() => removeChapter(chapter.id)}
                      class="rounded px-0.5 text-[9px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
                    >
                      ✕
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
          </div>

          <div class="border-t border-slate-800 pt-2">
            <button
              type="button"
              onclick={removeEpisode}
              class="w-full rounded-md border border-rose-500/30 px-2 py-1 text-[10px] text-rose-300 transition hover:bg-rose-500/10"
            >
              {t("Eliminar episodio")}
            </button>
          </div>
        </section>
      {/if}
    {:else if projectStore.studioMode === "simulator"}
      <section class="space-y-3">
        <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
          <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Simulación")}</p>
          <p class="mt-1 text-[10px] leading-4 text-slate-500">
            {t("El simulador usa el grafo de navegación calculado por Rust, incluidos los overrides del inspector y las acciones de cada botón.")}
          </p>
        </div>
        <label class="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900/50 px-2.5 py-1.5 text-[10px] text-slate-400">
          <span>{t("Mostrar áreas seguras")}</span>
          <input
            type="checkbox"
            checked={projectStore.showSafeAreas}
            onchange={(event) => (projectStore.showSafeAreas = event.currentTarget.checked)}
            class="accent-cyan-500"
          />
        </label>
        <label class="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900/50 px-2.5 py-1.5 text-[10px] text-slate-400">
          <span>{t("Mostrar navegación")}</span>
          <input
            type="checkbox"
            checked={projectStore.showNavigation}
            onchange={(event) => (projectStore.showNavigation = event.currentTarget.checked)}
            class="accent-cyan-500"
          />
        </label>

        {#if menu}
          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Menú activo")}</p>
            <p class="mt-1 truncate text-[11px] text-slate-200">{menu.name}</p>
            <p class="mt-0.5 text-[10px] text-slate-500">
              {menu.buttons.length} {t(menu.buttons.length === 1 ? "botón" : "botones")} ·
              {menu.canvas.width}×{menu.canvas.height}
            </p>
            <div class="mt-2 grid grid-cols-2 gap-1.5">
              <label class="block">
                <span class="mb-1 block text-[9px] text-slate-500">{t("Resaltado")}</span>
                <input
                  type="color"
                  class="h-6 w-full cursor-pointer rounded border border-slate-800 bg-slate-900 p-0.5"
                  value={projectStore.paletteHex(1)}
                  oninput={(event) => projectStore.setSubpictureColor(1, event.currentTarget.value)}
                />
              </label>
              <label class="block">
                <span class="mb-1 block text-[9px] text-slate-500">{t("Selección")}</span>
                <input
                  type="color"
                  class="h-6 w-full cursor-pointer rounded border border-slate-800 bg-slate-900 p-0.5"
                  value={projectStore.paletteHex(2)}
                  oninput={(event) => projectStore.setSubpictureColor(2, event.currentTarget.value)}
                />
              </label>
            </div>
          </div>
        {/if}

        <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
          <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Proyecto")}</p>
          <dl class="mt-1.5 space-y-1 text-[10px]">
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Episodios")}</dt>
              <dd class="text-slate-300">{episodes.length}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Duración")}</dt>
              <dd class="font-mono text-slate-300">
                {formatDuration(episodes.reduce((total, item) => total + projectStore.episodeEffectiveDuration(item), 0))}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Plan")}</dt>
              <dd class={projectStore.plan ? (projectStore.plan.feasible ? "text-emerald-300" : "text-rose-300") : "text-slate-500"}>
                {projectStore.plan ? `${projectStore.planPercent.toFixed(1)}%` : t("sin calcular")}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Tamaño fuente")}</dt>
              <dd class="text-slate-300">
                {formatBytes(episodes.reduce((total, item) => total + (projectStore.assetFor(item.video)?.probe.sizeBytes ?? 0), 0))}
              </dd>
            </div>
          </dl>
        </div>

        {#if isAvchd && menus.length === 0}
          <p class="rounded-lg border border-cyan-500/20 bg-cyan-500/10 px-2.5 py-2 text-[10px] leading-4 text-cyan-100/80">
            {t("Este proyecto AVCHD no tiene menús y se reproduce en continuo. Añade un menú para compilar Interactive Graphics HDMV.")}
          </p>
        {/if}
      </section>
    {:else}
      <section class="space-y-3">
        {#if projectStore.studioMode === "vmMap"}
          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Mapa de la VM")}</p>
            <p class="mt-1 text-[10px] leading-4 text-slate-500">
              {t("INICIO, VMGM, VTSM y títulos. Las aristas discontinuas son comandos de retorno o «Reproducir todo».")}
            </p>
            <dl class="mt-2 space-y-1 text-[10px]">
              <div class="flex justify-between gap-2">
                <dt class="text-slate-500">{t("Menús")}</dt>
                <dd class="text-slate-300">{menus.length}</dd>
              </div>
              <div class="flex justify-between gap-2">
                <dt class="text-slate-500">{t("Títulos")}</dt>
                <dd class="text-slate-300">{episodes.length}</dd>
              </div>
              <div class="flex justify-between gap-2">
                <dt class="text-slate-500">{t("Inicio")}</dt>
                <dd class="text-slate-300">{project?.firstPlay ? t("Sí") : t("No")}</dd>
              </div>
            </dl>
          </div>
          {#if projectStore.plan?.layerBreakSector !== null && projectStore.plan?.layerBreakSector !== undefined}
            <div class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5 text-[10px] leading-4 text-amber-100">
              {t("Corte de capa DVD-9 en el sector {sector} ({mode}).", {
                sector: projectStore.plan.layerBreakSector,
                mode: projectStore.plan.layerBreakMode?.toUpperCase() ?? "",
              })}
            </div>
          {/if}
        {:else if projectStore.studioMode === "tracks"}
          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Pistas del episodio")}</p>
            {#if episode}
              <p class="mt-1 truncate text-[11px] text-slate-200">{episode.title}</p>
              <dl class="mt-2 space-y-1 text-[10px]">
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Audio")}</dt>
                  <dd class="text-slate-300">{episode.audio.length} {t(episode.audio.length === 1 ? "pista" : "pistas")}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Subtítulos")}</dt>
                  <dd class="text-slate-300">{episode.subtitles.length} {t(episode.subtitles.length === 1 ? "pista" : "pistas")}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Audio por defecto")}</dt>
                  <dd class="truncate text-slate-300">
                    {episode.audio.find((track) => track.default)?.label ?? "—"}
                  </dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Subtítulo por defecto")}</dt>
                  <dd class="truncate text-slate-300">
                    {episode.subtitles.find((track) => track.isDefault)?.label ?? "—"}
                  </dd>
                </div>
              </dl>
              <ul class="mt-2 space-y-1">
                {#each episode.audio as track, index (track.sourceStream)}
                  <li class="flex items-center justify-between gap-2 rounded border border-slate-800 bg-slate-950/50 px-1.5 py-1 text-[9px]">
                    <span class="truncate text-amber-200/80">A{index + 1} · {track.label}</span>
                    <span class="font-mono text-slate-500">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                  </li>
                {/each}
                {#each episode.subtitles as track, index (index)}
                  <li class="flex items-center justify-between gap-2 rounded border border-slate-800 bg-slate-950/50 px-1.5 py-1 text-[9px]">
                    <span class="truncate text-violet-200/80">S{index + 1} · {track.label}</span>
                    <span class="font-mono text-slate-500">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="mt-1 text-[10px] text-slate-500">{t("Selecciona un episodio.")}</p>
            {/if}
          </div>
        {:else if projectStore.studioMode === "motion"}
          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Menú animado")}</p>
            {#if menu}
              <p class="mt-1 truncate text-[11px] text-slate-200">{menu.name}</p>
              <dl class="mt-2 space-y-1 text-[10px]">
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Estado")}</dt>
                  <dd class={menu.motionConfig ? "text-cyan-300" : "text-slate-500"}>
                    {menu.motionConfig ? t("Animado") : t("Estático")}
                  </dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Duración")}</dt>
                  <dd class="font-mono text-slate-300">{formatDuration(menu.timing.durationMs)}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Bucle")}</dt>
                  <dd class="text-slate-300">{menu.motionConfig?.loopEnabled ? t("Sí") : t("No")}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Botón")}</dt>
                  <dd class="truncate text-slate-300">{button?.name ?? "—"}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Máscara")}</dt>
                  <dd class="text-slate-300">{button ? maskLabel(button.shapeMask) : "—"}</dd>
                </div>
              </dl>
            {:else}
              <p class="mt-1 text-[10px] text-slate-500">{t("Sin menú activo.")}</p>
            {/if}
          </div>
        {:else}
          <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
            <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Calidad VBR")}</p>
            {#if projectStore.plan}
              <dl class="mt-2 space-y-1 text-[10px]">
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Promedio")}</dt>
                  <dd class="font-mono text-cyan-300">{projectStore.plan.videoKbps.toLocaleString("es-ES")} kbps</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Llenado")}</dt>
                  <dd class="font-mono text-slate-300">{projectStore.planPercent.toFixed(1)}%</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Viable")}</dt>
                  <dd class={projectStore.plan.feasible ? "text-emerald-300" : "text-rose-300"}>
                    {projectStore.plan.feasible ? t("Sí") : t("No")}
                  </dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Advertencias")}</dt>
                  <dd class="text-slate-300">{projectStore.plan.warnings.length}</dd>
                </div>
              </dl>
            {:else}
              <p class="mt-1 text-[10px] text-slate-500">{t("Calcula el plan para ver la curva.")}</p>
            {/if}
            {#if episode}
              <p class="mt-2 text-[9px] uppercase tracking-wider text-slate-500">{t("Prioridad del episodio")}</p>
              <div class="mt-1 flex rounded-md border border-slate-800 bg-slate-950 p-0.5">
                {#each [
                  { id: "low" as const, label: "Baja" },
                  { id: "medium" as const, label: "Media" },
                  { id: "high" as const, label: "Alta" },
                ] as priority}
                  <button
                    type="button"
                    onclick={() => projectStore.setEpisodePriority(episode.id, priority.id)}
                    class={`flex-1 rounded px-1 py-1 text-[9px] font-semibold transition ${
                      episode.encodingPriority === priority.id
                        ? "bg-cyan-500/20 text-cyan-200"
                        : "text-slate-500 hover:text-slate-200"
                    }`}
                  >
                    {t(priority.label)}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
          {#if project?.disc.video.kind === "dvd"}
            <label class="flex items-center justify-between rounded-lg border border-slate-800 bg-slate-900/50 px-2.5 py-1.5 text-[10px] text-slate-400">
              <span>{t("Trellis")}</span>
              <input
                type="checkbox"
                checked={project.disc.video.trellis}
                onchange={(event) => projectStore.setTrellis(event.currentTarget.checked)}
                class="accent-cyan-500"
              />
            </label>
          {/if}
        {/if}

        <div class="rounded-lg border border-slate-800 bg-slate-900/50 p-2.5">
          <p class="text-[10px] font-semibold uppercase tracking-wider text-slate-500">{t("Proyecto")}</p>
          <dl class="mt-1.5 space-y-1 text-[10px]">
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Episodios")}</dt>
              <dd class="text-slate-300">{episodes.length}</dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Duración")}</dt>
              <dd class="font-mono text-slate-300">
                {formatDuration(episodes.reduce((total, item) => total + projectStore.episodeEffectiveDuration(item), 0))}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Plan")}</dt>
              <dd class={projectStore.plan ? (projectStore.plan.feasible ? "text-emerald-300" : "text-rose-300") : "text-slate-500"}>
                {projectStore.plan ? `${projectStore.planPercent.toFixed(1)}%` : t("sin calcular")}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Tamaño fuente")}</dt>
              <dd class="text-slate-300">
                {formatBytes(episodes.reduce((total, item) => total + (projectStore.assetFor(item.video)?.probe.sizeBytes ?? 0), 0))}
              </dd>
            </div>
          </dl>
        </div>
      </section>
    {/if}
  </div>
</aside>
