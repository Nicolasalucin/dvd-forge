import type {
  ButtonShapeMask,
  ButtonStyleKind,
  LogicalRect,
  MenuButton_Serialize,
  Menu_Serialize,
  SubpicturePalette,
} from "../api/bindings";
import { buttonStyleLayout } from "./buttonLayout";
import { t } from "../i18n/index.svelte";
import { hexToRgb, hexToRgba } from "../utils/color";

export interface PixelRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface CompiledButton {
  id: string;
  name: string;
  rect: PixelRect;
  styleKind: ButtonStyleKind;
  shapeMask: ButtonShapeMask;
}

export interface MenuExport {
  background: Blob;
  highlight: Blob;
  select: Blob;
  backgroundUrl: string;
  highlightUrl: string;
  selectUrl: string;
  /** PNG completo con textos y miniaturas, listo para el render del backend. */
  backgroundDataUrl: string;
  highlightDataUrl: string;
  selectDataUrl: string;
  buttons: CompiledButton[];
}

export interface MotionMenuExport {
  backgroundUrl: string;
  labelsUrl: string;
  highlightUrl: string;
  selectUrl: string;
  /** Fondo sin textos ni botones; el backend superpone los vídeos. */
  backgroundDataUrl: string;
  /** Textos, sombras, marcos e iconos con alfa. */
  labelsDataUrl: string;
  highlightDataUrl: string;
  selectDataUrl: string;
  buttons: CompiledButton[];
}

const GAP = 4;

export async function exportMenu(
  menu: Menu_Serialize,
  backgroundColor: string,
  backgroundImageUrl: string | null = null,
  thumbnailUrls: Record<string, string> = {},
): Promise<MenuExport> {
  const width = menu.canvas.width;
  const height = menu.canvas.height;
  const buttons = menu.buttons.map((button) => compileButton(button, width, height));
  validateGaps(buttons);
  const background = document.createElement("canvas");
  background.width = width;
  background.height = height;
  const context = background.getContext("2d");
  if (!context) throw new Error(t("No se pudo crear el contexto 2D del fondo."));

  context.fillStyle = backgroundColor;
  context.fillRect(0, 0, width, height);
  if (backgroundImageUrl) {
    const image = await loadImage(backgroundImageUrl);
    if (menu.blurPx > 0) {
      context.save();
      context.filter = `blur(${menu.blurPx}px)`;
      drawImageWithScale(context, image, width, height, menu.scaleMode);
      context.restore();
    } else {
      drawImageWithScale(context, image, width, height, menu.scaleMode);
    }
  }
  if (menu.dimPercent > 0) {
    context.fillStyle = `rgba(0, 0, 0, ${Math.min(80, menu.dimPercent) / 100})`;
    context.fillRect(0, 0, width, height);
  }

  const thumbnailImages = new Map<string, HTMLImageElement>();
  for (const assetId of new Set(
    menu.buttons
      .map((button) => button.thumbnail?.asset)
      .filter((assetId): assetId is string => Boolean(assetId)),
  )) {
    const url = thumbnailUrls[assetId];
    if (!url) continue;
    try {
      thumbnailImages.set(assetId, await loadImage(url));
    } catch {
      // Una miniatura ilegible no debe impedir exportar el resto del menú.
    }
  }

  for (let index = 0; index < buttons.length; index += 1) {
    const compiled = buttons[index];
    const button = menu.buttons[index];
    const rect = compiled.rect;
    const buttonWidth = rect.x1 - rect.x0;
    const buttonHeight = rect.y1 - rect.y0;
    const image = button.thumbnail
      ? thumbnailImages.get(button.thumbnail.asset)
      : undefined;
    const layout = buttonStyleLayout(
      button.styleKind,
      buttonWidth,
      buttonHeight,
      image !== undefined,
    );
    context.save();
    context.translate(rect.x0, rect.y0);
    drawButton(context, button, layout, buttonWidth, buttonHeight, image);
    context.restore();
  }

  // quantizeMask() snaps every pixel to transparent/highlight/select, so the
  // layers are palette-safe without a second full getImageData pass.
  const highlight = renderMask(buttons, width, height, false, menu.subpicture.palette);
  const select = renderMask(buttons, width, height, true, menu.subpicture.palette);

  const backgroundBlob = await toBlob(background);
  const highlightBlob = await toBlob(highlight);
  const selectBlob = await toBlob(select);
  return {
    background: backgroundBlob,
    highlight: highlightBlob,
    select: selectBlob,
    backgroundUrl: URL.createObjectURL(backgroundBlob),
    highlightUrl: URL.createObjectURL(highlightBlob),
    selectUrl: URL.createObjectURL(selectBlob),
    backgroundDataUrl: await blobToDataUrl(backgroundBlob),
    highlightDataUrl: await blobToDataUrl(highlightBlob),
    selectDataUrl: await blobToDataUrl(selectBlob),
    buttons,
  };
}

