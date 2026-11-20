import { ProjectHistory } from "./history.svelte";
import {
  commands,
  type AspectRatio,
  type AuthoringMode,
  type AvchdResolution,
  type BitratePlan_Serialize,
  type BurnProgress,
  type BurnReport,
  type AudioFormat,
  type ButtonShapeMask,
  type ButtonStyleKind,
  type Chapter_Serialize,
  type EncodingPriority,
  type EpisodeAudio_Serialize,
  type Episode_Serialize,
  type FrameRange,
  type FrameRate,
  type JobEvent,
  type JobStage,
  type JobHistoryEntry_Serialize,
  type LayerBreakMode,
  type MediaAsset_Serialize,
  type MediaKind,
  type MediaProbe_Serialize,
  type VideoFilters_Serialize,
  type MenuButton_Serialize,
  type MenuTemplateId,
  type PlayerPreset,
  type Menu_Serialize,
  type MotionMenuConfig_Serialize,
  type OpticalDevice,
  type Project_Serialize,
  type RecentProject_Serialize,
  type SubtitleStyle_Serialize,
  type SubtitleTrack_Serialize,
  type ToolchainReport,
  type VerifyReport_Serialize,
  type TvStandard,
  type VideoScaleMode,
} from "../api/bindings";
import { messageOf, unwrap } from "../api/client";
import { hasGap, normalizeRect } from "../editor/exportMenu";
import {
  localizeDiagnostic,
  localizeJobStage,
  localizeJobSummary,
  localizeLogLine,
  localizeMessage,
  localizeRenderError,
} from "../i18n/diagnostics";
import { i18n, t } from "../i18n/index.svelte";
import { normalizeLanguageCode } from "../i18n/languages";
import { hexToRgb, rgbToHex } from "../utils/color";
import { isPlaceholderAsset } from "../utils/assets";
import { thumbnailKey, thumbnailTimeMs } from "../editor/thumbnailKey";

export type StudioMode =
  | "menu"
  | "trim"
  | "simulator"
  | "vmMap"
  | "tracks"
  | "motion"
  | "vbr";
export type PreviewState = "normal" | "highlight" | "select";
export type MediaTab = "media" | "layouts" | "styles";
export type LayoutKind = "grid2" | "grid3" | "list" | "direct" | "clear";

const NTSC_RATE: FrameRate = { num: 30_000, den: 1_001 };
const PAL_RATE: FrameRate = { num: 25, den: 1 };

function defaultSubtitleStyle(): SubtitleStyle_Serialize {
  return {
    fontFamily: "sans-serif",
    fontPath: null,
    fontWeight: 400,
    italic: false,
    fontSizePercent: 4.5,
    outlinePercent: 0.18,
    shadowOffsetPercent: 0.12,
    fillColor: "#FFFFFF",
    outlineColor: "#000000",
    shadowColor: "#000000A0",
    alignment: "bottomCenter",
    safeAreaPercent: 5,
    maxLines: 3,
  };
}
const AVCHD_BUDGET = {
  targetFillPpm: 990_000,
  headerMarginPpm: 30_000,
  muxOverheadPpm: 50_000,
  navOverheadPpm: 10_000,
  fixedReserveBytes: 0,
  minVideoKbps: 1_000,
  maxVideoKbps: 24_000,
  defaultAc3Kbps: 192,
};
/** Concurrent ffmpeg frame extractions for UI thumbnails. */
const THUMBNAIL_CONCURRENCY = 3;
const DVD_BUDGET = {
  targetFillPpm: 990_000,
  headerMarginPpm: 30_000,
  muxOverheadPpm: 25_000,
  navOverheadPpm: 10_000,
  fixedReserveBytes: 0,
  minVideoKbps: 2_500,
  maxVideoKbps: 9_800,
  defaultAc3Kbps: 192,
};

function even(value: number): number {
  return Math.round(value / 2) * 2;
}

function canvasFor(standard: TvStandard, aspect: AspectRatio) {
  return {
    width: 720,
    height: standard === "pal" ? 576 : 480,
    tvStandard: standard,
    aspect,
    safeMarginPx: 36,
    gridPx: 8,
  };
}

/**
 * Menu canvas for the project's disc. HD discs (AVCHD/Blu-ray 1080/720) edit
 * menus at the final resolution so HD artwork is never squeezed into the
 * 720x480 DVD canvas.
 */
function canvasForProject(project: Project_Serialize) {
  const video = project.disc.video;
  if (project.disc.mode === "avchd" && video.kind === "avchd") {
    if (video.resolution === "1080p" || video.resolution === "1080i") {
      return { width: 1920, height: 1080, tvStandard: project.disc.standard, aspect: "sixteenNine" as AspectRatio, safeMarginPx: 96, gridPx: 8 };
    }
    if (video.resolution === "720p") {
      return { width: 1280, height: 720, tvStandard: project.disc.standard, aspect: "sixteenNine" as AspectRatio, safeMarginPx: 64, gridPx: 8 };
    }
  }
  return canvasFor(project.disc.standard, project.disc.aspect);
}

/** Loudness every track is levelled to by "even volume" (kinodisc-domain default). */
const EVEN_VOLUME = { targetLufs: -16, truePeakDb: -1.5, lra: 11 };

/** Slice of the whole-job progress bar each production stage fills (%). */
const STAGE_SPAN: Record<JobStage, [number, number]> = {
  probe: [0, 3],
  plan: [3, 5],
  encodeIntro: [5, 8],
  encodeEpisodes: [8, 84],
  encodeMenus: [84, 88],
  author: [88, 94],
  iso: [94, 99],
  verify: [99, 100],
  burn: [0, 100],
};

/**
 * Disc volume label for a project name, as the backend makes it for a new
 * project: ASCII capitals and digits, accents reduced to the base letter,
 * other runs of characters as one "_", at most 32 characters.
 */
