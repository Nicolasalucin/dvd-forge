/**
 * Static menu items (titles, text, images, shapes) drawn with Canvas 2D.
 * Shared by the disc export and the editor so both show the same thing.
 */
import type { Menu_Serialize, MenuItem_Serialize } from "../api/bindings";

type Item = MenuItem_Serialize;

export function itemRect(item: Item): { x: number; y: number; w: number; h: number } {
  return { x: item.rect.x ?? 0, y: item.rect.y ?? 0, w: item.rect.w ?? 0, h: item.rect.h ?? 0 };
}

/** Font size of a text item (template titles store a scaled size). */
export function textSize(item: Item & { kind: "text" }): number {
  return Math.max(8, item.fontSize ?? 32);
}

export function textFont(item: Item & { kind: "text" }, size = textSize(item)): string {
  const family = item.fontFamily.replaceAll('"', "") || "system-ui";
  return `600 ${size}px "${family}", "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", system-ui, sans-serif`;
}

let scratch: CanvasRenderingContext2D | null = null;

/**
 * The size a text item is drawn at: its own size, or smaller (down to half)
 * when the wrapped text would not fit its box. Editor and disc use the same
 * figure, so long titles shrink instead of being cut off.
 */
export function fittedTextSize(item: Item & { kind: "text" }): number {
  const rect = itemRect(item);
  const base = textSize(item);
  scratch ??= document.createElement("canvas").getContext("2d");
  if (!scratch || rect.w <= 0 || rect.h <= 0) return base;
  const step = Math.max(1, base * 0.05);
  for (let size = base; size >= Math.max(8, base * 0.5); size -= step) {
    scratch.font = textFont(item, size);
    const lines = wrapText(scratch, item.text, Math.max(8, rect.w));
    const widest = Math.max(0, ...lines.map((line) => scratch!.measureText(line).width));
    if (lines.length * size * 1.2 <= rect.h + 1 && widest <= rect.w + 1) return size;
  }
  return Math.max(8, base * 0.5);
}

/** Splits text into lines that fit `width` (explicit line breaks kept). */
export function wrapText(context: CanvasRenderingContext2D, text: string, width: number): string[] {
  const lines: string[] = [];
  for (const paragraph of text.split("\n")) {
    const words = paragraph.split(/\s+/).filter(Boolean);
    if (words.length === 0) {
      lines.push("");
      continue;
    }
    let line = words[0];
    for (const word of words.slice(1)) {
      const next = `${line} ${word}`;
      if (context.measureText(next).width <= width) line = next;
      else {
        lines.push(line);
        line = word;
      }
    }
    lines.push(line);
  }
  return lines;
}

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error("image"));
    image.src = url;
  });
}

function drawText(context: CanvasRenderingContext2D, item: Item & { kind: "text" }): void {
  const rect = itemRect(item);
  const size = fittedTextSize(item);
  context.save();
  context.font = textFont(item, size);
  context.fillStyle = item.color || "#ffffff";
  context.textBaseline = "middle";
  context.textAlign = item.align;
  // A soft shadow keeps titles readable over photos and video.
  context.shadowColor = "rgba(0, 0, 0, 0.55)";
  context.shadowBlur = Math.max(2, size / 10);
  context.shadowOffsetY = Math.max(1, size / 24);
  const lines = wrapText(context, item.text, Math.max(8, rect.w));
  const lineHeight = size * 1.2;
  const top = rect.y + rect.h / 2 - ((lines.length - 1) * lineHeight) / 2;
  const x = item.align === "left" ? rect.x : item.align === "right" ? rect.x + rect.w : rect.x + rect.w / 2;
  lines.forEach((line, index) => context.fillText(line, x, top + index * lineHeight));
  context.restore();
}

/** Draws every item of `menu` in order (later items on top). */
export async function drawMenuItems(
  context: CanvasRenderingContext2D,
  menu: Menu_Serialize,
  imageUrls: Record<string, string>,
): Promise<void> {
  for (const item of menu.items) {
    const rect = itemRect(item);
    if (rect.w <= 0 || rect.h <= 0) continue;
    if (item.kind === "text") {
      if (item.text.trim()) drawText(context, item);
    } else if (item.kind === "shape") {
      context.save();
      context.fillStyle = item.fill;
      context.fillRect(rect.x, rect.y, rect.w, rect.h);
      if (item.stroke && (item.strokeWidth ?? 0) > 0) {
        context.strokeStyle = item.stroke;
        context.lineWidth = item.strokeWidth ?? 1;
        context.strokeRect(rect.x, rect.y, rect.w, rect.h);
      }
      context.restore();
    } else {
      const url = imageUrls[item.asset];
      if (!url) continue;
      try {
        const image = await loadImage(url);
        context.save();
        context.globalAlpha = item.kind === "image" ? Math.min(1, Math.max(0, item.opacity ?? 1)) : 1;
        context.drawImage(image, rect.x, rect.y, rect.w, rect.h);
        context.restore();
      } catch {
        // A missing picture must not stop the export of the rest.
      }
    }
  }
}

/** Assets the items need as previews (images and video posters). */
export function itemAssets(menu: Menu_Serialize): string[] {
  return menu.items.flatMap((item) => (item.kind === "image" || item.kind === "videoThumbnail" ? [item.asset] : []));
}