/**
 * Exporta las capas para un menú animado: el fondo sin botones, el overlay
 * transparente con textos/marcos y las máscaras de subpicture. El backend
 * compone encima los fragmentos de vídeo de cada botón con FFmpeg.
 */
export async function exportMotionMenu(
  menu: Menu_Serialize,
  backgroundColor: string,
  backgroundImageUrl: string | null = null,
  thumbnailUrls: Record<string, string> = {},
): Promise<MotionMenuExport> {
  const width = menu.canvas.width;
  const height = menu.canvas.height;
  const buttons = menu.buttons.map((button) => compileButton(button, width, height));
  validateGaps(buttons);

  const background = document.createElement("canvas");
  background.width = width;
  background.height = height;
  const backgroundContext = background.getContext("2d");
  if (!backgroundContext) throw new Error(t("No se pudo crear el contexto 2D del fondo."));
  backgroundContext.fillStyle = backgroundColor;
  backgroundContext.fillRect(0, 0, width, height);
  if (backgroundImageUrl) {
    const image = await loadImage(backgroundImageUrl);
    if (menu.blurPx > 0) {
      backgroundContext.save();
      backgroundContext.filter = `blur(${menu.blurPx}px)`;
      drawImageWithScale(backgroundContext, image, width, height, menu.scaleMode);
      backgroundContext.restore();
    } else {
      drawImageWithScale(backgroundContext, image, width, height, menu.scaleMode);
    }
  }
  if (menu.dimPercent > 0) {
    backgroundContext.fillStyle = `rgba(0, 0, 0, ${Math.min(80, menu.dimPercent) / 100})`;
    backgroundContext.fillRect(0, 0, width, height);
  }

  const labels = document.createElement("canvas");
  labels.width = width;
  labels.height = height;
  const labelsContext = labels.getContext("2d");
  if (!labelsContext) throw new Error(t("No se pudo crear el contexto 2D de las etiquetas."));

  const thumbnailImages = new Map<string, HTMLImageElement>();
  for (const assetId of new Set(
    menu.buttons
      .map((button) => button.thumbnail?.asset)
      .filter((assetId): assetId is string => Boolean(assetId)),
  )) {
    const url = thumbnailUrls[assetId];
    if (!url) continue;
    try {
      thumbnailImages.set(assetId, await loadImage(url));
    } catch {
      // La miniatura es opcional.
    }
  }

  for (let index = 0; index < buttons.length; index += 1) {
    const compiled = buttons[index];
    const button = menu.buttons[index];
    const rect = compiled.rect;
    const buttonWidth = rect.x1 - rect.x0;
    const buttonHeight = rect.y1 - rect.y0;
    const image = button.thumbnail
      ? thumbnailImages.get(button.thumbnail.asset)
      : undefined;
    const layout = buttonStyleLayout(
      button.styleKind,
      buttonWidth,
      buttonHeight,
      image !== undefined,
    );
    labelsContext.save();
    labelsContext.translate(rect.x0, rect.y0);
    // Mirror build_motion_buttons: without a thumbnail the backend animates
    // the episode targeted by the action, so no opaque card may cover it.
    const animated =
      button.motionPreview &&
      (button.thumbnail ||
        button.action.kind === "playEpisode" ||
        button.action.kind === "jumpTitleChapter");
    if (animated) {
      drawMotionButtonOverlay(labelsContext, button, layout, buttonWidth, buttonHeight);
    } else {
      drawButton(labelsContext, button, layout, buttonWidth, buttonHeight, image);
    }
    labelsContext.restore();
  }

  const highlight = renderMask(buttons, width, height, false, menu.subpicture.palette);
  const select = renderMask(buttons, width, height, true, menu.subpicture.palette);

  const backgroundBlob = await toBlob(background);
  const labelsBlob = await toBlob(labels);
  const highlightBlob = await toBlob(highlight);
  const selectBlob = await toBlob(select);
  return {
    backgroundUrl: URL.createObjectURL(backgroundBlob),
    labelsUrl: URL.createObjectURL(labelsBlob),
    highlightUrl: URL.createObjectURL(highlightBlob),
    selectUrl: URL.createObjectURL(selectBlob),
    backgroundDataUrl: await blobToDataUrl(backgroundBlob),
    labelsDataUrl: await blobToDataUrl(labelsBlob),
    highlightDataUrl: await blobToDataUrl(highlightBlob),
    selectDataUrl: await blobToDataUrl(selectBlob),
    buttons,
  };
}

