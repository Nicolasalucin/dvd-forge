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
  type FrameRate,
  type JobEvent,
  type JobHistoryEntry_Serialize,
  type LayerBreakMode,
  type MediaAsset_Serialize,
  type MediaKind,
  type MediaProbe_Serialize,
  type MenuButton_Serialize,
  type Menu_Serialize,
  type MotionMenuConfig_Serialize,
  type OpticalDevice,
  type Project_Serialize,
  type RecentProject_Serialize,
  type SubtitleTrack_Serialize,
  type ToolchainReport,
  type VerifyReport_Serialize,
  type TvStandard,
  type VideoScaleMode,
} from "../api/bindings";
import { messageOf, unwrap } from "../api/client";
import { hasGap, normalizeRect } from "../editor/exportMenu";
import { localizeJobStage } from "../i18n/diagnostics";
import { t } from "../i18n/index.svelte";
import { normalizeLanguageCode } from "../i18n/languages";
import { hexToRgb, rgbToHex } from "../utils/color";

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


function freshId(value: unknown): string {
  return typeof value === "string" && value.trim().length > 0 ? value : crypto.randomUUID();
}

function optionalRef(value: unknown, valid: Set<string>): string | null {
  return typeof value === "string" && valid.has(value) ? value : null;
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

export function guessMediaKind(path: string, probe: MediaProbe_Serialize): MediaKind {
  if (probe.video) return "video";
  if (probe.audio.length > 0) return "audio";
  const lower = path.toLowerCase();
  if (/\.(srt|ass|ssa|vtt)$/.test(lower)) return "subtitle";
  return "image";
}

class ProjectStore {
  project = $state<Project_Serialize | null>(null);
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

  menuBackgroundColor = $state("#0b1220");
  previewUrls = $state<Record<string, string>>({});
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
  leftCollapsed = $state(false);
  rightCollapsed = $state(false);

  lastIso = $state<string | null>(null);
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
        ? `${plan.avchd.resolution.toUpperCase()} ${t("HD")}`
        : t("Alta definición");
    }
    if (plan.profile === "halfD1") return t("Extendida Half-D1");
    if (plan.videoKbps >= 7_000) return t("Excelente");
    if (plan.videoKbps >= 4_500) return t("Buena");
    return t("Básica");
  }

  setProject(project: Project_Serialize): void {
    normalizeProject(project);
    this.project = project;
    this.syncPlayAll();
    this.planSequence += 1;
    this.plan = null;
    this.planError = null;
    this.planBusy = false;
    this.error = null;
    this.selectedMenuId = project.menus[0]?.id ?? null;
    this.selectedEpisodeId = project.episodes[0]?.id ?? null;
    this.selectedButtonId = project.menus[0]?.buttons[0]?.id ?? null;
    this.studioMode = project.menus.length > 0 ? "menu" : "trim";
    this.previewState = "normal";
    this.previewUrls = {};
    this.framePaths = {};
    void this.hydrateProjectPreviews();
  }

  clearProject(): void {
    this.project = null;
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
    if (this.project) this.project.modifiedAt = new Date().toISOString();
  }

  renameProject(name: string): void {
    if (!this.project) return;
    this.project.name = name;
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
        failures.push(`${path.split(/[\\/]/).pop() ?? path}: ${messageOf(cause)}`);
      }
    }
    this.busy = false;
    if (this.project !== project) return imported;
    if (failures.length > 0) {
      this.error = t("No se pudieron importar {count} archivo(s): {detail}", { count: failures.length, detail: failures[0] });
    }
    this.touch();
    return imported;
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

  async hydrateProjectPreviews(): Promise<void> {
    const project = this.project;
    if (!project) return;
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
    const title = asset.displayName.replace(/\.[^.]+$/, "") || `Episodio ${number}`;
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
        default: stream.default || index === 0,
        commentary: stream.commentary,
        mastering: { loudness: null, channelMode: "source", codec: null, passthrough: false },
      })),
      subtitles: (asset.probe.subtitleTracks ?? []).map((stream, index) => ({
        sourceStream: stream.index,
        sourcePath: null,
        language: stream.language ?? "und",
        label: stream.title?.trim() || t("Subtítulo {number}", { number: index + 1 }),
        delayMs: 0,
        isForced: stream.forced,
        isDefault: stream.default,
        bitmap: stream.bitmap,
      })),
      encodingPriority: "medium",
      postAction: "returnToMenu",
      encodeOverride: null,
    };
    project.episodes = [...project.episodes, episode];
    this.selectedEpisodeId = episode.id;
    this.syncPlayAll();
    const menu = this.activeMenu;
    if (menu && menu.buttons.length === 0) {
      const button = this.makeButton(
        { x: even(menu.canvas.width * 0.1), y: even(menu.canvas.height * 0.12), w: 280, h: 150 },
        "thumbnailCard",
        { kind: "playEpisode", episode: episode.id },
        title,
        episode,
      );
      menu.buttons = [button];
      this.selectedButtonId = button.id;
    }
    this.touch();
    return episode;
  }

  updateEpisode(episodeId: string, patch: Partial<Episode_Serialize>): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    Object.assign(episode, patch);
    this.touch();
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
    const menu = this.activeMenu;
    for (const button of menu?.buttons ?? []) {
      if (button.action.kind === "playEpisode" && button.action.episode === episode.id) {
        button.thumbnail = {
          asset: episode.video,
          inFrame: 0,
          durationFrames: 0,
          fps: { num: 30_000, den: 1_001 },
        };
      }
    }
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
        mastering: { loudness: null, channelMode: "source", codec: null, passthrough: false },
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
    episode.audio = [
      { ...chosen, default: true },
      ...episode.audio
        .filter((_, position) => position !== index)
        .map((audio) => ({ ...audio, default: false })),
    ];
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

  addEpisodeSubtitleTrack(
    episodeId: string,
    sourcePath: string | null,
    language = "und",
    label?: string,
  ): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    const track: SubtitleTrack_Serialize = {
      sourceStream: 0,
      sourcePath,
      language,
      label: label ?? `Subtítulo ${episode.subtitles.length + 1}`,
      delayMs: 0,
      isForced: false,
      isDefault: episode.subtitles.length === 0,
      bitmap: false,
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

  setDefaultEpisodeSubtitle(episodeId: string, index: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode || !episode.subtitles[index]) return;
    episode.subtitles = episode.subtitles.map((track, position) => ({
      ...track,
      isDefault: position === index,
    }));
    this.touch();
  }

  removeEpisodeSubtitleTrack(episodeId: string, index: number): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    episode.subtitles = episode.subtitles.filter((_, position) => position !== index);
    if (episode.subtitles.length > 0 && !episode.subtitles.some((track) => track.isDefault)) {
      episode.subtitles[0] = { ...episode.subtitles[0], isDefault: true };
    }
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

  autoChapters(episodeId: string, mode: "5min" | "10min" | "quarters"): void {
    const episode = this.project?.episodes.find((item) => item.id === episodeId);
    if (!episode) return;
    const duration = this.episodeEffectiveDuration(episode);
    const offsets: number[] = [0];
    if (mode === "quarters") {
      for (let index = 1; index < 4; index += 1) offsets.push((duration * index) / 4);
    } else {
      const step = (mode === "5min" ? 5 : 10) * 60_000;
      for (let offset = step; offset < duration - 500; offset += step) offsets.push(offset);
    }
    episode.chapters = offsets.map((offset, index) => ({
      id: crypto.randomUUID(),
      title: index === 0 ? t("Inicio") : t("Capítulo {number}", { number: index + 1 }),
      atFrame: Math.round((offset / 1000) * this.frameRate),
      thumbnail: null,
    }));
    this.clampChapterActions(episodeId, episode);
    this.touch();
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
    this.touch();
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
    } else {
      const fallback =
        this.project?.assets.find((asset) => asset.kind === "image")?.id ??
        (menu.background.kind === "static" ? menu.background.asset : null);
      if (fallback) menu.background = { kind: "static", asset: fallback };
    }
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
      fontSize: 22,
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
    const buttonWidth = styleKind === "thumbnailCard" ? 240 : 200;
    const buttonHeight = styleKind === "thumbnailCard" ? 150 : 64;
    const gap = 16;
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
    const occupied = menu.buttons.map((existing) =>
      normalizeRect(existing.rect, menu.canvas.width, menu.canvas.height),
    );
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
    const previousHeight = menu.canvas.height || next.height;
    const factor = next.height / previousHeight;
    for (const button of menu.buttons) {
      const rect = normalizeRect(button.rect, next.width, previousHeight);
      button.rect = this.evenRect(
        {
          x: rect.x0,
          y: rect.y0 * factor,
          w: rect.x1 - rect.x0,
          h: (rect.y1 - rect.y0) * factor,
        },
        next.width,
        next.height,
      );
    }
    for (const item of menu.items) {
      const rect = normalizeRect(item.rect, next.width, previousHeight);
      item.rect = this.evenRect(
        {
          x: rect.x0,
          y: rect.y0 * factor,
          w: rect.x1 - rect.x0,
          h: (rect.y1 - rect.y0) * factor,
        },
        next.width,
        next.height,
      );
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
    if (mode === "avchd") {
      project.disc.mode = "avchd";
      project.disc.video = {
        kind: "avchd",
        resolution: "720p",
        frameRate: { num: 24_000, den: 1_001 },
        level: "level41",
        blurayCompat: true,
        vbvMaxrateKbps: 24_000,
        vbvBufsizeKbps: 24_000,
        keyintFrames: 24,
        slices: 4,
      };
      project.disc.standard = "ntsc";
      project.disc.aspect = "sixteenNine";
      project.budget = { ...AVCHD_BUDGET };
      project.firstPlay = null;
      if (project.menus.length === 0) {
        this.createRootMenu(project);
      } else {
        for (const menu of project.menus) {
          this.rescaleMenu(menu, canvasFor("ntsc", "sixteenNine"));
        }
      }
      this.selectedMenuId = project.menus[0]?.id ?? null;
      this.studioMode = project.menus.length > 0 ? "menu" : "trim";
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
      if (project.menus.length === 0) {
        this.createRootMenu(project);
      } else {
        for (const menu of project.menus) {
          this.rescaleMenu(menu, canvasFor(project.disc.standard, project.disc.aspect));
        }
      }
      this.selectedMenuId = project.menus[0]?.id ?? null;
      this.studioMode = "menu";
    }
    this.touch();
    this.plan = null;
  }

  setStandard(standard: TvStandard): void {
    const project = this.project;
    if (!project || project.disc.mode === "avchd" || project.disc.standard === standard) return;
    project.disc.standard = standard;
    for (const menu of project.menus) {
      // rescaleMenu normalizes against the *previous* canvas height before
      // scaling (no clipping of the lower rows) and also rescales menu items.
      this.rescaleMenu(menu, canvasFor(standard, project.disc.aspect));
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

  setDiscCapacity(capacity: "dvd5" | "dvd9" | "bd25" | "bd50"): void {
    const project = this.project;
    if (!project || project.disc.capacity === capacity) return;
    const bluRay = capacity === "bd25" || capacity === "bd50";
    project.disc.capacity = capacity;
    if (bluRay) {
      project.disc.mode = "avchd";
      project.firstPlay = null;
      project.disc.standard = "ntsc";
      project.disc.aspect = "sixteenNine";
      project.disc.video = {
        kind: "avchd",
        resolution: "1080p",
        frameRate: { num: 24_000, den: 1_001 },
        level: "level41",
        blurayCompat: true,
        vbvMaxrateKbps: 38_000,
        vbvBufsizeKbps: 35_000,
        keyintFrames: 24,
        slices: 4,
      };
      project.budget.minVideoKbps = 1_000;
      project.budget.maxVideoKbps = 38_000;
      project.budget.headerMarginPpm = 30_000;
      project.budget.muxOverheadPpm = 50_000;
      for (const menu of project.menus) {
        this.rescaleMenu(menu, {
          ...menu.canvas,
          tvStandard: "ntsc",
          aspect: "sixteenNine",
          height: 480,
        });
      }
    } else if (project.disc.mode === "avchd") {
      if (project.disc.video.kind === "avchd") {
        project.disc.video.vbvMaxrateKbps = 24_000;
        project.disc.video.vbvBufsizeKbps = 24_000;
      }
      project.budget.minVideoKbps = 1_000;
      project.budget.maxVideoKbps = 24_000;
      project.budget.headerMarginPpm = 30_000;
      project.budget.muxOverheadPpm = 50_000;
    }
    this.plan = null;
    this.touch();
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
    const canvas = canvasFor(project.disc.standard, project.disc.aspect);
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
      timing: { durationMs: 30_000, loopPlayback: true },
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
        this.renderPercent = 0;
        this.renderLogs = [...this.renderLogs, `▸ ${label}`].slice(-300);
        break;
      }
      case "progress":
        this.renderPercent = kind.permille / 10;
        break;
      case "warning":
        this.renderLogs = [...this.renderLogs, `⚠ ${kind.code}: ${kind.message}`].slice(-300);
        break;
      case "log":
        this.renderLogs = [...this.renderLogs, `[${kind.level}] ${kind.message}`].slice(-300);
        break;
      case "artifact":
        if (kind.kind === "iso") this.lastIso = kind.path;
        this.renderLogs = [...this.renderLogs, `✓ ${kind.path}`].slice(-300);
        break;
      case "completed":
        this.renderSummary = kind.summary;
        this.renderPercent = 100;
        this.renderStage = t("Completado");
        this.renderLogs = [...this.renderLogs, kind.summary].slice(-300);
        break;
      case "failed":
        this.renderError = kind.message;
        this.renderStage = t("Error");
        this.renderLogs = [...this.renderLogs, `✕ ${kind.message}`].slice(-300);
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

  async newProject(
    name: string,
    standard: TvStandard,
    aspect: AspectRatio,
    mode: AuthoringMode,
    avchdResolution: AvchdResolution | null,
    avchdFrameRate: FrameRate | null,
  ): Promise<void> {
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
    this.busy = true;
    this.error = null;
    try {
      const project = await unwrap(commands.projectOpen(path));
      this.projectPath = path;
      this.setProject(project);
      await this.refreshRecentProjects();
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
