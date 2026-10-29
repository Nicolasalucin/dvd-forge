import { commands, type MediaAsset_Serialize, type Project_Serialize } from "./api/bindings";
import { messageOf, unwrap } from "./api/client";
import { exportMenu, exportMotionMenu } from "./editor/exportMenu";
import { t } from "./i18n/index.svelte";

export interface MenuPreviewSource {
  backgroundColor: string;
  previewUrls: Record<string, string>;
  framePaths: Record<string, string>;
}

/**
 * Rasteriza todos los menús del proyecto y deja los PNG compuestos en el
 * directorio temporal que consume `render_start`. Los menús sin botones se
 * omiten porque el backend usa el fondo original.
 */
export async function compositeProjectMenus(
  project: Project_Serialize,
  source: MenuPreviewSource,
): Promise<string[]> {
  const warnings: string[] = [];
  // One preview table shared by every menu: chapter menus reuse the same
  // background and episode thumbnails, so each missing asset is resolved once.
  const thumbnails: Record<string, string> = { ...source.previewUrls };
  const shared: MenuPreviewSource = { ...source, previewUrls: thumbnails };
  for (const menu of project.menus) {
    if (menu.buttons.length === 0) continue;
    try {
      const backgroundUrl = await previewForAsset(project, menu.background.asset, shared);
      for (const button of menu.buttons) {
        if (!button.thumbnail) continue;
        await previewForAsset(project, button.thumbnail.asset, shared);
      }
      const motion =
        menu.background.kind === "videoLoop" ||
        Boolean(menu.motionConfig) ||
        menu.buttons.some((button) => Boolean(button.motionPreview && button.thumbnail));
      if (motion) {
        const exported = await exportMotionMenu(
          menu,
          source.backgroundColor,
          backgroundUrl,
          thumbnails,
        );
        await unwrap(
          commands.menuSaveBackground(project.id, menu.id, exported.backgroundDataUrl),
        );
        await unwrap(commands.menuSaveLabels(project.id, menu.id, exported.labelsDataUrl));
        URL.revokeObjectURL(exported.backgroundUrl);
        URL.revokeObjectURL(exported.labelsUrl);
        URL.revokeObjectURL(exported.highlightUrl);
        URL.revokeObjectURL(exported.selectUrl);
      } else {
        const exported = await exportMenu(
          menu,
          source.backgroundColor,
          backgroundUrl,
          thumbnails,
        );
        await unwrap(
          commands.menuSaveBackground(project.id, menu.id, exported.backgroundDataUrl),
        );
        URL.revokeObjectURL(exported.backgroundUrl);
        URL.revokeObjectURL(exported.highlightUrl);
        URL.revokeObjectURL(exported.selectUrl);
      }
    } catch (cause) {
      warnings.push(
        t("No se pudo componer «{menu}»: {error}", {
          menu: menu.name,
          error: messageOf(cause),
        }),
      );
    }
  }
  return warnings;
}

async function previewForAsset(
  project: Project_Serialize,
  assetId: string,
  source: MenuPreviewSource,
): Promise<string | null> {
  const cached = source.previewUrls[assetId];
  if (cached) return cached;
  const frame = source.framePaths[assetId];
  const asset = project.assets.find((item) => item.id === assetId);
  let path = frame ?? (asset ? assetPath(asset) : null);
  if (!path) return null;
  try {
    if (!frame && asset?.kind === "video") {
      // A video file is never a readable image: extract the poster frame the
      // editor shows (same timestamp, cached by the backend) instead of
      // silently composing the menu without it.
      const duration = asset.probe.durationMs ?? 0;
      const target = duration > 0 ? Math.min(duration * 0.1, 10_000) : 1_000;
      path = await unwrap(
        commands.mediaExtractFrame(path, Math.max(0, Math.round(target)), null),
      );
    }
    const preview = await unwrap(commands.readImageDataUrl(path));
    source.previewUrls[assetId] = preview;
    return preview;
  } catch {
    return null;
  }
}

export function assetPath(asset: MediaAsset_Serialize): string | null {
  return asset.source.kind === "external"
    ? asset.source.absolutePath
    : (asset.relativePath ?? null);
}