function drawMotionButtonOverlay(
  context: CanvasRenderingContext2D,
  button: MenuButton_Serialize,
  layout: ReturnType<typeof buttonStyleLayout>,
  width: number,
  height: number,
): void {
  const shaped = button.shapeMask !== "rectangle";
  if (!shaped && (button.styleKind === "textOnly" || button.styleKind === "hotspot")) {
    drawButtonLabel(context, button, layout);
    return;
  }
  context.save();
  traceShape(context, 0, 0, width, height, button.shapeMask);
  context.strokeStyle = "rgba(226, 232, 240, 0.78)";
  context.lineWidth = 2;
  context.stroke();
  if (button.styleKind === "glassPill") {
    context.fillStyle = "rgba(15, 23, 42, 0.18)";
    context.fill();
  }
  if (button.styleKind === "playBadge") {
    const badge = Math.min(height * 0.3, 18);
    const centerX = 10 + badge;
    const centerY = height / 2;
    context.fillStyle = button.textColor;
    context.beginPath();
    context.arc(centerX, centerY, badge, 0, Math.PI * 2);
    context.fill();
    context.fillStyle = "#0b1220";
    context.beginPath();
    context.moveTo(centerX - badge * 0.28, centerY - badge * 0.45);
    context.lineTo(centerX + badge * 0.5, centerY);
    context.lineTo(centerX - badge * 0.28, centerY + badge * 0.45);
    context.closePath();
    context.fill();
  }
  context.restore();
  drawButtonLabel(context, button, layout);
}

function drawButtonLabel(
  context: CanvasRenderingContext2D,
  button: MenuButton_Serialize,
  layout: ReturnType<typeof buttonStyleLayout>,
): void {
  if (button.styleKind === "thumbnailCard") {
    context.save();
    context.fillStyle = "rgba(2, 6, 23, 0.72)";
    roundedRect(
      context,
      layout.label.x - 4,
      layout.label.y - 2,
      layout.label.w + 8,
      layout.label.h + 4,
      6,
    );
    context.fill();
    context.restore();
  }
  const textAlign = layout.align === "left" ? "left" : "center";
  context.font = cssFont(button);
  context.fillStyle = button.textColor;
  context.textAlign = textAlign;
  context.textBaseline = "middle";
  context.shadowColor = hexToRgba(button.shadowColor, 0.9);
  context.shadowBlur = 4;
  context.shadowOffsetY = 2;
  const textX =
    layout.align === "left" ? layout.label.x : layout.label.x + layout.label.w / 2;
  context.fillText(
    button.name,
    textX,
    layout.label.y + layout.label.h / 2,
    Math.max(20, layout.label.w - 4),
  );
  context.shadowColor = "transparent";
  context.shadowBlur = 0;
  context.shadowOffsetY = 0;
}

