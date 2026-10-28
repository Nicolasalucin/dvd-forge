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
  for (const menu of project.menus) {
    if (menu.buttons.length === 0) continue;
    try {
      const backgroundUrl =
        source.previewUrls[menu.background.asset] ??
        (await previewForAsset(project, menu.background.asset, source));
      const thumbnails = { ...source.previewUrls };
      for (const button of menu.buttons) {
        if (!button.thumbnail) continue;
        const assetId = button.thumbnail.asset;
        if (thumbnails[assetId]) continue;
        const preview = await previewForAsset(project, assetId, source);
        if (preview) thumbnails[assetId] = preview;
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
  const path = frame ?? (asset ? assetPath(asset) : null);
  if (!path) return null;
  try {
    return await unwrap(commands.readImageDataUrl(path));
  } catch {
    return null;
  }
}

export function assetPath(asset: MediaAsset_Serialize): string | null {
  return asset.source.kind === "external"
    ? asset.source.absolutePath
    : (asset.relativePath ?? null);
}
