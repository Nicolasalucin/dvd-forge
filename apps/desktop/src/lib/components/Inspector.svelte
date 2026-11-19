<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  import ColorField from "../ui/ColorField.svelte";
  import FontPicker from "../ui/FontPicker.svelte";
  import SizeField from "../ui/SizeField.svelte";
  import { i18n, t, formatPercent } from "../i18n/index.svelte";
  import type {
    ButtonAction,
    ButtonStyleKind,
    Episode_Serialize,
    VideoScaleMode,
  } from "../api/bindings";
  import { normalizeRect } from "../editor/exportMenu";
  import { commands } from "../api/bindings";
  import { projectStore } from "../stores/project.svelte";
  import { basename, formatBytes, formatDuration, parseDecimal } from "../utils/format";

  let localError = $state<string | null>(null);

  const project = $derived(projectStore.project);
  const menu = $derived(projectStore.activeMenu);
  const button = $derived(projectStore.selectedButton);
  const item = $derived(projectStore.selectedItem);
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

  // Fonts that stay legible on a TV; only the installed ones are offered.
  const curatedFonts = [
    "Segoe UI", "Arial", "Verdana", "Tahoma", "Trebuchet MS", "Calibri",
    "Georgia", "Times New Roman", "Impact", "Helvetica", "Roboto", "Noto Sans",
    "DejaVu Sans", "Inter",
  ];
  // Every font installed on this computer (the label raster runs in the
  // webview, so any local family renders on the final disc).
  let systemFonts = $state<string[]>([]);
  $effect(() => {
    void commands
      .systemFonts()
      .then((fonts) => (systemFonts = fonts))
      .catch(() => (systemFonts = []));
  });
  const recommendedFonts = $derived(
    systemFonts.length === 0
      ? ["system-ui"]
      : curatedFonts.filter((family) => systemFonts.includes(family)),
  );
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
    const chapterEpisode =
      fallbackEpisode && fallbackEpisode.chapters.length > 0
        ? fallbackEpisode
        : episodes.find((item) => item.chapters.length > 0);
    if ((kind === "playEpisode" || kind === "jumpTitleChapter") && !fallbackEpisode) {
      localError = t("Añade un episodio antes de asignar esta acción.");
      return;
    }
    if (kind === "jumpTitleChapter" && !chapterEpisode) {
      localError = t("Añade un capítulo a un episodio antes de asignar esta acción.");
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
        action = { kind, episode: chapterEpisode!.id, chapter: 0 };
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
    if (button.action.kind === "jumpTitleChapter" && typeof patch.episode === "string") {
      const target = episodes.find((item) => item.id === patch.episode);
      if (!target || target.chapters.length === 0) {
        localError = t("El episodio seleccionado no tiene capítulos.");
        return;
      }
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

  // Stream-copy notes come from Rust in Spanish with values; other locales
  // use the per-code explanation.
  const COPY_NOTES: Record<string, string> = {
    COPY_OK: "Compatible: el vídeo se copia sin recodificar (más rápido y sin pérdida).",
    COPY_DISABLED: "Recodificación elegida para este episodio.",
    COPY_FILTERS: "Tiene ajustes de imagen (girar, recortar, color…); se recodifica.",
    COPY_NO_VIDEO: "El archivo no tiene vídeo analizable.",
    COPY_CODEC: "El vídeo no es H.264; se recodifica.",
    COPY_PROFILE: "El perfil H.264 no es High o Main; se recodifica.",
    COPY_LEVEL: "El nivel H.264 supera el máximo del disco; se recodifica.",
    COPY_PIXEL_FORMAT: "El formato de color no es yuv420p (8 bits); se recodifica.",
    COPY_RESOLUTION: "La resolución del vídeo no coincide con la del proyecto; se recodifica.",
    COPY_FRAME_RATE: "Los fps del vídeo no coinciden con los del proyecto; se recodifica.",
    COPY_SCAN: "El barrido (progresivo/entrelazado) no coincide con el proyecto; se recodifica.",
    COPY_TRIM: "El episodio está recortado al principio; para copiar sin recodificar no puede haber recorte inicial.",
    COPY_BITRATE: "El bitrate del vídeo es desconocido o supera el máximo del disco; se recodifica.",
  };

  function copyNote(code: string | null | undefined, note: string): string {
    if (i18n.locale === "es" || !code || !COPY_NOTES[code]) return t(note);
    return t(COPY_NOTES[code]);
  }

  function removeThumbnail() {
    if (!button) return;
    projectStore.updateButton(button.id, { thumbnail: null });
  }

  /** Episode this button plays, used for one-click thumbnails. */
  const buttonEpisode = $derived.by(() => {
    const action = button?.action;
    if (!action || (action.kind !== "playEpisode" && action.kind !== "jumpTitleChapter")) return null;
    return projectStore.episodes.find((item) => item.id === action.episode) ?? null;
  });

  function useEpisodeThumbnail() {
    if (!button || !buttonEpisode) return;
    projectStore.useEpisodeThumbnail(button.id, buttonEpisode.id);
  }

  function pickThumbnailFrame() {
    if (!buttonEpisode) return;
    projectStore.selectEpisode(buttonEpisode.id);
    projectStore.studioMode = "trim";
  }

  function setEpisodeTitle(value: string) {
    if (!episode) return;
    projectStore.updateEpisode(episode.id, { title: value });
  }

  function setTrim(field: "start" | "end", value: string) {
    if (!episode) return;
    const ms = value.trim() === "" ? null : Math.round(parseDecimal(value) * 1000);
    if (ms !== null && (!Number.isFinite(ms) || ms < 0)) return;
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
    const ms = value.trim() === "" ? null : Math.round(parseDecimal(value) * 1000);
    if (ms !== null && (!Number.isFinite(ms) || ms < 0)) return;
    projectStore.updateEpisode(episode.id, { thumbnailTimeMs: ms });
  }

  function removeEpisode() {
    if (!episode) return;
    if (!window.confirm(t("¿Eliminar el episodio «{episode}»?", { episode: episode.title }))) return;
    projectStore.removeEpisode(episode.id);
  }

  function chapterTime(chapter: Episode_Serialize["chapters"][number]): number {
    return episode
      ? (episode.trimStartMs ?? 0) + projectStore.chapterTimeMs(episode, chapter)
      : 0;
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

<aside class="ui-panel w-[300px] shrink-0 border-l border-slate-800">
  <div class="ui-panel-header">
    <p class="ui-title truncate">
      {projectStore.studioMode === "menu"
        ? item
          ? t("Texto u objeto")
          : button
          ? t("Botón de menú")
          : t("Propiedades del menú")
        : projectStore.studioMode === "trim"
          ? t("Episodio")
          : projectStore.studioMode === "simulator"
            ? t("Vista previa")
            : projectStore.studioMode === "vmMap"
              ? t("Mapa de la VM")
              : projectStore.studioMode === "tracks"
                ? t("Audio y subtítulos")
                : projectStore.studioMode === "motion"
                  ? t("Menú animado")
                  : t("Calidad VBR")}
    </p>
    <button
      type="button"
      onclick={() => (projectStore.rightCollapsed = true)}
      class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon"
      title={t("Ocultar propiedades")}
      aria-label={t("Ocultar propiedades")}
    ><Icon name="chevronRight" size={14} /></button>
  </div>

  <div class="inspector-body min-h-0 flex-1 overflow-y-auto px-3 pb-4 pt-3">
    {#if localError}
      <!-- Global errors already show in the banner under the toolbar. -->
      <div class="mb-3 rounded-md bg-rose-500/10 px-2.5 py-2 text-[12px] leading-5 text-rose-200" role="alert">
        {localError}
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
        <p class="px-1 text-[12px] leading-5 text-slate-500">
          {t("Este proyecto no tiene menús. Cambia a DVD-Video para diseñar uno.")}
        </p>
      {:else if item}
        <section class="inspector-sections space-y-4">
          {#if item.kind === "text"}
            <div>
              <span class="ui-label">{t("Texto")}</span>
              <textarea
                class="ui-input min-h-16 resize-y"
                rows="2"
                value={item.text}
                oninput={(event) => projectStore.updateItem(item.id, { text: event.currentTarget.value })}
              ></textarea>
            </div>
            <div class="space-y-3">
              <FontPicker
                label={t("Tipo de letra")}
                value={item.fontFamily}
                recommended={recommendedFonts}
                installed={systemFonts}
                onchange={(family) => projectStore.updateItem(item.id, { fontFamily: family })}
              />
              <div class="grid grid-cols-[1fr_auto] items-start gap-2">
                <SizeField
                  label={t("Tamaño")}
                  value={item.fontSize ?? 32}
                  max={160}
                  onchange={(size) => projectStore.updateItem(item.id, { fontSize: size })}
                />
                <div>
                  <span class="ui-label">{t("Alineación")}</span>
                  <div class="ui-seg" role="group" aria-label={t("Alineación")}>
                    {#each [{ id: "left", label: t("Izq.") }, { id: "center", label: t("Centro") }, { id: "right", label: t("Der.") }] as align}
                      <button type="button" aria-pressed={item.align === align.id} onclick={() => projectStore.updateItem(item.id, { align: align.id })}>
                        {align.label}
                      </button>
                    {/each}
                  </div>
                </div>
              </div>
              <ColorField label={t("Color")} value={item.color} onchange={(hex) => projectStore.updateItem(item.id, { color: hex })} />
            </div>
          {:else if item.kind === "shape"}
            <div class="grid grid-cols-2 gap-2">
              <ColorField label={t("Relleno")} value={item.fill} onchange={(hex) => projectStore.updateItem(item.id, { fill: hex })} />
              <ColorField label={t("Borde")} value={item.stroke ?? "#000000"} onchange={(hex) => projectStore.updateItem(item.id, { stroke: hex, strokeWidth: item.strokeWidth || 2 })} />
            </div>
          {:else if item.kind === "image"}
            <label class="block">
              <span class="ui-label">{t("Opacidad")} · {Math.round((item.opacity ?? 1) * 100)}%</span>
              <input type="range" min="0.1" max="1" step="0.05" class="w-full accent-cyan-500" value={item.opacity ?? 1}
                oninput={(event) => projectStore.updateItem(item.id, { opacity: Number(event.currentTarget.value) })} />
            </label>
          {/if}
          <div>
            <span class="ui-label">{t("Posición y tamaño")}</span>
            <div class="grid grid-cols-4 gap-1.5">
              {#each [["x", "X"], ["y", "Y"], ["w", "W"], ["h", "H"]] as [field, label]}
                <label class="block">
                  <span class="mb-0.5 block text-center text-[11px] text-slate-500">{label}</span>
                  <input
                    class="ui-input ui-input-sm text-center tabular-nums"
                    type="number"
                    value={Math.round((item.rect as Record<string, number | null>)[field] ?? 0)}
                    onchange={(event) => projectStore.updateItem(item.id, { rect: { ...item.rect, [field]: Math.max(0, Math.round(Number(event.currentTarget.value) / 2) * 2) } })}
                  />
                </label>
              {/each}
            </div>
          </div>
          <button type="button" class="ui-btn ui-btn-danger w-full" onclick={() => projectStore.deleteSelectedItem()}>
            <Icon name="trash" size={15} />{t("Eliminar")}
          </button>
        </section>
      {:else if button}
        <section class="inspector-sections space-y-4">
          <label class="ui-label">
            <span class="ui-section-title mb-1 block">{t("Etiqueta")}</span>
            <input
              class="ui-input w-full"
              value={button.name}
              oninput={(event) => projectStore.updateButtonName(button.id, event.currentTarget.value)}
            />
          </label>

          <div>
            <span class="ui-section-title mb-1 block">{t("Estilo")}</span>
            <div class="grid grid-cols-3 gap-1">
              {#each styles as style}
                <button
                  type="button"
                  onclick={() => projectStore.updateButton(button.id, { styleKind: style.kind })}
                  aria-pressed={button.styleKind === style.kind}
                  class={`h-8 truncate rounded-md border px-1 text-[12px] ${
                    button.styleKind === style.kind
                      ? "border-cyan-500 bg-cyan-500/15 text-slate-50"
                      : "border-slate-700 text-slate-300 hover:border-slate-500 hover:text-slate-50"
                  }`}
                >
                  {t(style.label)}
                </button>
              {/each}
            </div>
          </div>

          <div>
            <span class="ui-section-title mb-1 block">{t("Geometría")}</span>
            <div class="grid grid-cols-4 gap-1">
              {#each [
                { field: "x" as const, label: "X", value: rect.x0 },
                { field: "y" as const, label: "Y", value: rect.y0 },
                { field: "w" as const, label: "W", value: rect.x1 - rect.x0 },
                { field: "h" as const, label: "H", value: rect.y1 - rect.y0 },
              ] as item}
                <label class="ui-label">
                  <span class="mb-0.5 block text-center text-[11px] text-slate-600">{item.label}</span>
                  <input
                    type="number"
                    step="2"
                    class="ui-input ui-input-sm w-full text-center font-mono"
                    value={item.value}
                    onchange={(event) =>
                      commitRect(item.field, Number(event.currentTarget.value), event.currentTarget)}
                  />
                </label>
              {/each}
            </div>
          </div>

          <section class="inspector-sections space-y-4">
            <span class="ui-section-title block">{t("Texto")}</span>
            <!-- What the button text will look like on the disc. -->
            <div
              class="flex h-14 items-center justify-center overflow-hidden rounded-md border border-slate-800 px-3"
              style={`background:${button.fillColor}`}
            >
              <span
                class="truncate"
                style={`font-family:"${button.fontFamily.replaceAll('"', "")}", system-ui, sans-serif; font-weight:${button.fontWeight === "bold" ? 700 : 400}; font-size:${Math.min(30, Math.max(12, button.fontSize * 0.75))}px; color:${button.textColor}; text-shadow: 0 1px 2px ${button.shadowColor}`}
              >{button.label?.text || button.name || t("Texto del botón")}</span>
            </div>
            <FontPicker
              label={t("Tipo de letra")}
              value={button.fontFamily}
              recommended={recommendedFonts}
              installed={systemFonts}
              onchange={(family) => projectStore.updateButton(button.id, { fontFamily: family })}
            />
            <div class="grid grid-cols-[1fr_auto] items-start gap-2">
              <SizeField
                label={t("Tamaño")}
                value={button.fontSize}
                onchange={(size) => projectStore.updateButton(button.id, { fontSize: size })}
              />
              <div>
                <span class="ui-label">{t("Peso")}</span>
                <div class="flex h-8 overflow-hidden rounded-md border border-slate-700" role="group" aria-label={t("Peso")}>
                  {#each [{ id: "normal", label: t("Normal"), weight: 400 }, { id: "bold", label: t("Negrita"), weight: 700 }] as option}
                    <button
                      type="button"
                      class={`px-2.5 text-[13px] ${button.fontWeight === option.id ? "bg-slate-700 text-slate-50" : "bg-slate-900 text-slate-400 hover:text-slate-100"}`}
                      style={`font-weight:${option.weight}`}
                      aria-pressed={button.fontWeight === option.id}
                      title={option.label}
                      onclick={() => projectStore.updateButton(button.id, { fontWeight: option.id })}
                    >Aa</button>
                  {/each}
                </div>
              </div>
            </div>
            <div class="grid grid-cols-3 gap-2">
              <ColorField label={t("Texto")} value={button.textColor}
                onchange={(hex) => projectStore.updateButton(button.id, { textColor: hex })} />
              <ColorField label={t("Relleno")} value={button.fillColor}
                onchange={(hex) => projectStore.updateButton(button.id, { fillColor: hex })} />
              <ColorField label={t("Sombra")} value={button.shadowColor}
                onchange={(hex) => projectStore.updateButton(button.id, { shadowColor: hex })} />
            </div>
          </section>

          <div class="space-y-2">
            <span class="ui-section-title block">{t("Al pulsar")}</span>
            <select
              class="ui-input"
              value={actionKind}
              aria-label={t("Acción")}
              onchange={(event) => setActionKind(event.currentTarget.value as ButtonAction["kind"])}
            >
              {#each actionKinds as action}
                <option value={action.id}>{t(action.label)}</option>
              {/each}
            </select>

            {#if button.action.kind === "playEpisode"}
              <label class="block">
                <span class="ui-label">{t("Episodio")}</span>
                <select class="ui-input" value={button.action.episode} onchange={(event) => patchAction({ episode: event.currentTarget.value })}>
                  {#each episodes as item}
                    <option value={item.id}>{item.number} · {item.title}</option>
                  {/each}
                </select>
              </label>
            {:else if button.action.kind === "gotoMenu"}
              <label class="block">
                <span class="ui-label">{t("Menú")}</span>
                <select class="ui-input" value={button.action.menu} onchange={(event) => patchAction({ menu: event.currentTarget.value })}>
                  {#each menus as item}
                    <option value={item.id}>{item.name}</option>
                  {/each}
                </select>
              </label>
            {:else if button.action.kind === "jumpTitleChapter"}
              <div class="grid grid-cols-[1fr_88px] gap-2">
                <label class="block">
                  <span class="ui-label">{t("Episodio")}</span>
                  <select class="ui-input" value={button.action.episode} onchange={(event) => patchAction({ episode: event.currentTarget.value, chapter: 0 })}>
                    {#each episodes as item}
                      <option value={item.id}>{item.number} · {item.title}</option>
                    {/each}
                  </select>
                </label>
                <label class="block">
                  <span class="ui-label">{t("Capítulo")}</span>
                  <select class="ui-input" value={button.action.chapter} onchange={(event) => patchAction({ chapter: Number(event.currentTarget.value) })}>
                    {#each chaptersForAction()?.chapters ?? [] as chapter, index}
                      <option value={index}>{index + 1}</option>
                    {/each}
                  </select>
                </label>
              </div>
            {:else if button.action.kind === "setAudioTrack"}
              <label class="block">
                <span class="ui-label">{t("Pista de audio")}</span>
                <input type="number" min="1" max="8" class="ui-input tabular-nums" value={button.action.track + 1}
                  onchange={(event) => patchAction({ track: Math.max(0, Number(event.currentTarget.value) - 1) })} />
              </label>
            {:else}
              <p class="ui-hint">
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

          <div class="space-y-2">
            <span class="ui-section-title block">{t("Miniatura")}</span>
            {#if button.thumbnail}
              <div class="overflow-hidden rounded-md border border-slate-700">
                {#if projectStore.previewForAsset(button.thumbnail.asset)}
                  <img src={projectStore.previewForAsset(button.thumbnail.asset)} alt="" class="aspect-video w-full bg-black object-cover" />
                {/if}
                <div class="flex items-center gap-2 px-2 py-1.5">
                  <span class="min-w-0 flex-1 truncate text-[12px] text-slate-300">
                    {thumbnailAsset ? basename(thumbnailAsset.displayName) : t("Medio")}
                  </span>
                  <button type="button" class="ui-btn ui-btn-sm ui-btn-ghost ui-btn-icon ui-btn-danger" onclick={removeThumbnail} title={t("Quitar miniatura")} aria-label={t("Quitar miniatura")}>
                    <Icon name="close" size={14} />
                  </button>
                </div>
              </div>
            {:else}
              <p class="ui-hint">{t("Sin miniatura. Usa la del episodio o «⋯ › Usar como miniatura del botón» en Medios.")}</p>
            {/if}
            {#if buttonEpisode}
              <div class="grid grid-cols-2 gap-2">
                <button type="button" class="ui-btn ui-btn-sm" onclick={useEpisodeThumbnail}>
                  <Icon name="film" size={14} />{t("Del episodio")}
                </button>
                <button type="button" class="ui-btn ui-btn-sm" onclick={pickThumbnailFrame} title={t("Abre el recorte del episodio para elegir el fotograma exacto")}>
                  <Icon name="photo" size={14} />{t("Fotograma…")}
                </button>
              </div>
            {/if}
          </div>

          <div class="space-y-2">
            <span class="ui-section-title block">{t("Navegación con el mando")}</span>
            <!-- D-pad: each arrow chooses the button reached from this one. -->
            <div class="grid grid-cols-3 items-center gap-1.5">
              {#each [
                { key: "up" as const, label: "↑", cell: "col-start-2" },
                { key: "left" as const, label: "←", cell: "col-start-1 row-start-2" },
                { key: "right" as const, label: "→", cell: "col-start-3 row-start-2" },
                { key: "down" as const, label: "↓", cell: "col-start-2 row-start-3" },
              ] as direction}
                <label class={`relative block ${direction.cell}`} title={t("Botón al pulsar {arrow}", { arrow: direction.label })}>
                  <span class="pointer-events-none absolute left-1.5 top-1/2 -translate-y-1/2 text-[12px] text-slate-500">{direction.label}</span>
                  <select
                    class="ui-input ui-input-sm pl-5"
                    value={button.navOverride[direction.key] ?? ""}
                    onchange={(event) => projectStore.setButtonNavOverride(button.id, direction.key, event.currentTarget.value || null)}
                  >
                    <option value="">{t("Auto")}</option>
                    {#each menu.buttons.filter((item) => item.id !== button.id) as target}
                      <option value={target.id}>{target.name}</option>
                    {/each}
                  </select>
                </label>
              {/each}
              <span class="col-start-2 row-start-2 truncate rounded-md bg-cyan-500/15 px-2 py-1 text-center text-[12px] text-slate-100">{button.name}</span>
            </div>
          </div>

          <div class="flex items-center justify-between gap-2">
            <label class="flex items-center gap-2 text-[13px] text-slate-300">
              <input type="checkbox" class="ui-check" checked={button.enabled} onchange={(event) => setEnabled(event.currentTarget.checked)} />
              {t("Activo")}
            </label>
            <button type="button" class="ui-btn ui-btn-sm ui-btn-danger" onclick={() => projectStore.deleteSelectedButton()}>
              <Icon name="trash" size={14} />{t("Eliminar botón")}
            </button>
          </div>
        </section>
      {:else}
        <section class="inspector-sections space-y-4">
          <label class="ui-label">
            <span class="ui-section-title mb-1 block">{t("Nombre del menú")}</span>
            <input
              class="ui-input w-full"
              value={menu.name}
              oninput={(event) => touchMenuName(event.currentTarget.value)}
            />
          </label>
          <p class="rounded-lg border border-dashed border-slate-800 px-2.5 py-2 text-[12px] leading-5 text-slate-500">
            {t("Selecciona un botón en el lienzo para editar su estilo, acción y navegación.")}
          </p>
          <button
            type="button"
            onclick={() => {
              if (!projectStore.addButton("classicFrame")) localError = projectStore.error;
            }}
            class="w-full rounded-lg bg-cyan-500 px-2 py-1.5 text-[12px] font-bold text-slate-950 transition hover:bg-cyan-400"
          >
            {t("+ Añadir botón")}
          </button>
        </section>
      {/if}

      {#if menu}
        <details class="mt-4 border-t border-slate-800 pt-3 [&_summary]:list-none" open={!button}>
          <summary class="ui-section-title flex cursor-pointer select-none items-center gap-1.5">
            {t("Fondo y menú")}
          </summary>
          <div class="mt-2 space-y-3">
            <label class="ui-label">
              <span class="ui-label">{t("Fondo animado")}</span>
              <select
                class="ui-input ui-input-sm w-full"
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
                <span class="mt-1 block text-[12px] leading-5 text-slate-600">
                  {t("Importa un vídeo desde la biblioteca para usarlo como fondo en bucle.")}
                </span>
              {:else if menu.background.kind === "videoLoop"}
                <span class="mt-1 block text-[12px] leading-5 text-cyan-300/70">
                  {t("El clip se repite en bucle y se codifica en el menú HD o DVD animado.")}
                </span>
              {/if}
            </label>
            <label class="ui-label">
              <span class="ui-label">{t("Ajuste del fondo")}</span>
              <select
                class="ui-input ui-input-sm w-full"
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
            <label class="ui-label">
              <span class="mb-1 flex items-center justify-between text-[11px] text-slate-500">
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
            <label class="ui-label">
              <span class="mb-1 flex items-center justify-between text-[11px] text-slate-500">
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
            <ColorField label={t("Color base del lienzo")} value={projectStore.menuBackgroundColor}
              onchange={(hex) => (projectStore.menuBackgroundColor = hex)} />

            <div class="grid grid-cols-2 gap-1.5">
              <ColorField label={t("Resaltado")} value={projectStore.paletteHex(1)}
                onchange={(hex) => projectStore.setSubpictureColor(1, hex)} />
              <ColorField label={t("Selección")} value={projectStore.paletteHex(2)}
                onchange={(hex) => projectStore.setSubpictureColor(2, hex)} />
            </div>

            {#if menu.audio}
              <div class="rounded-lg border border-slate-800 p-2">
                <div class="flex items-center justify-between">
                  <span class="text-[11px] font-medium text-slate-400">{t("Música del menú")}</span>
                  <button
                    type="button"
                    onclick={() => projectStore.setMenuAudio(null)}
                    class="text-[11px] text-slate-500 hover:text-rose-300"
                  >
                    {t("Quitar")}
                  </button>
                </div>
                <label class="mt-1.5 block">
                  <span class="mb-0.5 flex justify-between text-[11px] text-slate-500">
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
                <label class="mt-1 flex items-center gap-1.5 text-[11px] text-slate-400">
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
        <p class="px-1 text-[12px] leading-5 text-slate-500">
          {t("Selecciona un episodio en la secuencia o añade un vídeo desde la biblioteca.")}
        </p>
      {:else}
        <section class="inspector-sections space-y-4">
          <label class="ui-label">
            <span class="ui-section-title mb-1 block">{t("Título")}</span>
            <input
              class="ui-input w-full"
              value={episode.title}
              oninput={(event) => setEpisodeTitle(event.currentTarget.value)}
            />
          </label>

          <div class="grid grid-cols-2 gap-1.5">
            <label class="ui-label">
              <span class="ui-label">{t("Inicio (s)")}</span>
              <input
                type="text"
                inputmode="decimal"
                class="ui-input ui-input-sm w-full font-mono"
                value={episode.trimStartMs === null ? "" : (episode.trimStartMs / 1000).toFixed(1)}
                placeholder="0.0"
                onchange={(event) => setTrim("start", event.currentTarget.value)}
              />
            </label>
            <label class="ui-label">
              <span class="ui-label">{t("Fin (s)")}</span>
              <input
                type="text"
                inputmode="decimal"
                class="ui-input ui-input-sm w-full font-mono"
                value={episode.trimEndMs === null ? "" : (episode.trimEndMs / 1000).toFixed(1)}
                placeholder={t("fin del vídeo")}
                onchange={(event) => setTrim("end", event.currentTarget.value)}
              />
            </label>
          </div>
          <div class="flex items-center justify-between rounded-lg border border-slate-800 px-2 py-1.5 text-[11px]">
            <span class="text-slate-500">{t("Duración efectiva")}</span>
            <span class="font-mono text-cyan-300">{formatDuration(projectStore.episodeEffectiveDuration(episode))}</span>
          </div>

          <label class="ui-label">
            <span class="ui-label">{t("Escalado")}</span>
            <select
              class="ui-input ui-input-sm w-full"
              value={episode.scaleMode}
              onchange={(event) => setEpisodeScale(event.currentTarget.value as VideoScaleMode)}
            >
              {#each scaleModes.filter((mode) => !mode.label.includes("(heredado)") || projectStore.expertMode || mode.id === episode.scaleMode) as mode}
                <option value={mode.id}>{t(mode.label)}</option>
              {/each}
            </select>
          </label>

          {#if project?.disc.mode === "avchd"}
            {@const planEntry = projectStore.plan?.perEpisode.find((entry) => entry.episode === episode.id)}
            <label class="ui-label">
              <span class="ui-label">{t("Vídeo")}</span>
              <select
                class="ui-input ui-input-sm w-full"
                value={episode.videoMode ?? "auto"}
                onchange={(event) =>
                  projectStore.updateEpisode(episode.id, {
                    videoMode: event.currentTarget.value as Episode_Serialize["videoMode"],
                  })}
              >
                <option value="auto">{t("Automático: copiar si ya es compatible")}</option>
                <option value="copy">{t("Copiar sin recodificar")}</option>
                <option value="reencode">{t("Recodificar siempre")}</option>
              </select>
              {#if planEntry?.streamCopyNote}
                <p
                  class={`mt-1 rounded-md px-2 py-1 text-[12px] leading-5 ${
                    planEntry.streamCopy
                      ? "border border-emerald-500/25 bg-emerald-500/10 text-emerald-200"
                      : "border border-slate-800 bg-slate-900/60 text-slate-400"
                  }`}
                >
                  {planEntry.streamCopy ? "⚡ " : "↻ "}{copyNote(planEntry.streamCopyCode, planEntry.streamCopyNote)}
                </p>
              {/if}
            </label>
          {/if}

          <label class="ui-label">
            <span class="ui-label">{t("Al terminar")}</span>
            <select
              class="ui-input ui-input-sm w-full"
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
            <label class="ui-label">
              <span class="ui-label">{t("Tasa de bits AC-3")}</span>
              <select
                class="ui-input ui-input-sm w-full"
                value={episode.audio[0].ac3Kbps}
                onchange={(event) => setAudioBitrate(Number(event.currentTarget.value))}
              >
                {#each [128, 192, 224, 256, 320, 384, 448] as kbps}
                  <option value={kbps}>{kbps} kbps</option>
                {/each}
              </select>
            </label>
          {/if}

          <div class="rounded-lg border border-slate-800 p-2">
            <div class="flex items-center justify-between">
              <span class="text-[11px] font-medium text-slate-400">{t("Miniatura")}</span>
              {#if episode.thumbnailPath || episode.thumbnailTimeMs !== null}
                <button
                  type="button"
                  onclick={() => projectStore.updateEpisode(episode.id, { thumbnailPath: null, thumbnailTimeMs: null })}
                  class="ui-btn-ghost text-[11px]"
                >
                  {t("Restablecer")}
                </button>
              {/if}
            </div>
            {#if projectStore.episodePreview(episode)}
              <img
                src={projectStore.episodePreview(episode)}
                alt=""
                class="mt-1.5 aspect-video w-full rounded border border-slate-800 bg-black object-contain"
              />
            {/if}
            <label class="mt-1.5 block">
              <span class="ui-label">{t("Tiempo del fotograma (s)")}</span>
              <input
                type="text"
                inputmode="decimal"
                class="ui-input ui-input-sm w-full font-mono"
                value={episode.thumbnailTimeMs === null ? "" : (episode.thumbnailTimeMs / 1000).toFixed(1)}
                placeholder={t("automático")}
                onchange={(event) => setThumbnailTime(event.currentTarget.value)}
              />
            </label>
          </div>

          <div>
            <div class="mb-1 flex items-center justify-between">
              <span class="ui-section-title">
                {t("Capítulos")} · {episode.chapters.length}
              </span>
            </div>
            <ul class="space-y-1">
              {#each episode.chapters as chapter, index}
                <li class="flex items-center gap-1.5 rounded-md border border-slate-800 px-2 py-1">
                  <span class="w-3 text-center font-mono text-[11px] text-slate-600">{index + 1}</span>
                  <span class="min-w-0 flex-1 truncate text-[11px] text-slate-300">{chapter.title}</span>
                  <span class="font-mono text-[11px] text-slate-500">{formatDuration(chapterTime(chapter))}</span>
                  {#if index > 0}
                    <button
                      type="button"
                      onclick={() => removeChapter(chapter.id)}
                      class="rounded px-0.5 text-[11px] text-slate-600 transition hover:bg-rose-500/10 hover:text-rose-300"
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
              class="w-full rounded-md border border-rose-500/30 px-2 py-1 text-[11px] text-rose-300 transition hover:bg-rose-500/10"
            >
              {t("Eliminar episodio")}
            </button>
          </div>
        </section>
      {/if}
    {:else if projectStore.studioMode === "simulator"}
      <section class="inspector-sections space-y-4">
        <div class="rounded-lg border border-slate-800 p-2.5">
          <p class="ui-section-title">{t("Simulación")}</p>
          <p class="mt-1 text-[12px] leading-5 text-slate-500">
            {t("Prueba el menú como en un reproductor: flechas y OK del mando, o el teclado.")}
          </p>
        </div>
        <label class="flex items-center justify-between rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] text-slate-400">
          <span>{t("Mostrar áreas seguras")}</span>
          <input
            type="checkbox"
            checked={projectStore.showSafeAreas}
            onchange={(event) => (projectStore.showSafeAreas = event.currentTarget.checked)}
            class="accent-cyan-500"
          />
        </label>
        <label class="flex items-center justify-between rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] text-slate-400">
          <span>{t("Mostrar navegación")}</span>
          <input
            type="checkbox"
            checked={projectStore.showNavigation}
            onchange={(event) => (projectStore.showNavigation = event.currentTarget.checked)}
            class="accent-cyan-500"
          />
        </label>

        {#if menu}
          <div class="rounded-lg border border-slate-800 p-2.5">
            <p class="ui-section-title">{t("Menú activo")}</p>
            <p class="mt-1 truncate text-[12px] text-slate-200">{menu.name}</p>
            <p class="mt-0.5 text-[11px] text-slate-500">
              {menu.buttons.length} {t(menu.buttons.length === 1 ? "botón" : "botones")} ·
              {menu.canvas.width}×{menu.canvas.height}
            </p>
            <div class="mt-2 grid grid-cols-2 gap-1.5">
              <ColorField label={t("Resaltado")} value={projectStore.paletteHex(1)}
                onchange={(hex) => projectStore.setSubpictureColor(1, hex)} />
              <ColorField label={t("Selección")} value={projectStore.paletteHex(2)}
                onchange={(hex) => projectStore.setSubpictureColor(2, hex)} />
            </div>
          </div>
        {/if}

        <div class="rounded-lg border border-slate-800 p-2.5">
          <p class="ui-section-title">{t("Proyecto")}</p>
          <dl class="mt-1.5 space-y-1 text-[11px]">
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
                {projectStore.plan ? formatPercent(projectStore.planPercent) : t("sin calcular")}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Tamaño de los archivos")}</dt>
              <dd class="text-slate-300">
                {formatBytes(episodes.reduce((total, item) => total + (projectStore.assetFor(item.video)?.probe.sizeBytes ?? 0), 0))}
              </dd>
            </div>
          </dl>
        </div>

        {#if isAvchd && menus.length === 0}
          <p class="rounded-lg border border-cyan-500/20 bg-cyan-500/10 px-2.5 py-2 text-[12px] leading-5 text-cyan-100/80">
            {t("Este proyecto AVCHD no tiene menús y se reproduce en continuo. Añade un menú para compilar Interactive Graphics HDMV.")}
          </p>
        {/if}
      </section>
    {:else}
      <section class="inspector-sections space-y-4">
        {#if projectStore.studioMode === "vmMap"}
          <div class="rounded-lg border border-slate-800 p-2.5">
            <p class="ui-section-title">{t("Mapa de la VM")}</p>
            <p class="mt-1 text-[12px] leading-5 text-slate-500">
              {t("INICIO, VMGM, VTSM y títulos. Las aristas discontinuas son comandos de retorno o «Reproducir todo».")}
            </p>
            <dl class="mt-2 space-y-1 text-[11px]">
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
            <div class="rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5 text-[12px] leading-5 text-amber-100">
              {t("Corte de capa DVD-9 en el sector {sector} ({mode}).", {
                sector: projectStore.plan.layerBreakSector,
                mode: projectStore.plan.layerBreakMode?.toUpperCase() ?? "",
              })}
            </div>
          {/if}
        {:else if projectStore.studioMode === "tracks"}
          <div class="rounded-lg border border-slate-800 p-2.5">
            <p class="ui-section-title">{t("Pistas del episodio")}</p>
            {#if episode}
              <p class="mt-1 truncate text-[12px] text-slate-200">{episode.title}</p>
              <dl class="mt-2 space-y-1 text-[11px]">
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
                {#each episode.audio as track, index (index)}
                  <li class="flex items-center justify-between gap-2 rounded border border-slate-800 bg-slate-950/50 px-1.5 py-1 text-[11px]">
                    <span class="truncate text-amber-200/80">A{index + 1} · {track.label}</span>
                    <span class="font-mono text-slate-500">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                  </li>
                {/each}
                {#each episode.subtitles as track, index (index)}
                  <li class="flex items-center justify-between gap-2 rounded border border-slate-800 bg-slate-950/50 px-1.5 py-1 text-[11px]">
                    <span class="truncate text-violet-200/80">S{index + 1} · {track.label}</span>
                    <span class="font-mono text-slate-500">{track.delayMs > 0 ? "+" : ""}{track.delayMs} ms</span>
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="mt-1 text-[11px] text-slate-500">{t("Selecciona un episodio.")}</p>
            {/if}
          </div>
        {:else if projectStore.studioMode === "motion"}
          <div class="rounded-lg border border-slate-800 p-2.5">
            <p class="ui-section-title">{t("Menú animado")}</p>
            {#if menu}
              <p class="mt-1 truncate text-[12px] text-slate-200">{menu.name}</p>
              <dl class="mt-2 space-y-1 text-[11px]">
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
              <p class="mt-1 text-[11px] text-slate-500">{t("Sin menú activo.")}</p>
            {/if}
          </div>
        {:else}
          <div class="rounded-lg border border-slate-800 p-2.5">
            <p class="ui-section-title">{t("Calidad VBR")}</p>
            {#if projectStore.plan}
              <dl class="mt-2 space-y-1 text-[11px]">
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Promedio")}</dt>
                  <dd class="font-mono text-cyan-300">{projectStore.plan.videoKbps.toLocaleString(i18n.locale)} kbps</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Llenado")}</dt>
                  <dd class="font-mono text-slate-300">{formatPercent(projectStore.planPercent)}</dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Viable")}</dt>
                  <dd class={projectStore.plan.feasible ? "text-emerald-300" : "text-rose-300"}>
                    {projectStore.plan.feasible ? t("Sí") : t("No")}
                  </dd>
                </div>
                <div class="flex justify-between gap-2">
                  <dt class="text-slate-500">{t("Advertencias")}</dt>
                  <dd class="text-slate-300">{projectStore.plan.warnings.filter((warning) => warning.severity !== "info").length}</dd>
                </div>
              </dl>
            {:else}
              <p class="mt-1 text-[11px] text-slate-500">{t("Calcula el plan para ver la curva.")}</p>
            {/if}
            {#if episode}
              <p class="ui-section-title mt-2">{t("Prioridad del episodio")}</p>
              <div class="mt-1 flex rounded-md border border-slate-800 bg-slate-950 p-0.5">
                {#each [
                  { id: "low" as const, label: "Baja" },
                  { id: "medium" as const, label: "Media" },
                  { id: "high" as const, label: "Alta" },
                ] as priority}
                  <button
                    type="button"
                    onclick={() => projectStore.setEpisodePriority(episode.id, priority.id)}
                    class={`flex-1 rounded px-1 py-1 text-[11px] font-semibold transition ${
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
            <label class="flex items-center justify-between rounded-lg border border-slate-800 px-2.5 py-1.5 text-[11px] text-slate-400">
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

        <div class="rounded-lg border border-slate-800 p-2.5">
          <p class="ui-section-title">{t("Proyecto")}</p>
          <dl class="mt-1.5 space-y-1 text-[11px]">
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
                {projectStore.plan ? formatPercent(projectStore.planPercent) : t("sin calcular")}
              </dd>
            </div>
            <div class="flex justify-between gap-2">
              <dt class="text-slate-500">{t("Tamaño de los archivos")}</dt>
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

<style>
  /* Every block of a property section is separated by a hairline. */
  :global(.inspector-sections > * + *) {
    border-top: 1px solid var(--color-slate-800);
    padding-top: 0.875rem;
  }
</style>