function drawImageWithScale(
  context: CanvasRenderingContext2D,
  image: HTMLImageElement,
  width: number,
  height: number,
  mode: Menu_Serialize["scaleMode"],
): void {
  if (mode === "stretch") {
    context.drawImage(image, 0, 0, width, height);
    return;
  }
  if (mode === "contain") {
    const scale = Math.min(width / image.width, height / image.height);
    const drawWidth = image.width * scale;
    const drawHeight = image.height * scale;
    context.drawImage(
      image,
      (width - drawWidth) / 2,
      (height - drawHeight) / 2,
      drawWidth,
      drawHeight,
    );
    return;
  }
  const scale = Math.max(width / image.width, height / image.height);
  const sourceWidth = width / scale;
  const sourceHeight = height / scale;
  context.drawImage(
    image,
    (image.width - sourceWidth) / 2,
    (image.height - sourceHeight) / 2,
    sourceWidth,
    sourceHeight,
    0,
    0,
    width,
    height,
  );
}

function drawButton(
  context: CanvasRenderingContext2D,
  button: MenuButton_Serialize,
  layout: ReturnType<typeof buttonStyleLayout>,
  width: number,
  height: number,
  image: HTMLImageElement | undefined,
): void {
  const radius = Math.min(14, height / 4);

  switch (button.styleKind) {
    case "hotspot":
      return;
    case "textOnly":
      break;
    case "thumbnailCard": {
      roundedRect(context, 0, 0, width, height, radius);
      context.fillStyle = hexToRgba(button.fillColor, 0.82);
      context.fill();
      context.strokeStyle = "rgba(148, 163, 184, 0.55)";
      context.lineWidth = 2;
      context.stroke();
      if (image && layout.thumbnail) {
        drawThumbnail(context, layout.thumbnail, image);
      } else if (layout.thumbnail) {
        context.fillStyle = "rgba(30, 41, 59, 0.9)";
        context.fillRect(
          layout.thumbnail.x,
          layout.thumbnail.y,
          layout.thumbnail.w,
          layout.thumbnail.h,
        );
      }
      break;
    }
    case "glassPill":
      roundedRect(context, 0, 0, width, height, height / 2);
      context.fillStyle = hexToRgba(button.fillColor, 0.48);
      context.fill();
      context.strokeStyle = "rgba(255, 255, 255, 0.28)";
      context.lineWidth = 1.5;
      context.stroke();
      break;
    case "classicFrame":
      roundedRect(context, 0, 0, width, height, radius);
      context.fillStyle = hexToRgba(button.fillColor, 0.55);
      context.fill();
      context.strokeStyle = "rgba(226, 232, 240, 0.65)";
      context.lineWidth = 2;
      context.stroke();
      if (image && layout.thumbnail) {
        drawThumbnail(context, layout.thumbnail, image);
      }
      break;
    case "playBadge": {
      roundedRect(context, 0, 0, width, height, height / 2);
      context.fillStyle = hexToRgba(button.fillColor, 0.62);
      context.fill();
      context.strokeStyle = "rgba(255, 255, 255, 0.25)";
      context.lineWidth = 1.5;
      context.stroke();
      const radiusBadge = Math.min(height * 0.3, 18);
      const centerX = 10 + radiusBadge;
      const centerY = height / 2;
      context.fillStyle = button.textColor;
      context.beginPath();
      context.arc(centerX, centerY, radiusBadge, 0, Math.PI * 2);
      context.fill();
      context.fillStyle = "#0b1220";
      context.beginPath();
      context.moveTo(centerX - radiusBadge * 0.28, centerY - radiusBadge * 0.45);
      context.lineTo(centerX + radiusBadge * 0.5, centerY);
      context.lineTo(centerX - radiusBadge * 0.28, centerY + radiusBadge * 0.45);
      context.closePath();
      context.fill();
      break;
    }
  }

  drawButtonLabel(context, button, layout);
}