export function volumeLabelFor(name: string): string {
  const base = name
    .replace(/ß/g, "ss")
    .replace(/[æÆ]/g, "ae")
    .replace(/[œŒ]/g, "oe")
    .replace(/[øØ]/g, "o")
    .replace(/[łŁ]/g, "l")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toUpperCase()
    .replace(/[^A-Z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return (base || "KINODISC").slice(0, 32).replace(/_+$/, "");
}

/** Rate that chapter and trim frames are counted in: the disc's own rate. */
function discFps(project: Project_Serialize): number {
  const video = project.disc.video;
  if (video.kind === "avchd") return video.frameRate.num / video.frameRate.den;
  return project.disc.standard === "pal" ? 25 : 30_000 / 1_001;
}

/**
 * Chapters and trims are stored in disc-rate frames. When the disc rate
 * changes (PAL/NTSC, DVD/HD), recount them so they keep the same time.
 */
function retimeDiscFrames(project: Project_Serialize, fromFps: number): void {
  const toFps = discFps(project);
  if (!(fromFps > 0) || Math.abs(fromFps - toFps) < 1e-6) return;
  const recount = (frame: number) => Math.round((frame * toFps) / fromFps);
  const recountRange = (range: FrameRange | null | undefined) =>
    range ? { start: recount(range.start), end: recount(range.end) } : range ?? null;
  for (const episode of project.episodes) {
    for (const chapter of episode.chapters) chapter.atFrame = recount(chapter.atFrame);
    episode.trim = recountRange(episode.trim);
  }
  if (project.firstPlay) project.firstPlay.trim = recountRange(project.firstPlay.trim);
}


/**
 * The Rust core names generated content in Spanish ("Menú principal", the
 * template "‹ Anterior"/"Siguiente ›" buttons). Those names end up on the
 * disc, so they follow the interface language. Only untouched default names
 * are renamed; anything the user typed is kept. Nav buttons keep their ‹/›
 * arrows, which is how the template engine recognises them.
 */
/** Names the Rust templates give their menus (see kinodisc-menu templates.rs). */
const TEMPLATE_MENU_NAMES = new Set([
  "Cinema Dark",
  "Multi-Page TV Series Grid",
  "Classic Editorial",
  "Retro CRT",
  "Minimalist",
  "Boda",
  "Cumpleaños",
  "Viaje",
  "Deportes",
  "Vacaciones",
  "Cine antiguo",
  "Naturaleza",
  "Documental",
  "Infantil",
  "Elegante",
]);

function localizeGeneratedMenus(menus: Menu_Serialize[]): void {
  // Spanish too: the first templates have English names in the engine.
  const defaults: Record<string, string> = {
    "Menú principal": t("Menú principal"),
    "‹ Anterior": t("‹ Anterior"),
    "Siguiente ›": t("Siguiente ›"),
    "Volver": t("Volver"),
    "Escenas": t("Escenas"),
    "Ajustes": t("Ajustes"),
    "Sin subtítulos": t("Sin subtítulos"),
    "Inicio": t("Inicio"),
  };
  const localize = (name: string): string => {
    if (defaults[name]) return defaults[name];
    // Generated scene pages and buttons: "Escenas · <episode>", "Escena 3".
    if (name.startsWith("Escenas · ")) return t("Escenas") + name.slice("Escenas".length);
    // Setup page buttons: "Audio: Español", "Subtítulos: Español".
    const audio = /^Audio: (.+)$/.exec(name);
    if (audio) return `${t("Audio")}: ${audio[1]}`;
    const subtitle = /^Subtítulos: (.+)$/.exec(name);
    if (subtitle) return `${t("Subtítulos")}: ${subtitle[1]}`;
    const scene = /^Escena (\d+)$/.exec(name);
    if (scene) return t("Escena {n}", { n: scene[1] });
    // Template pages ("Elegante · 2"). The engine finds its pages by their
    // template field, never by name, so renaming them is safe. "Extras" is
    // matched by name in Rust and must stay as it is.
    const page = /^(.+?)( · \d+)?$/.exec(name);
    if (page && TEMPLATE_MENU_NAMES.has(page[1])) return t(page[1]) + (page[2] ?? "");
    return name;
  };
  for (const menu of menus) {
    menu.name = localize(menu.name);
    for (const button of menu.buttons) {
      button.name = localize(button.name);
      // The words drawn on generated buttons ("‹ Anterior", "Escena 3").
      if (button.label) button.label.text = localize(button.label.text);
    }
    // Titles drawn on generated pages ("Escenas · <episode>") are burned
    // into the menu, so they follow the interface language too.
    for (const item of menu.items) {
      if (item.kind === "text" && item.id.startsWith("tpl:")) item.text = localize(item.text);
    }
  }
}

function readFlag(key: string): boolean {
  try {
    return typeof window !== "undefined" && window.localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

/** Size factor of a menu canvas relative to the 720x480 DVD reference. */
function menuScale(menu: Menu_Serialize): number {
  return Math.max(1, menu.canvas.height / 480);
}

function freshId(value: unknown): string {
  return typeof value === "string" && value.trim().length > 0 ? value : crypto.randomUUID();
}

function optionalRef(value: unknown, valid: Set<string>): string | null {
  return typeof value === "string" && valid.has(value) ? value : null;
}

/**
 * Index of the one track that should be the default: the first one the
 * source flags as default, else the first track (`fallback`), else none.
 */
function singleDefaultIndex(flags: boolean[], fallback: boolean): number {
  const flagged = flags.indexOf(true);
  if (flagged >= 0) return flagged;
  return fallback && flags.length > 0 ? 0 : -1;
}

/**
 * Repairs projects produced by older builds or by partially edited UI state.
 * Required IDs are regenerated and dangling optional references become null,
 * so a single stale field can never abort a Tauri command.
 */
function normalizeProject(project: Project_Serialize): void {
  project.id = freshId(project.id);
  for (const asset of project.assets) {
    asset.id = freshId(asset.id);
  }
  const assetIds = new Set(project.assets.map((asset) => asset.id));
  const firstImage = project.assets.find((asset) => asset.kind === "image")?.id ?? null;
  const firstVideo = project.assets.find((asset) => asset.kind === "video")?.id ?? null;

  for (const episode of project.episodes) {
    episode.id = freshId(episode.id);
    if (!episode.video || !assetIds.has(episode.video)) {
      if (firstVideo) episode.video = firstVideo;
    }
    for (const chapter of episode.chapters) {
      chapter.id = freshId(chapter.id);
      chapter.thumbnail = optionalRef(chapter.thumbnail, assetIds);
    }
    if (episode.thumbnailPath !== null && !episode.thumbnailPath.trim()) {
      episode.thumbnailPath = null;
    }
    for (const track of episode.audio) {
      if (track.sourcePath !== null && !track.sourcePath.trim()) track.sourcePath = null;
      track.language = normalizeLanguageCode(track.language);
    }
    for (const track of episode.subtitles) {
      if (track.sourcePath !== null && !track.sourcePath.trim()) track.sourcePath = null;
      track.language = normalizeLanguageCode(track.language);
    }
    // Older builds could mark several tracks as the default; keep the first.
    const audioDefault = singleDefaultIndex(episode.audio.map((track) => track.default), true);
    episode.audio.forEach((track, index) => (track.default = index === audioDefault));
    const subtitleDefault = singleDefaultIndex(
      episode.subtitles.map((track) => track.isDefault),
      false,
    );
    episode.subtitles.forEach((track, index) => (track.isDefault = index === subtitleDefault));
  }
  const episodeIds = new Set(project.episodes.map((episode) => episode.id));
  const firstEpisode = project.episodes[0]?.id ?? null;

  for (const menu of project.menus) {
    menu.id = freshId(menu.id);
  }
  const menuIds = new Set(project.menus.map((menu) => menu.id));
  if (!project.rootMenu || !menuIds.has(project.rootMenu)) {
    project.rootMenu = project.menus[0]?.id ?? freshId(project.rootMenu);
  }

  if (project.firstPlay) {
    const firstPlay = project.firstPlay;
    if (!firstPlay.asset || !assetIds.has(firstPlay.asset)) {
      project.firstPlay = null;
    } else {
      firstPlay.targetMenu = menuIds.has(firstPlay.targetMenu)
        ? firstPlay.targetMenu
        : project.rootMenu;
      if (firstPlay.audio) {
        firstPlay.audio.language = normalizeLanguageCode(firstPlay.audio.language);
      }
    }
  }
  project.playAll.order = project.playAll.order.filter((id) => episodeIds.has(id));

  for (const menu of project.menus) {
    if (menu.background.kind === "static") {
      if (!assetIds.has(menu.background.asset)) {
        if (firstImage) menu.background.asset = firstImage;
      }
    } else if (!assetIds.has(menu.background.asset)) {
      if (firstVideo) menu.background.asset = firstVideo;
    }
    if (menu.audio && !assetIds.has(menu.audio.asset)) {
      menu.audio = null;
    }
    for (const button of menu.buttons) {
      button.id = freshId(button.id);
      if (button.thumbnail && !assetIds.has(button.thumbnail.asset)) {
        button.thumbnail = null;
      }
    }
    const buttonIds = new Set(menu.buttons.map((button) => button.id));
    for (const button of menu.buttons) {
      button.navOverride = {
        up: optionalRef(button.navOverride.up, buttonIds),
        down: optionalRef(button.navOverride.down, buttonIds),
        left: optionalRef(button.navOverride.left, buttonIds),
        right: optionalRef(button.navOverride.right, buttonIds),
      };
      switch (button.action.kind) {
        case "playEpisode":
          if (!episodeIds.has(button.action.episode)) {
            button.action = firstEpisode
              ? { kind: "playEpisode", episode: firstEpisode }
              : { kind: "playAll" };
          }
          break;
        case "jumpTitleChapter": {
          const action = button.action;
          const target = project.episodes.find(
            (episode) => episode.id === action.episode,
          );
          if (!target) {
            const chapterEpisode = project.episodes.find(
              (episode) => episode.chapters.length > 0,
            );
            button.action = chapterEpisode
              ? { kind: "jumpTitleChapter", episode: chapterEpisode.id, chapter: 0 }
              : firstEpisode
                ? { kind: "playEpisode", episode: firstEpisode }
                : { kind: "playAll" };
          } else if (target.chapters.length === 0) {
            // Un salto al primer capítulo de un título sin capítulos es
            // inválido; reproducir el episodio desde el inicio es la
            // intención más cercana.
            button.action = { kind: "playEpisode", episode: target.id };
          } else {
            button.action = {
              ...action,
              chapter: Math.min(action.chapter, target.chapters.length - 1),
            };
          }
          break;
        }
        case "gotoMenu":
          if (!menuIds.has(button.action.menu)) {
            button.action = { kind: "gotoMenu", menu: project.rootMenu };
          }
          break;
        default:
          break;
      }
    }
  }
}

/** Neutral picture fixes (`VideoFilters::default()` in kinodisc-domain). */
export function defaultVideoFilters(): VideoFilters_Serialize {
  return {
    rotate: "none",
    flipHorizontal: false,
    crop: null,
    brightness: 0,
    contrast: 0,
    saturation: 0,
    denoise: "off",
    deinterlace: "auto",
    sideFill: "bars",
  };
}

export function filtersAreNeutral(filters: VideoFilters_Serialize | undefined | null): boolean {
  if (!filters) return true;
  const crop = filters.crop;
  return (
    filters.rotate === "none" &&
    !filters.flipHorizontal &&
    (!crop || (crop.left === 0 && crop.top === 0 && crop.right === 0 && crop.bottom === 0)) &&
    filters.brightness === 0 &&
    filters.contrast === 0 &&
    filters.saturation === 0 &&
    filters.denoise === "off" &&
    filters.deinterlace === "auto" &&
    filters.sideFill === "bars"
  );
}

export function guessMediaKind(path: string, probe: MediaProbe_Serialize): MediaKind {
  if (probe.video) return "video";
  if (probe.audio.length > 0) return "audio";
  const lower = path.toLowerCase();
  if (/\.(srt|ass|ssa|vtt)$/.test(lower)) return "subtitle";
  return "image";
}

class ProjectStore {
  project = $state<Project_Serialize | null>(null);
  /** Undo/redo and unsaved-changes tracking. */
  readonly history = new ProjectHistory();
  projectPath = $state<string | null>(null);
  recentProjects = $state<RecentProject_Serialize[]>([]);
  recentProjectsBusy = $state(false);
  recentProjectsError = $state<string | null>(null);
  jobHistory = $state<JobHistoryEntry_Serialize[]>([]);
  jobHistoryBusy = $state(false);
  jobHistoryError = $state<string | null>(null);
  toolsReport = $state<ToolchainReport | null>(null);
  toolsBusy = $state(false);
  toolsError = $state<string | null>(null);
  plan = $state<BitratePlan_Serialize | null>(null);
  planError = $state<string | null>(null);
  planBusy = $state(false);
  devices = $state<OpticalDevice[]>([]);
  busy = $state(false);
  error = $state<string | null>(null);
  templateDiagnostics = $state<string[]>([]);

  menuBackgroundColor = $state("#0b1220");
  previewUrls = $state<Record<string, string>>({});
  /** Assets whose file is gone (moved, deleted, disconnected drive). */
  missingAssetIds = $state<string[]>([]);
  framePaths = $state<Record<string, string>>({});
  selectedButtonId = $state<string | null>(null);
  selectedMenuId = $state<string | null>(null);
  selectedEpisodeId = $state<string | null>(null);
  studioMode = $state<StudioMode>("menu");
  previewState = $state<PreviewState>("normal");
  showSafeAreas = $state(true);
  showNavigation = $state(true);
  mediaTab = $state<MediaTab>("media");
  productionOpen = $state(false);
  /** Guided tour panel. Opens automatically on first use. */
  guideOpen = $state(false);
  /**
   * Simple mode (default) hides the technical studios (VM map, motion, VBR)
   * so first-time users only see the everyday steps.
   */
  expertMode = $state(readFlag("kinodisc_expert_mode"));
  leftCollapsed = $state(false);
  rightCollapsed = $state(false);

  lastIso = $state<string | null>(null);
  /** Stage of the running job and the progress each part of it reported. */
  private stageKey: JobStage | null = null;
  private stageFractions: Partial<Record<JobStage, number>> = {};
  rendering = $state(false);
  renderStage = $state("");
  renderPercent = $state(0);
  renderLogs = $state<string[]>([]);
  renderError = $state<string | null>(null);
  renderSummary = $state<string | null>(null);
  renderVerification = $state<VerifyReport_Serialize | null>(null);
  burnProgress = $state<BurnProgress | null>(null);
  burnReport = $state<BurnReport | null>(null);
  burning = $state(false);
  burnError = $state<string | null>(null);

  private planSequence = 0;
  private thumbnailJobs = new Map<string, Promise<void>>();
  private thumbnailSlots = 0;
  private readonly thumbnailWaiters: Array<() => void> = [];

  get hasProject(): boolean {
    return this.project !== null;
  }

  get activeMenu(): Menu_Serialize | null {
    const project = this.project;
    if (!project) return null;
    return (
      project.menus.find((menu) => menu.id === this.selectedMenuId) ??
      project.menus[0] ??
      null
    );
  }

  selectedItemId = $state<string | null>(null);

  get selectedItem(): Menu_Serialize["items"][number] | null {
    const menu = this.activeMenu;
    return menu?.items.find((item) => item.id === this.selectedItemId) ?? null;
  }

  selectItem(id: string | null): void {
    this.selectedItemId = id;
    if (id) this.selectedButtonId = null;
  }

  /** Adds a text object centred on the active menu and selects it. */
  addTextItem(): void {
    const menu = this.activeMenu;
    if (!menu) return;
    const scale = menuScale(menu);
    const w = Math.round((menu.canvas.width * 0.6) / 2) * 2;
    const h = Math.round((56 * scale) / 2) * 2;
    const id = `text:${crypto.randomUUID()}`;
    // First horizontal band (top to bottom) that touches no button or item.
    // (The template background and decorative shapes are not "taken": they
    // covered the whole menu, so the text fell back behind the buttons.)
    const taken = [
      ...menu.buttons.map((button) => button.rect),
      ...menu.items.filter((item) => item.kind === "text" || item.kind === "image").map((item) => item.rect),
    ].map((rect) => ({ y: rect.y ?? 0, h: rect.h ?? 0 }));
    // Free vertical gaps between what is already on the menu; the text
    // takes the first gap that fits (shrinking to it if needed).
    const margin = Math.round(menu.canvas.height * 0.06);
    const blocks = taken
      .map((rect) => [rect.y, rect.y + rect.h] as const)
      .sort((a, b) => a[0] - b[0]);
    let cursor = margin;
    let y = Math.round((menu.canvas.height * 0.12) / 2) * 2;
    let height = h;
    for (const [top, bottom] of [...blocks, [menu.canvas.height - margin, menu.canvas.height] as const]) {
      const gap = top - cursor - 8;
      if (gap >= 28) {
        height = Math.min(h, Math.floor(gap / 2) * 2);
        y = Math.round((cursor + 4) / 2) * 2;
        break;
      }
      cursor = Math.max(cursor, bottom);
    }
    menu.items = [
      ...menu.items,
      {
        kind: "text",
        id,
        text: t("Nuevo texto"),
        rect: {
          x: Math.round((menu.canvas.width - w) / 4) * 2,
          y,
          w,
          h: height,
        },
        fontFamily: "Segoe UI",
        fontSize: Math.round(Math.min(36 * scale, height * 0.7)),
        color: "#ffffff",
        align: "center",
      },
    ];
    this.selectItem(id);
    this.touch();
  }

  updateItem(id: string, patch: Record<string, unknown>): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.items = menu.items.map((item) =>
      item.id === id ? ({ ...item, ...patch } as typeof item) : item,
    );
    this.touch();
  }

  deleteSelectedItem(): void {
    const menu = this.activeMenu;
    if (!menu || !this.selectedItemId) return;
    menu.items = menu.items.filter((item) => item.id !== this.selectedItemId);
    this.selectedItemId = null;
    this.touch();
  }

  get selectedButton(): MenuButton_Serialize | null {
    const menu = this.activeMenu;
    if (!menu) return null;
    return menu.buttons.find((button) => button.id === this.selectedButtonId) ?? null;
  }

  get selectedEpisode(): Episode_Serialize | null {
    const project = this.project;
    if (!project) return null;
    return (
      project.episodes.find((episode) => episode.id === this.selectedEpisodeId) ??
      project.episodes[0] ??
      null
    );
  }

  get episodes(): Episode_Serialize[] {
    return this.project?.episodes ?? [];
  }

  get assets(): MediaAsset_Serialize[] {
    return this.project?.assets ?? [];
  }

  get backgroundPreviewUrl(): string | null {
    const menu = this.activeMenu;
    if (!menu) return null;
    return this.previewUrls[menu.background.asset] ?? null;
  }

  get thumbnailPreviewUrls(): Record<string, string> {
    return this.previewUrls;
  }

  get frameRate(): number {
    const project = this.project;
    if (!project) return 30000 / 1001;
    if (project.disc.video.kind === "avchd") {
      const rate = project.disc.video.frameRate;
      return rate.num / rate.den;
    }
    return project.disc.standard === "pal" ? 25 : 30000 / 1001;
  }

  get planPercent(): number {
    return this.plan ? Math.min(100, this.plan.fillPpm / 10_000) : 0;
  }

  get planUsedBytes(): number {
    if (!this.plan) return 0;
    return Math.round((this.plan.capacityBytes * this.plan.fillPpm) / 1_000_000);
  }

  get qualityLabel(): string {
    const plan = this.plan;
    if (!plan) return t("Sin plan");
    if (!plan.feasible) return t("No cabe");
    if (plan.mode === "avchd") {
      return plan.avchd
        ? `${plan.avchd.resolution}${plan.avchd.resolution.startsWith("1080") || plan.avchd.resolution === "720p" ? ` ${t("HD")}` : ""}`
        : t("Alta definición");
    }
    if (plan.profile === "halfD1") return t("Extendida Half-D1");
    if (plan.videoKbps >= 7_000) return t("Excelente");
    if (plan.videoKbps >= 4_500) return t("Buena");
    return t("Básica");
  }

  setProject(project: Project_Serialize): void {
    this.missingAssetIds = [];
    normalizeProject(project);
    localizeGeneratedMenus(project.menus);
    // Projects created by older builds kept HD menus on a 720x480 canvas, which
    // squeezed 1920x1080 backgrounds. Upgrade them to the disc resolution.
    const target = canvasForProject(project);
    for (const menu of project.menus) {
      if (menu.canvas.width !== target.width || menu.canvas.height !== target.height) {
        this.rescaleMenu(menu, target);
      }
    }
    this.project = project;
    // Results, errors and notices of another project do not carry over.
    if (!this.rendering) {
      this.resetRender();
      this.lastIso = null;
    }
    this.syncPlayAll();
    this.history.reset(project);
    this.planSequence += 1;
    this.plan = null;
    this.planError = null;
    this.planBusy = false;
    this.error = null;
    this.selectedMenuId = project.menus[0]?.id ?? null;
    this.selectedEpisodeId = project.episodes[0]?.id ?? null;
    this.selectedButtonId = project.menus[0]?.buttons[0]?.id ?? null;
    this.studioMode = project.menus.length > 0 && project.episodes.length > 0 ? "menu" : "trim";
    this.previewState = "normal";
    this.previewUrls = {};
    this.framePaths = {};
    void this.hydrateProjectPreviews();
  }

  setExpertMode(enabled: boolean): void {
    this.expertMode = enabled;
    try {
      window.localStorage.setItem("kinodisc_expert_mode", enabled ? "1" : "0");
    } catch {
      // Best effort.
    }
    if (!enabled && ["vmMap", "motion", "vbr"].includes(this.studioMode)) {
      this.studioMode = "menu";
    }
  }

  /** True until the user finishes or dismisses the guided tour once. */
  get guidePending(): boolean {
    return !readFlag("kinodisc_guide_done");
  }

  clearProject(): void {
    this.project = null;
    this.missingAssetIds = [];
    this.history.reset(null);
    this.projectPath = null;
    this.plan = null;
    this.planError = null;
    this.error = null;
    this.selectedButtonId = null;
    this.selectedMenuId = null;
    this.selectedEpisodeId = null;
    this.lastIso = null;
    this.previewUrls = {};
    this.framePaths = {};
    this.planSequence += 1;
    this.planBusy = false;
    this.busy = false;
    this.productionOpen = false;
    this.resetRender();
  }

  touch(): void {
    if (!this.project) return;
    this.project.modifiedAt = new Date().toISOString();
    this.history.schedule(() => this.project);
  }

  get canUndo(): boolean {
    return this.history.canUndo;
  }

  get canRedo(): boolean {
    return this.history.canRedo;
  }

  /** True when the project differs from the last saved/opened version. */
  get dirty(): boolean {
    return this.history.dirty;
  }

  undo(): void {
    this.restoreSnapshot(this.history.undo());
  }

  redo(): void {
    this.restoreSnapshot(this.history.redo());
  }

  private restoreSnapshot(snapshot: string | null): void {
    if (!snapshot || !this.project) return;
    const restored = JSON.parse(snapshot) as Project_Serialize;
    restored.modifiedAt = new Date().toISOString();
    this.project = restored;
    // Keep selections that still exist; fall back to the first item.
    if (!restored.menus.some((menu) => menu.id === this.selectedMenuId)) {
      this.selectedMenuId = restored.menus[0]?.id ?? null;
    }
    const menu = restored.menus.find((item) => item.id === this.selectedMenuId);
    if (!menu?.buttons.some((button) => button.id === this.selectedButtonId)) {
      this.selectedButtonId = null;
    }
    if (!restored.episodes.some((episode) => episode.id === this.selectedEpisodeId)) {
      this.selectedEpisodeId = restored.episodes[0]?.id ?? null;
    }
    this.plan = null;
  }

  /** A label typed by the user (made safe: ASCII capitals, digits, "_"). */
  setVolumeLabel(label: string): void {
    if (!this.project) return;
    this.project.disc.volumeLabel = volumeLabelFor(label.trim() ? label : this.project.name);
    this.touch();
  }

  renameProject(name: string): void {
    if (!this.project) return;
    // The disc label follows the name unless the user set their own.
    const followsName =
      this.project.disc.volumeLabel === volumeLabelFor(this.project.name) ||
      this.project.disc.volumeLabel === "KINODISC";
    this.project.name = name;
    if (followsName) this.project.disc.volumeLabel = volumeLabelFor(name);
    this.touch();
  }

  selectMenu(menuId: string): void {
    this.selectedMenuId = menuId;
    this.studioMode = "menu";
    const menu = this.project?.menus.find((item) => item.id === menuId);
    this.selectedButtonId = menu?.buttons[0]?.id ?? null;
  }

  selectEpisode(episodeId: string): void {
    this.selectedEpisodeId = episodeId;
    this.studioMode = "trim";
    this.previewState = "normal";
  }

  assetFor(id: string): MediaAsset_Serialize | null {
    return this.project?.assets.find((asset) => asset.id === id) ?? null;
  }

  assetPath(asset: MediaAsset_Serialize): string | null {
    return asset.source.kind === "external"
      ? asset.source.absolutePath
      : (asset.relativePath ?? null);
  }

  previewForAsset(assetId: string | null | undefined): string | null {
    return assetId ? (this.previewUrls[assetId] ?? null) : null;
  }

  episodeSourcePath(episode: Episode_Serialize): string | null {
    const asset = this.assetFor(episode.video);
    return asset ? this.assetPath(asset) : null;
  }

  episodeSourceDuration(episode: Episode_Serialize): number {
    return this.assetFor(episode.video)?.probe.durationMs ?? 0;
  }

  episodeEffectiveDuration(episode: Episode_Serialize): number {
    const source = this.episodeSourceDuration(episode);
    const start = episode.trimStartMs ?? 0;
    const end = episode.trimEndMs ?? source;
    return Math.max(0, end - start);
  }

  episodePreview(episode: Episode_Serialize): string | null {
    return this.previewUrls[episode.video] ?? null;
  }

  /** Huella reactiva de todas las entradas que afectan al plan de bitrate. */
  get planSignature(): string {
    const project = this.project;
    if (!project) return "";
    return JSON.stringify({
      id: project.id,
      disc: {
        capacity: project.disc.capacity,
        standard: project.disc.standard,
        aspect: project.disc.aspect,
        video: project.disc.video,
        audio: project.disc.audio,
      },
      budget: project.budget,
      assets: project.assets.map((asset) => ({
        id: asset.id,
        kind: asset.kind,
        duration: asset.probe.durationMs,
      })),
      firstPlay: project.firstPlay
        ? {
            asset: project.firstPlay.asset,
            trim: project.firstPlay.trim,
            audio: project.firstPlay.audio,
          }
        : null,
      menus: project.menus.map((menu) => ({
        id: menu.id,
        duration: menu.timing.durationMs,
        audio: Boolean(menu.audio),
        motion: menu.motionConfig,
      })),
      episodes: project.episodes.map((episode) => ({
        id: episode.id,
        video: episode.video,
        trimStart: episode.trimStartMs,
        trimEnd: episode.trimEndMs,
        trim: episode.trim,
        priority: episode.encodingPriority,
        videoMode: episode.videoMode,
        // Picture fixes turn off Blu-ray stream copy, which changes the plan.
        filters: episode.filters,
        chapters: episode.chapters.length,
        audio: episode.audio.map((track) => ({
          stream: track.sourceStream,
          path: track.sourcePath,
          kbps: track.ac3Kbps,
          format: track.format,
          delay: track.delayMs,
          default: track.default,
        })),
        subtitles: episode.subtitles.map((track) => ({
          stream: track.sourceStream,
          path: track.sourcePath,
          delay: track.delayMs,
          forced: track.isForced,
          default: track.isDefault,
        })),
      })),
    });
  }

  episodeForAsset(assetId: string): Episode_Serialize | null {
    return this.project?.episodes.find((episode) => episode.video === assetId) ?? null;
  }

  addAsset(
    path: string,
    probe: MediaProbe_Serialize,
    kind: MediaKind,
    previewUrl: string | null = null,
  ): string {
    if (!this.project) throw new Error(t("No hay proyecto abierto."));
    // Picking a file that is already in the library reuses it instead of
    // adding a second copy (Windows paths compare without case).
    const same = (other: string) =>
      /^[a-z]:[\\/]|^[\\/]{2}/i.test(path) ? other.toLowerCase() === path.toLowerCase() : other === path;
    const existing = this.project.assets.find((asset) => {
      const other = this.assetPath(asset);
      return asset.kind === kind && other !== null && same(other);
    });
    if (existing) {
      existing.probe = probe;
      if (previewUrl) this.previewUrls[existing.id] = previewUrl;
      return existing.id;
    }
    const id = crypto.randomUUID();
    const asset: MediaAsset_Serialize = {
      id,
      displayName: path.split(/[\\/]/).pop() ?? path,
      source: { kind: "external", absolutePath: path },
      relativePath: null,
      sha256: "0".repeat(64),
      bytes: probe.sizeBytes ?? 0,
      kind,
      probe,
      proxy: null,
      importedAt: new Date().toISOString(),
    };
    this.project.assets = [...this.project.assets, asset];
    if (previewUrl) {
      this.previewUrls[id] = previewUrl;
    }
    return id;
  }

  async importMedia(paths: string[]): Promise<number> {
    const project = this.project;
    if (!project || paths.length === 0) return 0;
    this.busy = true;
    this.error = null;
    const failures: string[] = [];
    let imported = 0;
    for (const path of paths) {
      try {
        const probe = await unwrap(commands.mediaProbeFile(path));
        const kind = guessMediaKind(path, probe);
        let preview: string | null = null;
        if (kind === "image") {
          try {
            preview = await unwrap(commands.readImageDataUrl(path));
          } catch {
            preview = null;
          }
        }
        // Opening/creating another project while probes are in flight must not
        // append the rest of this batch to that project.
        if (this.project !== project) break;
        const assetId = this.addAsset(path, probe, kind, preview);
        imported += 1;
        if (kind === "video") {
          // Thumbnails fill in asynchronously (bounded by withThumbnailSlot);
          // the import itself only waits for the probes.
          void this.ensureVideoThumbnail(assetId);
        }
      } catch (cause) {
        // A plain sentence for people; the tool's own message goes to the log.
        const name = path.split(/[\\/]/).pop() ?? path;
        console.warn(`import ${name}: ${messageOf(cause)}`);
        failures.push(
          t("«{name}» no se puede leer: no es un vídeo, una imagen o un audio válido, o está dañado.", { name }),
        );
      }
    }
    this.busy = false;
    if (this.project !== project) return imported;
    if (failures.length > 0) {
      this.error =
        failures.length === 1
          ? failures[0]
          : t("No se pudieron importar {count} archivo(s): {detail}", { count: failures.length, detail: failures[0] });
    }
    this.touch();
    return imported;
  }

  /**
   * Builds a slideshow video from photos (+ optional music) and adds it as
   * a new episode. Resolution and frame rate follow the disc.
   */
  async createSlideshow(options: {
    images: string[];
    secondsPerImage: number;
    crossfadeSeconds: number;
    music: string | null;
    title: string;
  }): Promise<boolean> {
    const project = this.project;
    if (!project) return false;
    const video = project.disc.video;
    const fps =
      video.kind === "avchd"
        ? video.frameRate.num / video.frameRate.den
        : project.disc.standard === "pal"
          ? 25
          : 30000 / 1001;
    this.busy = true;
    this.error = null;
    try {
      const outcome = await unwrap(
        commands.slideshowCreate({
          images: options.images,
          secondsPerImage: options.secondsPerImage,
          crossfadeSeconds: options.crossfadeSeconds,
          music: options.music,
          widescreen: project.disc.aspect === "sixteenNine",
          fps,
        }),
      );
      if (this.project !== project) return false;
      this.busy = false;
      await this.importMedia([outcome.path]);
      const asset = project.assets.find((item) => this.assetPath(item) === outcome.path);
      if (!asset) return false;
      this.nameGeneratedAsset(outcome.path, options.title);
      const episode = this.episodeForAsset(asset.id) ?? this.addEpisodeFromAsset(asset.id);
      if (episode) {
        episode.title = options.title;
        this.selectEpisode(episode.id);
        this.touch();
      }
      return Boolean(episode);
    } catch (cause) {
      this.error = messageOf(cause);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /** Generated videos (slideshows, joins) live in the cache under a hash
   *  name: show the episode title in Media instead. */
  private nameGeneratedAsset(path: string, title: string): void {
    const asset = this.project?.assets.find((item) => this.assetPath(item) === path);
    if (!asset) return;
    asset.displayName = `${title.replace(/[\\/:*?"<>|]/g, "_")}.mp4`;
    this.touch();
  }

  /** Joins clips into one video and adds it as an episode with a chapter
   *  at the start of every clip. */
  async createJoin(clips: string[], title: string): Promise<boolean> {
    const project = this.project;
    if (!project) return false;
    this.busy = true;
    this.error = null;
    try {
      const outcome = await unwrap(
        commands.joinCreate({
          clips,
          widescreen: project.disc.aspect === "sixteenNine",
          fps: this.frameRate,
        }),
      );
      if (this.project !== project) return false;
      this.busy = false;
      const added = await this.addImportedTitles([
        { path: outcome.path, title, chapterStartsMs: outcome.clipStartsMs },
      ]);
      this.nameGeneratedAsset(outcome.path, title);
      return added > 0;
    } catch (cause) {
      this.error = messageOf(cause);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /** Imports copied disc titles as episodes, keeping their chapters. */
  async addImportedTitles(
    titles: Array<{ path: string; title: string; chapterStartsMs: Array<number | null> }>,
  ): Promise<number> {
    const project = this.project;
    if (!project || titles.length === 0) return 0;
    await this.importMedia(titles.map((title) => title.path));
    if (this.project !== project) return 0;
    const video = project.disc.video;
    const fps =
      video.kind === "avchd"
        ? video.frameRate.num / video.frameRate.den
        : project.disc.standard === "pal"
          ? 25
          : 30000 / 1001;
    let added = 0;
    for (const imported of titles) {
      const asset = project.assets.find((item) => this.assetPath(item) === imported.path);
      if (!asset) continue;
      const episode = this.episodeForAsset(asset.id) ?? this.addEpisodeFromAsset(asset.id);
      if (!episode) continue;
      episode.title = imported.title;
      if (imported.chapterStartsMs.length > 1) {
        episode.chapters = imported.chapterStartsMs.map((ms, index) => ({
          // f64 values arrive as number | null (NaN); treat null as 0.

          id: crypto.randomUUID(),
          title: t("Capítulo {n}", { n: index + 1 }),
          atFrame: Math.round(((ms ?? 0) * fps) / 1000),
          thumbnail: null,
        }));
      }
      added += 1;
    }
    this.touch();
    return added;
  }

  async addImageAssetFromPath(path: string): Promise<string | null> {
    const project = this.project;
    if (!project) return null;
    try {
      const probe = await unwrap(commands.mediaProbeFile(path));
      const preview = await unwrap(commands.readImageDataUrl(path));
      if (this.project !== project) return null;
      const assetId = this.addAsset(path, probe, "image", preview);
      this.touch();
      return assetId;
    } catch (cause) {
      this.error = messageOf(cause);
      return null;
    }
  }

  private async withThumbnailSlot<T>(task: () => Promise<T>): Promise<T> {
    if (this.thumbnailSlots >= THUMBNAIL_CONCURRENCY) {
      // The releasing job hands its slot over directly.
      await new Promise<void>((resolve) => this.thumbnailWaiters.push(resolve));
    } else {
      this.thumbnailSlots += 1;
    }
    try {
      return await task();
    } finally {
      const next = this.thumbnailWaiters.shift();
      if (next) next();
      else this.thumbnailSlots -= 1;
    }
  }

  /** Marks the assets whose file is missing; returns how many. */
  async checkMissingAssets(): Promise<number> {
    const project = this.project;
    if (!project) {
      this.missingAssetIds = [];
      return 0;
    }
    const assets = project.assets.filter(
      (asset) => !isPlaceholderAsset(asset) && this.assetPath(asset) !== null,
    );
    try {
      const missing = await unwrap(
        commands.mediaMissing(assets.map((asset) => this.assetPath(asset) ?? "")),
      );
      if (this.project !== project) return 0;
      this.missingAssetIds = assets.filter((_, index) => missing[index]).map((asset) => asset.id);
    } catch {
      this.missingAssetIds = [];
    }
    return this.missingAssetIds.length;
  }

  isAssetMissing(assetId: string): boolean {
    return this.missingAssetIds.includes(assetId);
  }

  /**
   * Points a missing asset at its new location (same id, so episodes,
   * buttons and menus keep working). Other missing files with the same name
   * in the new folder are relinked too, as when a whole folder moved.
   * Returns how many assets were relinked.
   */
  async relinkAsset(assetId: string, newPath: string): Promise<number> {
    const project = this.project;
    const asset = this.assetFor(assetId);
    if (!project || !asset) return 0;
    const probe = await unwrap(commands.mediaProbeFile(newPath));
    if (this.project !== project) return 0;
    const relink = (target: MediaAsset_Serialize, path: string, targetProbe: MediaProbe_Serialize) => {
      target.source = { kind: "external", absolutePath: path };
      target.relativePath = null;
      target.displayName = path.split(/[\\/]/).pop() ?? path;
      target.probe = targetProbe;
      target.bytes = targetProbe.sizeBytes ?? target.bytes;
      delete this.previewUrls[target.id];
    };
    relink(asset, newPath, probe);
    let count = 1;
    const folder = newPath.slice(0, Math.max(newPath.lastIndexOf("/"), newPath.lastIndexOf("\\")) + 1);
    const others = project.assets.filter(
      (other) => other.id !== assetId && this.missingAssetIds.includes(other.id),
    );
    if (folder && others.length > 0) {
      const candidates = others.map((other) => {
        const name = (this.assetPath(other) ?? other.displayName).split(/[\/]/).pop() ?? "";
        return folder + name;
      });
      const absent = await unwrap(commands.mediaMissing(candidates)).catch(() => candidates.map(() => true));
      for (let index = 0; index < others.length; index += 1) {
        if (absent[index]) continue;
        try {
          const otherProbe = await unwrap(commands.mediaProbeFile(candidates[index]));
          relink(others[index], candidates[index], otherProbe);
          count += 1;
        } catch {
          // Same name but unreadable: leave it marked as missing.
        }
      }
    }
    project.assets = [...project.assets];
    this.touch();
    const stillMissing = await this.checkMissingAssets();
    if (stillMissing === 0) this.error = null;
    for (const item of project.assets) {
      if (item.kind === "video" && !this.missingAssetIds.includes(item.id)) {
        void this.ensureVideoThumbnail(item.id);
      }
    }
    return count;
  }

  /** Changes some of an episode's picture fixes (rotate, crop, colour…). */
  updateEpisodeFilters(episodeId: string, patch: Partial<VideoFilters_Serialize>): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    episode.filters = { ...(episode.filters ?? defaultVideoFilters()), ...patch };
    this.touch();
  }

  async ensureVideoThumbnail(assetId: string, timestampMs?: number): Promise<void> {
    if (this.previewUrls[assetId]) return;
    const existing = this.thumbnailJobs.get(assetId);
    if (existing) return existing;
    const asset = this.assetFor(assetId);
    if (!asset || asset.kind !== "video") return;
    const path = this.assetPath(asset);
    if (!path) return;
    const job = (async () => {
      try {
        const duration = asset.probe.durationMs ?? 0;
        const target =
          timestampMs ?? (duration > 0 ? Math.min(duration * 0.1, 10_000) : 1_000);
        const framePath = await this.withThumbnailSlot(async () =>
          // Skip jobs that were queued for a project that is no longer open.
          this.assetFor(assetId)
            ? unwrap(commands.mediaExtractFrame(path, Math.max(0, Math.round(target)), null))
            : null,
        );
        if (!framePath) return;
        const dataUrl = await unwrap(commands.readImageDataUrl(framePath));
        // The project may have been closed or replaced while ffmpeg ran.
        if (!this.assetFor(assetId)) return;
        this.previewUrls[assetId] = dataUrl;
        this.framePaths[assetId] = framePath;
      } catch {
        // La miniatura es opcional: un fallo no debe bloquear la importación.
      } finally {
        this.thumbnailJobs.delete(assetId);
      }
    })();
    this.thumbnailJobs.set(assetId, job);
    return job;
  }

  /**
   * Scene buttons show the frame at their chapter, not the poster: load
   * those frames for the editor the way the export does (keyed by
   * `thumbnailKey`).
   */
  async loadFrameThumbnails(): Promise<void> {
    const project = this.project;
    if (!project) return;
    for (const menu of project.menus) {
      for (const button of menu.buttons) {
        const thumbnail = button.thumbnail;
        const at = thumbnail ? thumbnailTimeMs(thumbnail) : null;
        if (!thumbnail || at === null) continue;
        const key = thumbnailKey(thumbnail);
        if (this.previewUrls[key]) continue;
        const asset = this.assetFor(thumbnail.asset);
        const path = asset && asset.kind === "video" ? this.assetPath(asset) : null;
        if (!path) continue;
        try {
          const frame = await unwrap(commands.mediaExtractFrame(path, at, null));
          const dataUrl = await unwrap(commands.readImageDataUrl(frame));
          if (this.project !== project) return;
          this.previewUrls[key] = dataUrl;
        } catch {
          // The poster frame stays as the fallback.
        }
      }
    }
  }

  async hydrateProjectPreviews(): Promise<void> {
    const project = this.project;
    if (!project) return;
    void this.loadFrameThumbnails();
    for (const asset of project.assets) {
      if (asset.kind === "image") {
        const path = this.assetPath(asset);
        if (!path || this.previewUrls[asset.id]) continue;
        try {
          const dataUrl = await unwrap(commands.readImageDataUrl(path));
          if (this.project !== project) return;
          this.previewUrls[asset.id] = dataUrl;
        } catch {
          // Ignorar assets movidos o inaccesibles.
        }
      }
    }
    for (const episode of project.episodes) {
      if (!episode.thumbnailPath || this.previewUrls[episode.video]) continue;
      try {
        const dataUrl = await unwrap(commands.readImageDataUrl(episode.thumbnailPath));
        if (this.project !== project) return;
        this.previewUrls[episode.video] = dataUrl;
        this.framePaths[episode.video] = episode.thumbnailPath;
      } catch {
        // La miniatura se regenerará bajo demanda.
      }
    }
    if (this.project !== project) return;
    for (const asset of project.assets) {
      if (asset.kind === "video" && !this.previewUrls[asset.id]) {
        void this.ensureVideoThumbnail(asset.id);
      }
    }
  }

  addEpisodeFromAsset(assetId: string): Episode_Serialize | null {
    const project = this.project;
    const asset = project?.assets.find((item) => item.id === assetId);
    if (!project || !asset || asset.kind !== "video") return null;
    const number =
      project.episodes.reduce((highest, episode) => Math.max(highest, episode.number), 0) + 1;
    const title = asset.displayName.replace(/\.[^.]+$/, "") || t("Episodio {number}", { number });
    const sourceChapters = asset.probe.chapters ?? [];
    const importedChapters = sourceChapters
      .map((chapter, index) => ({
        id: crypto.randomUUID(),
        title: chapter.title.trim() || t("Capítulo {number}", { number: index + 1 }),
        atFrame: Math.max(0, Math.round((chapter.startMs / 1000) * this.frameRate)),
        thumbnail: null,
      }))
      .filter((chapter, index, all) =>
        chapter.atFrame >= 0 &&
        all.findIndex((candidate) => Math.abs(candidate.atFrame - chapter.atFrame) < 1) === index,
      );
    const episode: Episode_Serialize = {
      id: crypto.randomUUID(),
      number,
      title,
      video: assetId,
      trim: null,
      trimStartMs: null,
      trimEndMs: null,
      scaleMode: "contain",
      thumbnailTimeMs: null,
      thumbnailPath: this.framePaths[assetId] ?? null,
      chapters: [
        {
          id: crypto.randomUUID(),
          title: t("Inicio"),
          atFrame: 0,
          thumbnail: null,
        },
        ...importedChapters.filter((chapter) => chapter.atFrame > 0),
      ].sort((a, b) => a.atFrame - b.atFrame),
      audio: asset.probe.audio.map((stream, index) => ({
        sourceStream: stream.index,
        sourcePath: null,
        language: stream.language ?? "und",
        label: stream.title?.trim() ||
          (stream.commentary
            ? t("Comentario {number}", { number: index + 1 })
            : t("Pista {number}", { number: index + 1 })),
        gainDb: 0,
        ac3Kbps: 192,
        delayMs: 0,
        format: stream.channels >= 6 ? "ac3Surround51" : "ac3Stereo",
        // The default audio is always the first track (the disc plays it first);
        // the source flags only decide the subtitle default.
        default: index === 0,
        commentary: stream.commentary,
        mastering: { loudness: this.evenVolume ? { ...EVEN_VOLUME } : null, channelMode: "source", codec: null, passthrough: false },
      })),
      subtitles: (asset.probe.subtitleTracks ?? []).map((stream, index, streams) => ({
        sourceStream: stream.index,
        sourcePath: null,
        language: stream.language ?? "und",
        label: stream.title?.trim() || t("Subtítulo {number}", { number: index + 1 }),
        delayMs: 0,
        isForced: stream.forced,
        isDefault:
          index ===
          singleDefaultIndex(
            streams.map((item) => item.default),
            false,
          ),
        bitmap: stream.bitmap,
        style: defaultSubtitleStyle(),
      })),
      encodingPriority: "medium",
      postAction: "returnToMenu",
      encodeOverride: null,
      videoMode: "auto",
      filters: defaultVideoFilters(),
    };
    project.episodes = [...project.episodes, episode];
    this.selectedEpisodeId = episode.id;
    this.syncPlayAll();
    // A menu the user has not designed yet (no template, only episode
    // buttons) gets one button per episode, so every video can be chosen on
    // the disc. A full menu is left alone; validation then warns about it.
    const menu = this.activeMenu;
    if (
      menu &&
      !menu.template &&
      menu.buttons.every((button) => button.action.kind === "playEpisode")
    ) {
      const previousError = this.error;
      if (!this.addButton("thumbnailCard")) this.error = previousError;
    }
    this.touch();
    return episode;
  }

  updateEpisode(episodeId: string, patch: Partial<Episode_Serialize>): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    const previousTitle = episode.title;
    Object.assign(episode, patch);
    if (patch.title !== undefined && patch.title !== previousTitle) {
      this.renameEpisodeButtons(episodeId, previousTitle, episode.title);
    }
    this.touch();
  }

  /**
   * Buttons that play an episode carry its title until the user types another
   * name. Renaming the episode renames those buttons (and their drawn label)
   * too; a button the user renamed by hand keeps its own text.
   */
  private renameEpisodeButtons(episodeId: string, previous: string, next: string): void {
    if (!this.project || !next.trim()) return;
    for (const menu of this.project.menus) {
      for (const button of menu.buttons) {
        if (button.action.kind !== "playEpisode" || button.action.episode !== episodeId) continue;
        if (button.name === previous) button.name = next;
        if (button.label && button.label.text === previous) button.label.text = next;
      }
    }
  }

  setEpisodeTrimStart(episodeId: string, ms: number | null): boolean {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return false;
    const duration = this.episodeSourceDuration(episode);
    const start = ms === null ? null : Math.max(0, Math.min(duration, Math.round(ms)));
    if (start !== null && episode.trimEndMs !== null && start >= episode.trimEndMs) {
      this.error = t("El inicio del recorte debe ser anterior al fin.");
      return false;
    }
    episode.trimStartMs = start;
    this.error = null;
    this.touch();
    return true;
  }

  setEpisodeTrimEnd(episodeId: string, ms: number | null): boolean {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return false;
    const duration = this.episodeSourceDuration(episode);
    const end = ms === null ? null : Math.max(0, Math.min(duration, Math.round(ms)));
    if (end !== null && end <= (episode.trimStartMs ?? 0) + 500) {
      this.error = t("El fin del recorte debe ser al menos 0,5 s posterior al inicio.");
      return false;
    }
    episode.trimEndMs = end;
    this.error = null;
    this.touch();
    return true;
  }

  resetEpisodeTrim(episodeId: string): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    episode.trimStartMs = null;
    episode.trimEndMs = null;
    this.error = null;
    this.touch();
  }

  setEpisodeScaleMode(episodeId: string, mode: VideoScaleMode): void {
    this.updateEpisode(episodeId, { scaleMode: mode });
  }

  setEpisodeThumbnail(episodeId: string, framePath: string, previewUrl: string): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    episode.thumbnailPath = framePath;
    this.previewUrls[episode.video] = previewUrl;
    this.framePaths[episode.video] = framePath;
    // Every menu (template pages included) showing this episode gets the frame.
    for (const menu of this.project?.menus ?? []) {
      for (const button of menu.buttons) {
        if (button.action.kind === "playEpisode" && button.action.episode === episode.id) {
          button.thumbnail = this.episodeThumbnailSpec(episode);
          if (button.styleKind === "textOnly" || button.styleKind === "hotspot") {
            button.styleKind = "thumbnailCard";
          }
        }
      }
    }
    this.touch();
  }

  private episodeThumbnailSpec(episode: Episode_Serialize): NonNullable<MenuButton_Serialize["thumbnail"]> {
    const fps = this.frameRate;
    const rate =
      Math.abs(fps - 25) < 0.01
        ? { num: 25, den: 1 }
        : Math.abs(fps - 24000 / 1001) < 0.01
          ? { num: 24_000, den: 1_001 }
          : { num: 30_000, den: 1_001 };
    const timeMs = episode.thumbnailTimeMs ?? 0;
    return {
      asset: episode.video,
      inFrame: Math.max(0, Math.round((timeMs / 1000) * (rate.num / rate.den))),
      durationFrames: 0,
      fps: rate,
    };
  }

  /** One-click thumbnail: the episode's chosen frame (or its first frame). */
  useEpisodeThumbnail(buttonId: string, episodeId: string): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    const button = this.activeMenu?.buttons.find((item) => item.id === buttonId);
    if (!episode || !button) return;
    button.thumbnail = this.episodeThumbnailSpec(episode);
    if (button.styleKind === "textOnly" || button.styleKind === "hotspot") {
      button.styleKind = "thumbnailCard";
    }
    void this.ensureVideoThumbnail(episode.video, episode.thumbnailTimeMs ?? undefined);
    this.touch();
  }

  setEpisodeAudio(episodeId: string, sourceStream: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    const chosen = episode.audio.find((audio) => audio.sourceStream === sourceStream);
    if (!chosen) return;
    episode.audio = [chosen, ...episode.audio.filter((audio) => audio !== chosen)].map(
      (audio, index) => ({ ...audio, default: index === 0 }),
    );
    this.touch();
  }

  setEpisodePriority(episodeId: string, priority: EncodingPriority): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    episode.encodingPriority = priority;
    this.plan = null;
    this.touch();
  }

  addEpisodeAudioTrack(episodeId: string, sourceStream: number): boolean {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return false;
    if (episode.audio.some((audio) => audio.sourceStream === sourceStream)) return false;
    const stream = this.assetFor(episode.video)?.probe.audio.find(
      (audio) => audio.index === sourceStream,
    );
    episode.audio = [
      ...episode.audio,
      {
        sourceStream,
        sourcePath: null,
        language: stream?.language ?? "und",
        label: stream?.title?.trim() ||
          (stream?.commentary
            ? t("Comentario {number}", { number: episode.audio.length + 1 })
            : t("Pista {number}", { number: episode.audio.length + 1 })),
        gainDb: 0,
        ac3Kbps: 192,
        delayMs: 0,
        format: (stream?.channels ?? 2) >= 6 ? "ac3Surround51" : "ac3Stereo",
        default: episode.audio.length === 0,
        commentary: stream?.commentary ?? false,
        mastering: { loudness: this.evenVolume ? { ...EVEN_VOLUME } : null, channelMode: "source", codec: null, passthrough: false },
      },
    ];
    this.touch();
    return true;
  }

  updateEpisodeAudioTrack(
    episodeId: string,
    index: number,
    patch: Partial<EpisodeAudio_Serialize>,
  ): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    const current = episode?.audio[index];
    if (!episode || !current) return;
    const format: AudioFormat = patch.format ?? current.format;
    episode.audio = episode.audio.map((audio, position) => {
      if (position !== index) {
        return patch.default ? { ...audio, default: false } : audio;
      }
      return {
        ...audio,
        ...patch,
        format,
        ac3Kbps: patch.ac3Kbps ?? current.ac3Kbps,
        delayMs: patch.delayMs ?? current.delayMs,
        gainDb: patch.gainDb ?? current.gainDb,
        default: patch.default ?? current.default,
      };
    });
    if (patch.default) this.setDefaultEpisodeAudio(episodeId, index);
    this.touch();
  }

  setDefaultEpisodeAudio(episodeId: string, index: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    const chosen = episode?.audio[index];
    if (!episode || !chosen) return;
    // The default track moves to the front (players start with track 1).
    // Names the app generated ("Track 2") follow their new position.
    const auto = (number: number) => t("Pista {number}", { number });
    const order = [index, ...episode.audio.map((_, position) => position).filter((p) => p !== index)];
    episode.audio = order.map((from, to) => {
      const audio = episode.audio[from];
      return {
        ...audio,
        default: from === index,
        label: audio.label === auto(from + 1) ? auto(to + 1) : audio.label,
      };
    });
    this.touch();
  }

  removeEpisodeAudioTrack(episodeId: string, index: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode || episode.audio.length <= index) return;
    const wasDefault = episode.audio[index].default;
    episode.audio = episode.audio.filter((_, position) => position !== index);
    if (wasDefault && episode.audio.length > 0) {
      episode.audio[0] = { ...episode.audio[0], default: true };
    }
    this.touch();
  }

  /**
   * Adds a subtitle from a file (`sourcePath`) or from one of the video's own
   * subtitle streams (`sourcePath` null, `sourceStream` = its stream index).
   */
  addEpisodeSubtitleTrack(
    episodeId: string,
    sourcePath: string | null,
    language = "und",
    label?: string,
    sourceStream = 0,
    bitmap = false,
  ): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    if (sourcePath === null) {
      // Only a real subtitle stream of the video (stream 0 is usually video).
      const asset = this.assetFor(episode.video);
      if (!asset?.probe.subtitleTracks?.some((stream) => stream.index === sourceStream)) return;
    }
    const track: SubtitleTrack_Serialize = {
      sourceStream,
      sourcePath,
      language,
      label: label ?? t("Subtítulo {number}", { number: episode.subtitles.length + 1 }),
      delayMs: 0,
      isForced: false,
      isDefault: episode.subtitles.length === 0,
      bitmap,
      style: defaultSubtitleStyle(),
    };
    episode.subtitles = [...episode.subtitles, track];
    this.touch();
  }

  updateEpisodeSubtitleTrack(
    episodeId: string,
    index: number,
    patch: Partial<SubtitleTrack_Serialize>,
  ): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode || !episode.subtitles[index]) return;
    episode.subtitles = episode.subtitles.map((track, position) => {
      if (position !== index) {
        return patch.isDefault ? { ...track, isDefault: false } : track;
      }
      return { ...track, ...patch };
    });
    this.touch();
  }

  /** Makes one subtitle the default, or none (`null`: subtitles start off). */
  setDefaultEpisodeSubtitle(episodeId: string, index: number | null): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode || (index !== null && !episode.subtitles[index])) return;
    episode.subtitles = episode.subtitles.map((track, position) => ({
      ...track,
      isDefault: position === index,
    }));
    this.touch();
  }

  removeEpisodeSubtitleTrack(episodeId: string, index: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    // Removing the default subtitle leaves none: subtitles are optional.
    episode.subtitles = episode.subtitles.filter((_, position) => position !== index);
    this.touch();
  }

  chapterTimeMs(episode: Episode_Serialize, chapter: Chapter_Serialize): number {
    return Math.round((chapter.atFrame / this.frameRate) * 1000);
  }

  addChapterAt(episodeId: string, sourceMs: number): boolean {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return false;
    const start = episode.trimStartMs ?? 0;
    if (sourceMs < start) return false;
    const offset = sourceMs - start;
    const duration = this.episodeEffectiveDuration(episode);
    if (offset >= duration - 500) return false;
    if (episode.chapters.some((chapter) => Math.abs(this.chapterTimeMs(episode, chapter) - offset) < 1000)) {
      return false;
    }
    const atFrame = Math.round((offset / 1000) * this.frameRate);
    const chapterId = crypto.randomUUID();
    episode.chapters = [
      ...episode.chapters,
      {
        id: chapterId,
        title: t("Capítulo {number}", { number: episode.chapters.length + 1 }),
        atFrame,
        thumbnail: null,
      },
    ].sort((a, b) => a.atFrame - b.atFrame);
    // Buttons store chapter *indices*: shift those at/after the insertion point
    // so they keep targeting the same chapter.
    const inserted = episode.chapters.findIndex((chapter) => chapter.id === chapterId);
    this.shiftChapterActions(episodeId, inserted, 1);
    this.touch();
    return true;
  }

  removeChapter(episodeId: string, chapterId: string): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    const removed = episode.chapters.findIndex((chapter) => chapter.id === chapterId);
    if (removed < 0) return;
    episode.chapters = episode.chapters.filter((chapter) => chapter.id !== chapterId);
    this.shiftChapterActions(episodeId, removed + 1, -1);
    this.clampChapterActions(episodeId, episode);
    this.touch();
  }

  private shiftChapterActions(episodeId: string, from: number, delta: number): void {
    for (const menu of this.project?.menus ?? []) {
      for (const button of menu.buttons) {
        if (
          button.action.kind === "jumpTitleChapter" &&
          button.action.episode === episodeId &&
          button.action.chapter >= from
        ) {
          button.action = {
            ...button.action,
            chapter: Math.max(0, button.action.chapter + delta),
          };
        }
      }
    }
  }

  /**
   * Chapters every 5/10 minutes or in four parts. Returns how many chapters
   * the episode has afterwards; a video shorter than the step keeps its
   * chapters (returns 0) instead of losing them.
   */
  autoChapters(episodeId: string, mode: "5min" | "10min" | "quarters"): number {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return 0;
    const duration = this.episodeEffectiveDuration(episode);
    const offsets: number[] = [0];
    if (mode === "quarters") {
      for (let index = 1; index < 4; index += 1) offsets.push((duration * index) / 4);
    } else {
      const step = (mode === "5min" ? 5 : 10) * 60_000;
      for (let offset = step; offset < duration - 500; offset += step) offsets.push(offset);
    }
    if (offsets.length < 2) return 0;
    episode.chapters = offsets.map((offset, index) => ({
      id: crypto.randomUUID(),
      title: index === 0 ? t("Inicio") : t("Capítulo {number}", { number: index + 1 }),
      atFrame: Math.round((offset / 1000) * this.frameRate),
      thumbnail: null,
    }));
    this.clampChapterActions(episodeId, episode);
    this.touch();
    return episode.chapters.length;
  }

  /**
   * Chapters at scene changes (FFmpeg scene score). The minimum gap follows
   * the length (5–30 s) and the count the disc limit (99 DVD, 999 Blu-ray).
   * Returns the number of chapters, or null when detection failed.
   */
  async sceneChapters(episodeId: string, sensitivity = 0.5): Promise<number | null> {
    const project = this.project;
    const episode = project?.episodes.find((item) => item.id === episodeId);
    const asset = episode ? this.assetFor(episode.video) : null;
    const path = asset ? this.assetPath(asset) : null;
    if (!project || !episode || !path) return null;
    const duration = this.episodeEffectiveDuration(episode);
    const minGap = Math.round(Math.min(30_000, Math.max(5_000, duration / 8)));
    const result = await commands.mediaDetectScenes(
      path,
      sensitivity,
      minGap,
      Math.round(episode.trimStartMs ?? 0),
      episode.trimEndMs === null ? null : Math.round(episode.trimEndMs),
      project.disc.mode === "dvdVideo" ? 99 : 999,
    );
    if (result.status === "error") {
      this.error = t("No se pudieron detectar las escenas: {error}", { error: result.error });
      return null;
    }
    if (this.project !== project) return null;
    episode.chapters = result.data.map((offset, index) => ({
      id: crypto.randomUUID(),
      title: index === 0 ? t("Inicio") : t("Capítulo {number}", { number: index + 1 }),
      atFrame: Math.round((offset / 1000) * this.frameRate),
      thumbnail: null,
    }));
    this.clampChapterActions(episodeId, episode);
    this.touch();
    return episode.chapters.length;
  }

  /** Imports FFprobe chapters into the active trim range. */
  importMediaChapters(episodeId: string): number {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return 0;
    const asset = this.assetFor(episode.video);
    const source = asset?.probe.chapters ?? [];
    const start = episode.trimStartMs ?? 0;
    const end = episode.trimEndMs ?? this.episodeSourceDuration(episode);
    const existing = new Set(episode.chapters.map((chapter) => chapter.atFrame));
    const imported: Chapter_Serialize[] = [];
    for (const chapter of source) {
      if (chapter.startMs < start || chapter.startMs >= end) continue;
      const atFrame = Math.round(((chapter.startMs - start) / 1000) * this.frameRate);
      if (atFrame < 0 || existing.has(atFrame)) continue;
      if (imported.some((candidate) => Math.abs(candidate.atFrame - atFrame) < 1)) continue;
      imported.push({
        id: crypto.randomUUID(),
        title: chapter.title.trim() || t("Capítulo {number}", { number: imported.length + 1 }),
        atFrame,
        thumbnail: null,
      });
    }
    if (imported.length === 0) return 0;
    episode.chapters = [...episode.chapters, ...imported].sort((a, b) => a.atFrame - b.atFrame);
    if (!episode.chapters.some((chapter) => chapter.atFrame === 0)) {
      episode.chapters.unshift({
        id: crypto.randomUUID(),
        title: t("Inicio"),
        atFrame: 0,
        thumbnail: null,
      });
    }
    this.clampChapterActions(episodeId, episode);
    this.touch();
    return imported.length;
  }

  moveEpisode(episodeId: string, delta: number): void {
    const project = this.project;
    if (!project) return;
    const index = project.episodes.findIndex((episode) => episode.id === episodeId);
    const target = index + delta;
    if (index < 0 || target < 0 || target >= project.episodes.length) return;
    const [episode] = project.episodes.splice(index, 1);
    project.episodes.splice(target, 0, episode);
    project.episodes = project.episodes.map((item, position) => ({
      ...item,
      number: position + 1,
    }));
    this.syncPlayAll();
    this.touch();
  }

  removeEpisode(episodeId: string): void {
    const project = this.project;
    if (!project) return;
    project.episodes = project.episodes.filter((episode) => episode.id !== episodeId);
    project.episodes = project.episodes.map((item, position) => ({
      ...item,
      number: position + 1,
    }));
    const fallback = project.episodes[0] ?? null;
    const chapterFallback =
      project.episodes.find((episode) => episode.chapters.length > 0) ?? null;
    for (const menu of project.menus) {
      for (const button of menu.buttons) {
        if (
          button.action.kind === "playEpisode" &&
          button.action.episode === episodeId
        ) {
          button.action = fallback
            ? { kind: "playEpisode", episode: fallback.id }
            : { kind: "playAll" };
        } else if (
          button.action.kind === "jumpTitleChapter" &&
          button.action.episode === episodeId
        ) {
          button.action = chapterFallback
            ? { kind: "jumpTitleChapter", episode: chapterFallback.id, chapter: 0 }
            : fallback
              ? { kind: "playEpisode", episode: fallback.id }
              : { kind: "playAll" };
        }
      }
    }
    this.syncPlayAll();
    this.selectedEpisodeId = project.episodes[0]?.id ?? null;
    this.touch();
  }

  setMenuBackground(assetId: string): void {
    const menu = this.activeMenu;
    if (!menu || !assetId.trim()) return;
    menu.background = { kind: "static", asset: assetId };
    this.dropTemplateBackdrop(menu);
    this.touch();
  }

  /** A picture or video chosen as background replaces the template's own
   *  colour (`tpl:bg`); its decoration stays. */
  private dropTemplateBackdrop(menu: Menu_Serialize): void {
    if (menu.items.some((item) => item.kind === "shape" && item.id === "tpl:bg")) {
      menu.items = menu.items.filter((item) => !(item.kind === "shape" && item.id === "tpl:bg"));
    }
  }

  /// Selects a video asset as the animated menu background. Passing `null`
  /// restores a static background using the first image asset available.
  setMenuBackgroundVideo(assetId: string | null): void {
    const menu = this.activeMenu;
    if (!menu) return;
    if (assetId) {
      menu.background = {
        kind: "videoLoop",
        asset: assetId,
        inFrame: 0,
        outFrame: 0,
        loopForever: true,
        muteSource: true,
      };
      this.dropTemplateBackdrop(menu);
    } else {
      const fallback =
        this.project?.assets.find((asset) => asset.kind === "image")?.id ??
        (menu.background.kind === "static" ? menu.background.asset : null);
      if (fallback) menu.background = { kind: "static", asset: fallback };
    }
    this.touch();
  }

  setMenuTransition(assetId: string | null, durationMs = 1_500): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.transition = assetId
      ? { asset: assetId, durationMs: Math.max(100, Math.round(durationMs)), easing: "linear" }
      : null;
    this.touch();
  }

  updateMenuTransition(patch: Partial<NonNullable<Menu_Serialize["transition"]>>): void {
    const menu = this.activeMenu;
    if (!menu?.transition) return;
    menu.transition = { ...menu.transition, ...patch };
    this.touch();
  }

  /** True when every audio track of the disc is levelled to the same loudness. */
  get evenVolume(): boolean {
    const tracks = (this.project?.episodes ?? []).flatMap((episode) => episode.audio);
    return tracks.length > 0 && tracks.every((track) => track.mastering.loudness !== null);
  }

  /**
   * Same loudness for every episode (EBU R 128 style, -16 LUFS), so the
   * viewer does not reach for the remote between videos. Levelled tracks
   * are re-encoded; turning it off restores the source levels.
   */
  setEvenVolume(enabled: boolean): void {
    const project = this.project;
    if (!project) return;
    for (const episode of project.episodes) {
      episode.audio = episode.audio.map((track) => ({
        ...track,
        mastering: {
          ...track.mastering,
          loudness: enabled ? { ...EVEN_VOLUME } : null,
          passthrough: enabled ? false : track.mastering.passthrough,
        },
      }));
    }
    this.plan = null;
    this.touch();
  }

  /** Root menu: start Play All when nobody chooses anything (DVD-Video). */
  setMenuAutoPlay(enabled: boolean): void {
    const project = this.project;
    const root = project?.menus.find((menu) => menu.id === project.rootMenu);
    if (!root) return;
    root.timing = { ...root.timing, autoPlay: enabled };
    this.touch();
  }

  setMenuAudio(assetId: string | null): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.audio = assetId
      ? {
          asset: assetId,
          gainDb: 0,
          fadeInMs: 0,
          fadeOutMs: 0,
          loopAudio: true,
        }
      : null;
    this.touch();
  }

  setMenuAudioOption(
    key: "gainDb" | "fadeInMs" | "fadeOutMs" | "loopAudio",
    value: number | boolean,
  ): void {
    const menu = this.activeMenu;
    if (!menu?.audio) return;
    Object.assign(menu.audio, { [key]: value });
    this.touch();
  }

  setMenuScaleMode(mode: Menu_Serialize["scaleMode"]): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.scaleMode = mode;
    this.touch();
  }

  setMenuDim(percent: number): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.dimPercent = Math.max(0, Math.min(80, Math.round(percent)));
    this.touch();
  }

  setMenuBlur(px: number): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.blurPx = Math.max(0, Math.min(20, Math.round(px)));
    this.touch();
  }

  setSubpictureColor(index: 0 | 1 | 2 | 3, hex: string, alpha?: number): void {
    const menu = this.activeMenu;
    if (!menu) return;
    const color = menu.subpicture.palette.colors[index];
    color.rgb = hexToRgb(hex);
    if (alpha !== undefined) color.alpha = Math.max(0, Math.min(100, Math.round(alpha)));
    this.touch();
  }

  paletteHex(index: 0 | 1 | 2 | 3): string {
    const menu = this.activeMenu;
    if (!menu) return "#ffffff";
    return rgbToHex(menu.subpicture.palette.colors[index].rgb);
  }

  private makeButton(
    rect: { x: number; y: number; w: number; h: number },
    styleKind: ButtonStyleKind,
    action: MenuButton_Serialize["action"],
    name: string,
    episode: Episode_Serialize | null = null,
    scale = 1,
  ): MenuButton_Serialize {
    return {
      id: crypto.randomUUID(),
      name,
      rect,
      z: 0,
      action,
      thumbnail:
        styleKind === "thumbnailCard" && episode
          ? {
              asset: episode.video,
              inFrame: 0,
              durationFrames: 0,
              fps: { num: 30_000, den: 1_001 },
            }
          : null,
      label: null,
      styleKind,
      fontFamily: "Inter",
      fontSize: Math.round(22 * scale),
      fontWeight: "bold",
      textColor: "#FFFFFF",
      shadowColor: "#000000",
      fillColor: "#0F172A",
      shapeMask: "rectangle",
      motionPreview: null,
      navOverride: { up: null, down: null, left: null, right: null },
      enabled: true,
    };
  }

  addButton(styleKind: ButtonStyleKind = "classicFrame"): boolean {
    const menu = this.activeMenu;
    if (!menu) return false;
    const episode = this.selectedEpisode ?? this.episodes[0] ?? null;
    const action: MenuButton_Serialize["action"] = episode
      ? { kind: "playEpisode", episode: episode.id }
      : { kind: "playAll" };
    const name = episode?.title ?? t("Reproducir todo");
    const scale = menuScale(menu);
    const buttonWidth = Math.round((styleKind === "thumbnailCard" ? 240 : 200) * scale);
    const buttonHeight = Math.round((styleKind === "thumbnailCard" ? 150 : 64) * scale);
    const gap = Math.round(16 * scale);
    const marginX = Math.round(menu.canvas.width * 0.08);
    const marginY = Math.round(menu.canvas.height * 0.1);
    const columns = Math.max(
      1,
      Math.floor((menu.canvas.width - marginX * 2 + gap) / (buttonWidth + gap)),
    );
    const rows = Math.max(
      1,
      Math.floor((menu.canvas.height - marginY * 2 + gap) / (buttonHeight + gap)),
    );
    // Titles and other text on the menu count as taken too, so a new button
    // never lands on top of the template title (decorative shapes and the
    // template background may sit under buttons).
    const occupied = [
      ...menu.buttons.map((existing) => existing.rect),
      ...menu.items.filter((item) => item.kind === "text").map((item) => item.rect),
    ].map((rect) => normalizeRect(rect, menu.canvas.width, menu.canvas.height));
    for (let row = 0; row < rows; row += 1) {
      for (let column = 0; column < columns; column += 1) {
        const candidate = normalizeRect(
          {
            x: marginX + column * (buttonWidth + gap),
            y: marginY + row * (buttonHeight + gap),
            w: buttonWidth,
            h: buttonHeight,
          },
          menu.canvas.width,
          menu.canvas.height,
        );
        if (!occupied.every((existing) => hasGap(candidate, existing, 8))) continue;
        const button = this.makeButton(
          {
            x: candidate.x0,
            y: candidate.y0,
            w: candidate.x1 - candidate.x0,
            h: candidate.y1 - candidate.y0,
          },
          styleKind,
          action,
          name,
          episode,
          scale,
        );
        menu.buttons = [...menu.buttons, button];
        this.selectedButtonId = button.id;
        this.syncPlayAll();
        this.touch();
        return true;
      }
    }
    this.error = t("No queda espacio libre con la separación mínima de 4 px.");
    return false;
  }

  updateButton(id: string, patch: Partial<MenuButton_Serialize>): void {
    const button = this.activeMenu?.buttons.find((item) => item.id === id);
    if (!button) return;
    Object.assign(button, patch);
    this.touch();
  }

  updateButtonName(id: string, name: string): void {
    this.updateButton(id, { name });
  }

  updateButtonAction(id: string, action: MenuButton_Serialize["action"]): void {
    this.updateButton(id, { action });
    this.syncPlayAll();
  }

  updateButtonRect(
    id: string,
    rect: { x: number; y: number; w: number; h: number },
  ): boolean {
    const menu = this.activeMenu;
    if (!menu) return false;
    const button = menu.buttons.find((item) => item.id === id);
    if (!button) return false;
    const normalized = normalizeRect(rect, menu.canvas.width, menu.canvas.height);
    for (const other of menu.buttons) {
      if (other.id === id) continue;
      const otherRect = normalizeRect(other.rect, menu.canvas.width, menu.canvas.height);
      if (!hasGap(normalized, otherRect)) return false;
    }
    button.rect = {
      x: normalized.x0,
      y: normalized.y0,
      w: normalized.x1 - normalized.x0,
      h: normalized.y1 - normalized.y0,
    };
    this.touch();
    return true;
  }

  setButtonNavOverride(
    id: string,
    direction: "up" | "down" | "left" | "right",
    targetId: string | null,
  ): void {
    const button = this.activeMenu?.buttons.find((item) => item.id === id);
    if (!button) return;
    button.navOverride = { ...button.navOverride, [direction]: targetId };
    this.touch();
  }

  /**
   * Adds a new menu screen (submenu). It copies the look of the current menu,
   * gets a "Back" button and the current menu gets a button that opens it, so
   * the new screen is reachable right away.
   */
  addMenu(name?: string): Menu_Serialize | null {
    const project = this.project;
    if (!project) return null;
    const source = this.activeMenu ?? project.menus[0] ?? null;
    if (!source) {
      this.createRootMenu(project);
      this.touch();
      return project.menus[0] ?? null;
    }
    const index = project.menus.length;
    const menuName = name?.trim() || t("Menú {number}", { number: index + 1 });
    const menu: Menu_Serialize = {
      ...structuredClone($state.snapshot(source) as Menu_Serialize),
      id: crypto.randomUUID(),
      name: menuName,
      role: "submenu",
      placement: { kind: "vtsm", titleset: 1 },
      items: [],
      buttons: [],
      template: null,
      transition: null,
    };
    const scale = menuScale(menu);
    const w = Math.round(200 * scale);
    const h = Math.round(64 * scale);
    const margin = Math.round(menu.canvas.width * 0.08);
    const back = this.makeButton(
      this.evenRect(
        { x: margin, y: menu.canvas.height - Math.round(menu.canvas.height * 0.1) - h, w, h },
        menu.canvas.width,
        menu.canvas.height,
      ),
      "classicFrame",
      { kind: "gotoMenu", menu: source.id },
      t("Volver"),
      null,
      scale,
    );
    menu.buttons = [back];
    project.menus = [...project.menus, menu];
    // Link the new screen from the menu the user was editing.
    this.selectedMenuId = source.id;
    const before = source.buttons.length;
    if (this.addButton("classicFrame")) {
      const link = source.buttons[source.buttons.length - 1];
      if (source.buttons.length > before && link) {
        link.action = { kind: "gotoMenu", menu: menu.id };
        link.name = menuName;
      }
    }
    this.selectedMenuId = menu.id;
    this.selectedButtonId = back.id;
    this.studioMode = "menu";
    this.error = null;
    this.syncPlayAll();
    this.touch();
    return menu;
  }

  renameMenu(menuId: string, name: string): void {
    const menu = this.project?.menus.find((item) => item.id === menuId);
    if (!menu || !name.trim()) return;
    menu.name = name.trim();
    this.touch();
  }

  /** Removes a non-root menu; buttons that opened it go back to the root. */
  removeMenu(menuId: string): boolean {
    const project = this.project;
    if (!project || menuId === project.rootMenu) return false;
    if (!project.menus.some((menu) => menu.id === menuId)) return false;
    project.menus = project.menus.filter((menu) => menu.id !== menuId);
    for (const menu of project.menus) {
      for (const button of menu.buttons) {
        if (button.action.kind === "gotoMenu" && button.action.menu === menuId) {
          button.action = { kind: "gotoMenu", menu: project.rootMenu };
        }
      }
    }
    if (this.selectedMenuId === menuId) {
      this.selectedMenuId = project.rootMenu;
      this.selectedButtonId = this.activeMenu?.buttons[0]?.id ?? null;
    }
    this.syncPlayAll();
    this.touch();
    return true;
  }

  /**
   * Layout tools: centre the selected button, or align / distribute every
   * button of the menu. Rejected (with a message) when buttons would touch,
   * because DVD/HDMV highlights need 4 px between buttons.
   */
  arrangeButtons(
    mode: "centerH" | "centerV" | "alignTop" | "alignMiddle" | "distributeH" | "distributeV",
  ): boolean {
    const menu = this.activeMenu;
    if (!menu) return false;
    const rects = new Map(
      menu.buttons.map((button) => [
        button.id,
        { x: button.rect.x ?? 0, y: button.rect.y ?? 0, w: button.rect.w ?? 0, h: button.rect.h ?? 0 },
      ]),
    );
    const even = (value: number) => Math.max(0, Math.round(value / 2) * 2);
    const selected = this.selectedButtonId ? rects.get(this.selectedButtonId) : undefined;
    if (mode === "centerH" || mode === "centerV") {
      if (!selected) return false;
      if (mode === "centerH") selected.x = even((menu.canvas.width - selected.w) / 2);
      else selected.y = even((menu.canvas.height - selected.h) / 2);
    } else {
      const list = [...rects.values()];
      if (list.length < 2) return false;
      if (mode === "alignTop") {
        const top = Math.min(...list.map((rect) => rect.y));
        for (const rect of list) rect.y = even(top);
      } else if (mode === "alignMiddle") {
        const middle = list.reduce((sum, rect) => sum + rect.y + rect.h / 2, 0) / list.length;
        for (const rect of list) rect.y = even(middle - rect.h / 2);
      } else {
        const horizontal = mode === "distributeH";
        const sorted = [...list].sort((a, b) => (horizontal ? a.x - b.x : a.y - b.y));
        const first = sorted[0];
        const last = sorted[sorted.length - 1];
        const span = horizontal ? last.x + last.w - first.x : last.y + last.h - first.y;
        const used = sorted.reduce((sum, rect) => sum + (horizontal ? rect.w : rect.h), 0);
        const gap = (span - used) / (sorted.length - 1);
        let cursor = horizontal ? first.x : first.y;
        for (const rect of sorted) {
          if (horizontal) rect.x = even(cursor);
          else rect.y = even(cursor);
          cursor += (horizontal ? rect.w : rect.h) + gap;
        }
      }
    }
    // Validate: inside the canvas and 4 px apart.
    const all = [...rects.values()];
    const inside = all.every(
      (rect) => rect.x + rect.w <= menu.canvas.width && rect.y + rect.h <= menu.canvas.height,
    );
    const apart = all.every((a, index) =>
      all.slice(index + 1).every(
        (b) => a.x + a.w + 4 <= b.x || b.x + b.w + 4 <= a.x || a.y + a.h + 4 <= b.y || b.y + b.h + 4 <= a.y,
      ),
    );
    if (!inside || !apart) {
      this.error = t("No se puede: los botones quedarían pegados o fuera del menú. Sepáralos un poco antes.");
      return false;
    }
    for (const button of menu.buttons) {
      const rect = rects.get(button.id);
      if (rect) button.rect = { ...button.rect, ...rect };
    }
    this.touch();
    return true;
  }

  /** Moves the selected button in front of / behind the others. */
  restackSelectedButton(direction: "front" | "back"): void {
    const menu = this.activeMenu;
    const button = menu?.buttons.find((item) => item.id === this.selectedButtonId);
    if (!menu || !button) return;
    const levels = menu.buttons.map((item) => item.z);
    button.z = direction === "front" ? Math.max(...levels) + 1 : Math.min(...levels) - 1;
    this.touch();
  }

  /** Copies the selected button next to it (new id, same action). */
  duplicateSelectedButton(): boolean {
    const menu = this.activeMenu;
    const source = menu?.buttons.find((button) => button.id === this.selectedButtonId);
    if (!menu || !source) return false;
    const copy = JSON.parse(JSON.stringify(source)) as typeof source;
    copy.id = crypto.randomUUID();
    copy.name = `${source.name} (${t("copia")})`;
    const offset = 16 * menuScale(menu);
    const { x, y, w, h } = { x: copy.rect.x ?? 0, y: copy.rect.y ?? 0, w: copy.rect.w ?? 0, h: copy.rect.h ?? 0 };
    copy.rect = {
      ...copy.rect,
      x: Math.min(x + offset, Math.max(0, menu.canvas.width - w)),
      y: Math.min(y + offset, Math.max(0, menu.canvas.height - h)),
    };
    copy.navOverride = { up: null, down: null, left: null, right: null };
    menu.buttons = [...menu.buttons, copy];
    this.selectedButtonId = copy.id;
    this.syncPlayAll();
    this.touch();
    return true;
  }

  deleteSelectedButton(): void {
    const menu = this.activeMenu;
    if (!menu || !this.selectedButtonId) return;
    const removed = this.selectedButtonId;
    menu.buttons = menu.buttons.filter((button) => button.id !== removed);
    // Overrides that targeted the deleted button would survive as dangling IDs;
    // Rust reports E_NAV_OVERRIDE_INVALID and the render aborts.
    for (const button of menu.buttons) {
      const nav = button.navOverride;
      if (nav.up === removed || nav.down === removed || nav.left === removed || nav.right === removed) {
        button.navOverride = {
          up: nav.up === removed ? null : nav.up,
          down: nav.down === removed ? null : nav.down,
          left: nav.left === removed ? null : nav.left,
          right: nav.right === removed ? null : nav.right,
        };
      }
    }
    this.selectedButtonId = menu.buttons[0]?.id ?? null;
    this.syncPlayAll();
    this.touch();
  }

  applyLayout(kind: LayoutKind): boolean {
    const menu = this.activeMenu;
    if (!menu) return false;
    if (kind === "clear") {
      menu.buttons = [];
      this.selectedButtonId = null;
      this.touch();
      return true;
    }
    const episodes = this.episodes;
    if (episodes.length === 0) {
      this.error = t("Importa y añade al menos un episodio antes de autogenerar el menú.");
      return false;
    }
    const width = menu.canvas.width;
    const height = menu.canvas.height;
    const marginX = even(width * 0.06);
    const marginY = even(height * 0.08);
    const gap = even(Math.max(16, width * 0.025));
    const buttons: MenuButton_Serialize[] = [];

    if (kind === "grid2" || kind === "grid3") {
      const columns = kind === "grid2" ? 2 : Math.min(3, episodes.length);
      const rows = Math.ceil(episodes.length / columns);
      const cellWidth = Math.floor((width - marginX * 2 - gap * (columns - 1)) / columns);
      const cellHeight = Math.floor((height - marginY * 2 - gap * (rows - 1)) / rows);
      episodes.forEach((episode, index) => {
        const column = index % columns;
        const row = Math.floor(index / columns);
        const rect = {
          x: marginX + column * (cellWidth + gap),
          y: marginY + row * (cellHeight + gap),
          w: cellWidth,
          h: cellHeight,
        };
        buttons.push(
          this.makeButton(
            this.evenRect(rect, width, height),
            "thumbnailCard",
            { kind: "playEpisode", episode: episode.id },
            episode.title,
            episode,
          ),
        );
      });
    } else if (kind === "list") {
      const columns = episodes.length > 6 ? 2 : 1;
      const rows = Math.ceil(episodes.length / columns);
      const cardWidth = Math.floor(
        (width - marginX * 2 - gap * (columns - 1)) / columns,
      );
      const cardHeight = Math.floor((height - marginY * 2 - gap * (rows - 1)) / rows);
      episodes.forEach((episode, index) => {
        const column = Math.floor(index / rows);
        const row = index % rows;
        const rect = {
          x: marginX + column * (cardWidth + gap),
          y: marginY + row * (cardHeight + gap),
          w: cardWidth,
          h: cardHeight,
        };
        buttons.push(
          this.makeButton(
            this.evenRect(rect, width, height),
            "thumbnailCard",
            { kind: "playEpisode", episode: episode.id },
            episode.title,
            episode,
          ),
        );
      });
    } else {
      const playWidth = even(Math.min(width * 0.42, 360));
      const playHeight = even(Math.min(height * 0.18, 92));
      buttons.push(
        this.makeButton(
          this.evenRect(
            {
              x: (width - playWidth) / 2,
              y: marginY,
              w: playWidth,
              h: playHeight,
            },
            width,
            height,
          ),
          "playBadge",
          { kind: "playAll" },
          t("Reproducir todo"),
        ),
      );
      const top = marginY + playHeight + gap;
      const columns = Math.min(2, episodes.length);
      const rows = Math.ceil(episodes.length / columns);
      const cellWidth = Math.floor((width - marginX * 2 - gap * (columns - 1)) / columns);
      const cellHeight = Math.floor((height - top - marginY - gap * (rows - 1)) / rows);
      episodes.forEach((episode, index) => {
        const column = index % columns;
        const row = Math.floor(index / columns);
        buttons.push(
          this.makeButton(
            this.evenRect(
              {
                x: marginX + column * (cellWidth + gap),
                y: top + row * (cellHeight + gap),
                w: cellWidth,
                h: cellHeight,
              },
              width,
              height,
            ),
            "textOnly",
            { kind: "playEpisode", episode: episode.id },
            episode.title,
          ),
        );
      });
    }
    menu.buttons = buttons;
    this.selectedButtonId = buttons[0]?.id ?? null;
    this.error = null;
    this.syncPlayAll();
    this.touch();
    for (const episode of episodes) {
      void this.ensureVideoThumbnail(episode.video);
    }
    return true;
  }

  /**
   * Creates or refreshes the on-disc "Audio and subtitles" page, where the
   * viewer picks the language with the remote. False when there is nothing
   * to choose (one audio track and no subtitles).
   */
  async applySetupMenu(): Promise<boolean> {
    const project = this.project;
    if (!project) return false;
    this.busy = true;
    this.error = null;
    try {
      const before = project.menus.length;
      const result = await unwrap(commands.menuApplySetupMenu(project));
      localizeGeneratedMenus(result.menus);
      project.menus = result.menus;
      this.plan = null;
      this.touch();
      return result.menus.length > before || result.menus.some((menu) => menu.name === t("Ajustes"));
    } catch (cause) {
      this.error = messageOf(cause);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /** Creates or refreshes the scene-selection menus from the chapters. */
  async applyChapterMenus(): Promise<boolean> {
    const project = this.project;
    if (!project) return false;
    this.busy = true;
    this.error = null;
    try {
      const result = await unwrap(commands.menuApplyChapterMenus(project));
      localizeGeneratedMenus(result.menus);
      project.menus = result.menus;
      void this.loadFrameThumbnails();
      this.templateDiagnostics = result.diagnostics.map(
        (diagnostic) => diagnostic.code + ": " + localizeDiagnostic(diagnostic),
      );
      this.plan = null;
      this.touch();
      return true;
    } catch (cause) {
      this.error = messageOf(cause);
      return false;
    } finally {
      this.busy = false;
    }
  }

  async applyMenuTemplate(templateId: MenuTemplateId): Promise<boolean> {
    const project = this.project;
    if (!project) return false;
    this.busy = true;
    this.error = null;
    this.templateDiagnostics = [];
    try {
      const result = await unwrap(commands.menuApplyTemplate(project, templateId));
      localizeGeneratedMenus(result.menus);
      project.menus = result.menus;
      project.rootMenu = result.rootMenu;
      this.selectedMenuId = result.rootMenu;
      const root = result.menus.find((menu) => menu.id === result.rootMenu);
      this.selectedButtonId = root?.buttons[0]?.id ?? null;
      this.templateDiagnostics = result.diagnostics.map(
        (diagnostic) => diagnostic.code + ": " + localizeDiagnostic(diagnostic),
      );
      this.plan = null;
      this.touch();
      return true;
    } catch (cause) {
      this.error = messageOf(cause);
      return false;
    } finally {
      this.busy = false;
    }
  }

  /**
   * Keeps the play-all configuration aligned with the episode list and the
   * buttons that actually trigger it. Rust validation rejects stale episode
   * IDs and warns when a play-all button points at a disabled feature.
   */
  private syncPlayAll(): void {
    const project = this.project;
    if (!project) return;
    project.playAll.order = project.episodes.map((episode) => episode.id);
    project.playAll.enabled = project.menus.some((menu) =>
      menu.buttons.some((button) => button.action.kind === "playAll"),
    );
  }

  private clampChapterActions(episodeId: string, episode: Episode_Serialize): void {
    const project = this.project;
    if (!project) return;
    for (const menu of project.menus) {
      for (const button of menu.buttons) {
        if (
          button.action.kind !== "jumpTitleChapter" ||
          button.action.episode !== episodeId
        ) {
          continue;
        }
        if (episode.chapters.length === 0) {
          button.action = { kind: "playEpisode", episode: episodeId };
        } else {
          button.action = {
            ...button.action,
            chapter: Math.min(button.action.chapter, episode.chapters.length - 1),
          };
        }
      }
    }
  }

  /**
   * Reflows a menu when its canvas changes (DVD ↔ AVCHD). Every rectangle is
   * normalized and rounded to even pixels so the DVD subpicture masks remain
   * valid after the conversion.
   */
  private rescaleMenu(menu: Menu_Serialize, next: Menu_Serialize["canvas"]): void {
    // Normalize against the *previous* canvas, then scale both axes so a
    // DVD (720x480) layout maps proportionally onto an HD (1920x1080) canvas.
    const previousWidth = menu.canvas.width || next.width;
    const previousHeight = menu.canvas.height || next.height;
    const fx = next.width / previousWidth;
    const fy = next.height / previousHeight;
    const scale = (rect: MenuButton_Serialize["rect"]) => {
      const normalized = normalizeRect(rect, previousWidth, previousHeight);
      return this.evenRect(
        {
          x: normalized.x0 * fx,
          y: normalized.y0 * fy,
          w: (normalized.x1 - normalized.x0) * fx,
          h: (normalized.y1 - normalized.y0) * fy,
        },
        next.width,
        next.height,
      );
    };
    for (const button of menu.buttons) button.rect = scale(button.rect);
    for (const item of menu.items) item.rect = scale(item.rect);
    if (Math.abs(fy - 1) > 0.01) {
      const font = (size: number) => Math.max(8, Math.round(size * fy));
      for (const button of menu.buttons) {
        button.fontSize = font(button.fontSize);
        if (button.label?.fontSize) button.label.fontSize = font(button.label.fontSize);
      }
      for (const item of menu.items) {
        const text = item as { fontSize?: number | null };
        if (typeof text.fontSize === "number") text.fontSize = font(text.fontSize);
      }
    }
    menu.canvas = next;
  }

  private evenRect(
    rect: { x: number; y: number; w: number; h: number },
    width: number,
    height: number,
  ): { x: number; y: number; w: number; h: number } {
    const normalized = normalizeRect(rect, width, height);
    return {
      x: normalized.x0,
      y: normalized.y0,
      w: normalized.x1 - normalized.x0,
      h: normalized.y1 - normalized.y0,
    };
  }

  setDiscMode(mode: AuthoringMode): void {
    const project = this.project;
    if (!project || project.disc.mode === mode) return;
    const fromFps = discFps(project);
    if (mode === "avchd") {
      project.disc.mode = "avchd";
      // HD keeps the TV standard: PAL sources stay at 25 fps (1080i), NTSC
      // projects get 720p 23.976 on DVD media (1080p on a Blu-ray capacity).
      const bluRay = project.disc.capacity === "bd25" || project.disc.capacity === "bd50";
      const pal = project.disc.standard === "pal";
      project.disc.video = {
        kind: "avchd",
        resolution: pal ? "1080i" : bluRay ? "1080p" : "720p",
        frameRate: pal ? { num: 25, den: 1 } : { num: 24_000, den: 1_001 },
        level: "level41",
        blurayCompat: true,
        vbvMaxrateKbps: bluRay ? 38_000 : 24_000,
        vbvBufsizeKbps: bluRay ? 35_000 : 24_000,
        keyintFrames: 24,
        slices: 4,
      };
      project.disc.aspect = "sixteenNine";
      project.budget = { ...AVCHD_BUDGET, maxVideoKbps: bluRay ? 38_000 : 24_000 };
      project.firstPlay = null;
    } else {
      project.disc.mode = "dvdVideo";
      project.disc.video = {
        kind: "dvd",
        profile: "auto",
        autoHalfD1AfterMs: 9_000_000,
        maxVideoKbps: 9_800,
        minVideoKbps: 2_500,
        gopFrames: 12,
        bFrames: 2,
        twoPass: true,
        trellis: true,
      };
      project.budget = { ...DVD_BUDGET };
    }
    retimeDiscFrames(project, fromFps);
    if (project.menus.length === 0) {
      this.createRootMenu(project);
    } else {
      for (const menu of project.menus) {
        this.rescaleMenu(menu, canvasForProject(project));
      }
    }
    // The current tab stays: switching format from VBR quality, say, must
    // not jump to the menu editor.
    this.selectedMenuId = project.menus[0]?.id ?? null;
    this.touch();
    this.plan = null;
  }

  setStandard(standard: TvStandard): void {
    const project = this.project;
    if (!project || project.disc.mode === "avchd" || project.disc.standard === standard) return;
    const fromFps = discFps(project);
    project.disc.standard = standard;
    retimeDiscFrames(project, fromFps);
    for (const menu of project.menus) {
      // rescaleMenu normalizes against the *previous* canvas height before
      // scaling (no clipping of the lower rows) and also rescales menu items.
      this.rescaleMenu(menu, canvasForProject(project));
    }
    this.touch();
  }

  setAspect(aspect: AspectRatio): void {
    const project = this.project;
    if (!project || project.disc.mode === "avchd" || project.disc.aspect === aspect) return;
    project.disc.aspect = aspect;
    for (const menu of project.menus) menu.canvas.aspect = aspect;
    this.touch();
  }

  /**
   * Disc size. A Blu-ray size turns a DVD project into HD; an HD project
   * keeps its resolution, frame rate and menus, and only gets the higher
   * Blu-ray bitrate limits (or the AVCHD ones back on DVD media).
   */
  setDiscCapacity(capacity: "dvd5" | "dvd9" | "bd25" | "bd50"): void {
    const project = this.project;
    if (!project || project.disc.capacity === capacity) return;
    const bluRay = capacity === "bd25" || capacity === "bd50";
    project.disc.capacity = capacity;
    if (bluRay && project.disc.mode !== "avchd") this.setDiscMode("avchd");
    const video = project.disc.video;
    if (project.disc.mode === "avchd" && video.kind === "avchd") {
      video.vbvMaxrateKbps = bluRay ? 38_000 : 24_000;
      video.vbvBufsizeKbps = bluRay ? 35_000 : 24_000;
      project.budget.minVideoKbps = 1_000;
      project.budget.maxVideoKbps = bluRay ? 38_000 : 24_000;
      project.budget.headerMarginPpm = 30_000;
      project.budget.muxOverheadPpm = 50_000;
    }
    this.plan = null;
    this.touch();
  }

  /**
   * Target player. Mirrors `PlayerPreset::apply` in kinodisc-domain: it only
   * tightens limits; compatibility warnings come back with the plan.
   */
  setPlayerPreset(preset: PlayerPreset): void {
    const project = this.project;
    if (!project) return;
    const video = project.disc.video;
    if (video.kind === "dvd" && (preset === "ps2" || preset === "dvdPlayer")) {
      const cap = preset === "ps2" ? 8_000 : 8_500;
      video.maxVideoKbps = Math.min(video.maxVideoKbps, cap);
      video.minVideoKbps = Math.min(video.minVideoKbps, cap);
    }
    if (video.kind === "avchd" && (preset === "bluRayPlayer" || preset === "playStation")) {
      video.vbvMaxrateKbps = Math.min(video.vbvMaxrateKbps, 35_000);
      const fps = video.frameRate.num / video.frameRate.den;
      const highFrameRate = video.resolution === "1080p" && fps > 31;
      if (!highFrameRate) video.level = "level41";
    }
    project.disc.playerPreset = preset;
    this.touch();
    this.plan = null;
  }

  setRegionMask(mask: number): void {
    const project = this.project;
    if (!project) return;
    project.disc.regionMask = Math.max(0, Math.min(0x3F, Math.round(mask)));
    this.plan = null;
    this.touch();
  }

  setLayerBreakMode(mode: LayerBreakMode): void {
    const project = this.project;
    if (!project || project.disc.layerBreakMode === mode) return;
    project.disc.layerBreakMode = mode;
    this.plan = null;
    this.touch();
  }

  setTrellis(enabled: boolean): void {
    const project = this.project;
    if (!project || project.disc.video.kind !== "dvd") return;
    project.disc.video.trellis = enabled;
    this.touch();
  }

  updateMenuMotion(patch: Partial<MotionMenuConfig_Serialize>): void {
    const menu = this.activeMenu;
    if (!menu) return;
    const current: MotionMenuConfig_Serialize = menu.motionConfig ?? {
      durationMs: 15_000,
      loopEnabled: true,
      fadeInMs: 500,
      fadeOutMs: 500,
      audioVolume: 1,
    };
    menu.motionConfig = { ...current, ...patch };
    const duration = menu.motionConfig.durationMs;
    menu.timing.durationMs = Math.max(1_000, Math.min(600_000, Math.round(duration)));
    menu.timing.loopPlayback = menu.motionConfig.loopEnabled;
    this.touch();
  }

  disableMenuMotion(): void {
    const menu = this.activeMenu;
    if (!menu) return;
    menu.motionConfig = null;
    this.touch();
  }

  updateButtonMotion(
    id: string,
    patch: Partial<NonNullable<MenuButton_Serialize["motionPreview"]>> | null,
  ): void {
    const button = this.activeMenu?.buttons.find((item) => item.id === id);
    if (!button) return;
    if (patch === null) {
      button.motionPreview = null;
    } else {
      if (!this.activeMenu?.motionConfig) {
        this.updateMenuMotion({});
      }
      button.motionPreview = {
        sourceStartMs: 0,
        loopDurationMs: 10_000,
        ...button.motionPreview,
        ...patch,
      };
    }
    this.touch();
  }

  private createRootMenu(project: Project_Serialize): void {
    const canvas = canvasForProject(project);
    let background = project.assets.find((asset) => asset.kind === "image");
    if (!background) {
      const backgroundId = crypto.randomUUID();
      background = {
        id: backgroundId,
        displayName: "background.png",
        source: { kind: "project" },
        relativePath: "assets/background.png",
        sha256: "0".repeat(64),
        bytes: 0,
        kind: "image",
        probe: {
          formatName: "png",
          durationMs: null,
          sizeBytes: null,
          bitrateBps: null,
          video: null,
          audio: [],
          subtitleStreams: 0,
          subtitleTracks: [],
          chapters: [],
        },
        proxy: null,
        importedAt: new Date().toISOString(),
      };
      project.assets = [...project.assets, background!];
    }
    const menu: Menu_Serialize = {
      id: crypto.randomUUID(),
      name: t("Menú Principal"),
      role: "root",
      placement: { kind: "vmgm" },
      canvas,
      background: { kind: "static", asset: background!.id },
      scaleMode: "cover",
      dimPercent: 0,
      blurPx: 0,
      audio: null,
      motionConfig: null,
      items: [],
      buttons: [],
      subpicture: {
        palette: {
          colors: [
            { rgb: [0, 0, 0], alpha: 0 },
            { rgb: [255, 255, 255], alpha: 15 },
            { rgb: [255, 220, 0], alpha: 15 },
            { rgb: [0, 120, 255], alpha: 15 },
          ],
        },
        force: true,
        autoOutline: false,
        highlightOpacity: 15,
        selectOpacity: 15,
      },
      timing: { durationMs: 30_000, loopPlayback: true, autoPlay: false },
      template: null,
      transition: null,
    };
    project.menus = [menu];
    project.rootMenu = menu.id;
    this.selectedMenuId = menu.id;
  }

  async calculatePlan(): Promise<void> {
    // Bump first so a response still in flight for an older project/state can
    // never overwrite the cleared plan below.
    const sequence = ++this.planSequence;
    const project = this.project;
    if (!project || project.episodes.length === 0) {
      this.plan = null;
      this.planError = null;
      this.planBusy = false;
      return;
    }
    this.planBusy = true;
    try {
      const result = await unwrap(commands.planCalculate(project));
      if (sequence !== this.planSequence) return;
      this.plan = result;
      this.planError = null;
    } catch (cause) {
      if (sequence !== this.planSequence) return;
      this.plan = null;
      this.planError = messageOf(cause);
    } finally {
      if (sequence === this.planSequence) this.planBusy = false;
    }
  }

  resetRender(): void {
    this.rendering = false;
    this.renderStage = "";
    this.renderPercent = 0;
    this.renderLogs = [];
    this.renderError = null;
    this.renderSummary = null;
    this.renderVerification = null;
    this.burnProgress = null;
    this.burnReport = null;
    this.burnError = null;
  }

  applyJobEvent(event: JobEvent): void {
    const kind = event.kind;
    switch (kind.type) {
      case "stageStarted": {
        const label = localizeJobStage(kind.label);
        this.renderStage = label;
        // One bar for the whole job: each stage owns a slice of it, so the
        // bar never restarts at 0 or sits at 100 % while work remains.
        this.renderPercent = Math.max(this.renderPercent, STAGE_SPAN[kind.stage][0]);
        this.stageKey = kind.stage;
        this.stageFractions = {};
        this.renderLogs = [...this.renderLogs, `▸ ${label}`].slice(-300);
        break;
      }
      case "progress": {
        // Within a stage the bar only moves forward (a new stage starts it at
        // 0). DVD menus are encoded while the episodes are: that stage shows
        // both, weighted, so quick menus do not read as "100 %".
        const fraction = Math.min(1, kind.permille / 1000);
        this.stageFractions[kind.stage] = Math.max(this.stageFractions[kind.stage] ?? 0, fraction);
        const episodes = this.stageFractions.encodeEpisodes;
        const menus = this.stageFractions.encodeMenus;
        // DVD encodes menus during this stage too: they count from the
        // start, so cached episodes do not fill the bar before the menus.
        const dvd = this.project?.disc.mode === "dvdVideo";
        const current =
          this.stageKey === "encodeEpisodes" && (dvd || menus !== undefined)
            ? 0.75 * (episodes ?? 0) + 0.25 * (menus ?? 0)
            : (this.stageFractions[this.stageKey ?? kind.stage] ?? fraction);
        // A finished stage stops just short of its slice's end: the next
        // stage (or "completed") moves the bar on.
        const [start, end] = STAGE_SPAN[this.stageKey ?? kind.stage];
        this.renderPercent = Math.max(this.renderPercent, start + Math.min(current, 0.97) * (end - start));
        break;
      }
      case "warning": {
        // The code stays out of the text. Several notes of one kind (one per
        // audio track, say) read the same once translated: shown once.
        const line = `⚠ ${localizeMessage(kind.code, kind.message)}`;
        if (!this.renderLogs.includes(line)) this.renderLogs = [...this.renderLogs, line].slice(-300);
        break;
      }
      case "log":
        this.renderLogs = [...this.renderLogs, localizeLogLine(kind.message)].slice(-300);
        break;
      case "artifact":
        if (kind.kind === "iso") this.lastIso = kind.path;
        this.renderLogs = [...this.renderLogs, `✓ ${kind.path}`].slice(-300);
        break;
      case "completed": {
        const summary = localizeJobSummary(kind.summary);
        this.renderSummary = summary;
        this.renderPercent = 100;
        this.renderStage = t("Completado");
        this.renderLogs = [...this.renderLogs, summary].slice(-300);
        break;
      }
      case "failed":
        // Rust failure text is technical and in Spanish; keep it as detail
        // under a localized headline.
        this.renderError = localizeRenderError(kind.message);
        this.renderStage = t("Error");
        this.renderLogs = [...this.renderLogs, `✕ ${localizeRenderError(kind.message)}`].slice(-300);
        break;
      case "cancelled":
        this.renderError = t("Generación cancelada.");
        this.renderStage = t("Cancelado");
        break;
    }
  }

  applyBurnProgress(progress: BurnProgress): void {
    this.burnProgress = progress;
  }

  async refreshDevices(): Promise<void> {
    this.busy = true;
    try {
      this.devices = await unwrap(commands.burnListDevices());
      this.error = null;
    } catch (cause) {
      this.error = messageOf(cause);
      this.devices = [];
    } finally {
      this.busy = false;
    }
  }

  private autosaveTimer: ReturnType<typeof setInterval> | null = null;

  /** Writes a crash-recovery copy every 30 s while there are unsaved changes. */
  startAutosave(): void {
    if (this.autosaveTimer) return;
    this.autosaveTimer = setInterval(() => void this.autosaveNow(), 30_000);
  }

  async autosaveNow(): Promise<void> {
    this.history.flush();
    const project = this.project;
    if (!project || !this.dirty) return;
    try {
      await unwrap(
        commands.autosaveWrite({
          projectId: project.id,
          name: project.name,
          path: this.projectPath,
          savedAt: new Date().toISOString(),
          projectJson: JSON.stringify(project),
        }),
      );
    } catch {
      // Recovery copies are best effort; the real save reports errors.
    }
  }

  async discardAutosave(projectId?: string): Promise<void> {
    const id = projectId ?? this.project?.id;
    if (!id) return;
    try {
      await unwrap(commands.autosaveDiscard(id));
    } catch {
      // Nothing to discard.
    }
  }

  /**
   * Asks before unsaved changes are lost (new/open/close). Returns true when
   * it is fine to continue.
   */
  async confirmLoseChanges(): Promise<boolean> {
    this.history.flush();
    if (!this.project || !this.dirty) return true;
    const { message } = await import("@tauri-apps/plugin-dialog");
    const save = t("Guardar");
    const discard = t("Descartar cambios");
    const choice = await message(
      t("«{name}» tiene cambios sin guardar. ¿Quieres guardarlos antes de continuar?", {
        name: this.project.name,
      }),
      { title: "Kinodisc", kind: "warning", buttons: { yes: save, no: discard, cancel: t("Cancelar") } },
    );
    if (choice === save || choice === "Yes") {
      const actions = await import("../actions");
      await actions.saveProject();
      // Saving can be cancelled (no file chosen) or fail: then stay.
      return !this.dirty;
    }
    if (choice === discard || choice === "No") {
      await this.discardAutosave();
      return true;
    }
    return false;
  }

  /** Opens a crash-recovery copy; it stays "unsaved" until the user saves. */
  recoverAutosave(entry: { projectJson: string; path?: string | null }): void {
    const project = JSON.parse(entry.projectJson) as Project_Serialize;
    this.projectPath = entry.path ?? null;
    this.setProject(project);
    this.history.markUnsaved();
  }

  async newProject(
    name: string,
    standard: TvStandard,
    aspect: AspectRatio,
    mode: AuthoringMode,
    avchdResolution: AvchdResolution | null,
    avchdFrameRate: FrameRate | null,
  ): Promise<void> {
    if (!(await this.confirmLoseChanges())) return;
    this.busy = true;
    this.error = null;
    try {
      const project = await unwrap(
        commands.projectNew(name, standard, aspect, mode, avchdResolution, avchdFrameRate),
      );
      this.projectPath = null;
      this.setProject(project);
    } catch (cause) {
      this.error = messageOf(cause);
      throw cause;
    } finally {
      this.busy = false;
    }
  }

  async openProject(path: string): Promise<void> {
    if (!(await this.confirmLoseChanges())) return;
    this.busy = true;
    this.error = null;
    try {
      const project = await unwrap(commands.projectOpen(path));
      this.projectPath = path;
      this.setProject(project);
      await this.refreshRecentProjects();
      const missing = await this.checkMissingAssets();
      if (missing > 0) {
        this.error = t(
          "Faltan {count} archivo(s) de este proyecto. En Medios están marcados «No se encuentra»: usa «Buscar archivo…».",
          { count: missing },
        );
      }
    } catch (cause) {
      this.error = messageOf(cause);
      throw cause;
    } finally {
      this.busy = false;
    }
  }

  async refreshRecentProjects(): Promise<void> {
    this.recentProjectsBusy = true;
    this.recentProjectsError = null;
    try {
      this.recentProjects = await unwrap(commands.recentProjectsList(12));
    } catch (cause) {
      // Keep the last known-good list: an empty list would falsely suggest
      // that the user never opened a project.
      this.recentProjectsError = messageOf(cause);
    } finally {
      this.recentProjectsBusy = false;
    }
  }

  async forgetRecentProject(path: string): Promise<void> {
    this.recentProjectsError = null;
    const previous = this.recentProjects;
    this.recentProjects = previous.filter((recent) => recent.filePath !== path);
    try {
      await unwrap(commands.recentProjectForget(path));
    } catch (cause) {
      this.recentProjects = previous;
      this.recentProjectsError = messageOf(cause);
    } finally {
      await this.refreshRecentProjects();
    }
  }

  async refreshJobHistory(): Promise<void> {
    this.jobHistoryBusy = true;
    this.jobHistoryError = null;
    try {
      this.jobHistory = await unwrap(commands.jobHistoryList(20));
    } catch (cause) {
      // Preserve the previous history instead of showing an empty list.
      this.jobHistoryError = messageOf(cause);
    } finally {
      this.jobHistoryBusy = false;
    }
  }

  async refreshToolsDoctor(): Promise<void> {
    this.toolsBusy = true;
    this.toolsError = null;
    try {
      this.toolsReport = await commands.toolsDoctor();
    } catch (cause) {
      this.toolsError = messageOf(cause);
      this.toolsReport = null;
    } finally {
      this.toolsBusy = false;
    }
  }

  async saveProject(path: string): Promise<void> {
    const project = this.project;
    if (!project) return;
    this.busy = true;
    this.error = null;
    try {
      await unwrap(commands.projectSave(path, project));
      this.projectPath = path;
      this.history.markSaved(project);
      void this.discardAutosave(project.id);
      await this.refreshRecentProjects();
    } catch (cause) {
      this.error = messageOf(cause);
      throw cause;
    } finally {
      this.busy = false;
    }
  }
}

export const projectStore = new ProjectStore();