function drawThumbnail(
  context: CanvasRenderingContext2D,
  area: { x: number; y: number; w: number; h: number },
  image: HTMLImageElement,
): void {
  context.save();
  context.beginPath();
  context.rect(area.x, area.y, area.w, area.h);
  context.clip();
  const scale = Math.max(area.w / image.width, area.h / image.height);
  const drawWidth = image.width * scale;
  const drawHeight = image.height * scale;
  context.drawImage(
    image,
    area.x + (area.w - drawWidth) / 2,
    area.y + (area.h - drawHeight) / 2,
    drawWidth,
    drawHeight,
  );
  context.restore();
}

export function compileButton(
  button: MenuButton_Serialize,
  width: number,
  height: number,
): CompiledButton {
  return {
    id: button.id,
    name: button.name,
    rect: normalizeRect(button.rect, width, height),
    styleKind: button.styleKind,
    shapeMask: button.shapeMask,
  };
}

export function normalizeRect(rect: LogicalRect, width: number, height: number): PixelRect {
  const x = rect.x ?? 0;
  const y = rect.y ?? 0;
  const w = rect.w ?? 0;
  const h = rect.h ?? 0;
  const x0 = Math.min(Math.max(0, width - 8), clampEvenFloor(x, width));
  const y0 = Math.min(Math.max(0, height - 8), clampEvenFloor(y, height));
  const x1 = Math.min(width, Math.max(x0 + 8, clampEvenCeil(x + w, width)));
  const y1 = Math.min(height, Math.max(y0 + 8, clampEvenCeil(y + h, height)));
  return { x0, y0, x1, y1 };
}

export function hasGap(a: PixelRect, b: PixelRect, gap = GAP): boolean {
  return (
    a.x1 + gap <= b.x0 ||
    b.x1 + gap <= a.x0 ||
    a.y1 + gap <= b.y0 ||
    b.y1 + gap <= a.y0
  );
}

function renderMask(
  buttons: CompiledButton[],
  width: number,
  height: number,
  selected: boolean,
  palette: SubpicturePalette,
): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  // quantizeMask() reads the pixels back; keep the backing store on the CPU.
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) throw new Error(t("No se pudo crear el contexto 2D de la subimagen."));

  const highlightColor = paletteHex(palette, 1);
  const selectColor = paletteHex(palette, 2);
  for (const button of buttons) {
    const { x0, y0, x1, y1 } = button.rect;
    const widthPx = x1 - x0;
    const heightPx = y1 - y0;
    const border = 4;
    const shaped = button.shapeMask !== "rectangle";
    if (
      !shaped &&
      (button.styleKind === "textOnly" || button.styleKind === "hotspot")
    ) {
      context.fillStyle = selected ? selectColor : highlightColor;
      context.fillRect(x0, y1 - border, widthPx, border);
      if (button.styleKind === "hotspot") {
        context.fillRect(x0, y0, 3, heightPx);
      }
      continue;
    }
    if (selected) {
      context.fillStyle = selectColor;
      traceShape(context, x0, y0, widthPx, heightPx, button.shapeMask);
      context.fill();
    }
    context.strokeStyle = highlightColor;
    context.lineWidth = border;
    traceShape(
      context,
      x0 + border / 2,
      y0 + border / 2,
      Math.max(1, widthPx - border),
      Math.max(1, heightPx - border),
      button.shapeMask,
    );
    context.stroke();
  }
  quantizeMask(canvas, palette);
  return canvas;
}

/**
 * HTML5 Canvas antialiases curved and diagonal edges, producing dozens of
 * intermediate alpha values that DVD subpictures cannot represent. Snap every
 * pixel to either transparent or the nearest of the two subpicture colors.
 */
function quantizeMask(canvas: HTMLCanvasElement, palette: SubpicturePalette): void {
  const context = canvas.getContext("2d");
  if (!context) throw new Error(t("No se pudo validar la paleta de la subimagen."));
  const image = context.getImageData(0, 0, canvas.width, canvas.height);
  const data = image.data;
  const highlight = palette.colors[1].rgb;
  const select = palette.colors[2].rgb;
  for (let index = 0; index < data.length; index += 4) {
    if (data[index + 3] < 128) {
      data[index] = 0;
      data[index + 1] = 0;
      data[index + 2] = 0;
      data[index + 3] = 0;
      continue;
    }
    const red = data[index];
    const green = data[index + 1];
    const blue = data[index + 2];
    const highlightDistance =
      (red - highlight[0]) ** 2 +
      (green - highlight[1]) ** 2 +
      (blue - highlight[2]) ** 2;
    const selectDistance =
      (red - select[0]) ** 2 + (green - select[1]) ** 2 + (blue - select[2]) ** 2;
    const color = highlightDistance <= selectDistance ? highlight : select;
    data[index] = color[0];
    data[index + 1] = color[1];
    data[index + 2] = color[2];
    data[index + 3] = 255;
  }
  context.putImageData(image, 0, 0);
}

function traceShape(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  shape: ButtonShapeMask,
): void {
  context.beginPath();
  if (shape === "circle") {
    context.ellipse(x + width / 2, y + height / 2, width / 2, height / 2, 0, 0, Math.PI * 2);
    return;
  }
  if (shape === "roundedRect") {
    const radius = Math.min(16, width / 4, height / 4);
    roundedRect(context, x, y, width, height, radius);
    return;
  }
  if (shape === "star") {
    const centerX = x + width / 2;
    const centerY = y + height / 2;
    const outer = Math.min(width, height) / 2;
    const inner = outer * 0.45;
    for (let point = 0; point < 10; point += 1) {
      const angle = -Math.PI / 2 + (point * Math.PI) / 5;
      const radius = point % 2 === 0 ? outer : inner;
      const px = centerX + Math.cos(angle) * radius;
      const py = centerY + Math.sin(angle) * radius;
      if (point === 0) context.moveTo(px, py);
      else context.lineTo(px, py);
    }
    context.closePath();
    return;
  }
  context.rect(x, y, width, height);
}

function paletteHex(palette: SubpicturePalette, index: 0 | 1 | 2 | 3): string {
  const color = palette.colors[index];
  return `rgb(${color.rgb[0]}, ${color.rgb[1]}, ${color.rgb[2]})`;
}

function validateGaps(buttons: CompiledButton[]) {
  for (let i = 0; i < buttons.length; i += 1) {
    for (let j = i + 1; j < buttons.length; j += 1) {
      if (!hasGap(buttons[i].rect, buttons[j].rect)) {
        throw new Error(
          `Los botones ${buttons[i].name} y ${buttons[j].name} se solapan o no tienen 4 px de separación.`,
        );
      }
    }
  }
}

function cssFont(button: MenuButton_Serialize): string {
  const family = /\s/.test(button.fontFamily)
    ? `"${button.fontFamily.replaceAll('"', "")}"`
    : button.fontFamily;
  const weight = button.fontWeight === "bold" ? "700" : "400";
  const size =
    button.styleKind === "thumbnailCard"
      ? Math.min(button.fontSize, 18)
      : button.fontSize;
  return `${weight} ${size}px ${family}`;
}

function roundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  const safe = Math.max(0, Math.min(radius, width / 2, height / 2));
  context.beginPath();
  context.moveTo(x + safe, y);
  context.lineTo(x + width - safe, y);
  context.quadraticCurveTo(x + width, y, x + width, y + safe);
  context.lineTo(x + width, y + height - safe);
  context.quadraticCurveTo(x + width, y + height, x + width - safe, y + height);
  context.lineTo(x + safe, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - safe);
  context.lineTo(x, y + safe);
  context.quadraticCurveTo(x, y, x + safe, y);
  context.closePath();
}

function clampEvenFloor(value: number, limit: number): number {
  return Math.min(limit, Math.max(0, Math.floor(value / 2) * 2));
}

function clampEvenCeil(value: number, limit: number): number {
  return Math.min(limit, Math.max(0, Math.ceil(value / 2) * 2));
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error(t("No se pudo cargar la imagen de fondo.")));
    image.src = url;
  });
}

/** Base64-encodes an already encoded PNG off the main thread. */
function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(t("No se pudo exportar el PNG.")));
    reader.readAsDataURL(blob);
  });
}

function toBlob(canvas: HTMLCanvasElement): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error(t("No se pudo exportar el PNG.")));
    }, "image/png");
  });
}
