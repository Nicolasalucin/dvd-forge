import Konva from "konva";
import type { Menu_Serialize } from "../../api/bindings";
import { fittedTextSize, itemRect, textFont } from "../menuItems";

export interface ItemsLayerOptions {
  selectedId: string | null;
  images: Record<string, string>;
  editable: boolean;
  onSelect?: (id: string) => void;
  onMove?: (id: string, rect: { x: number; y: number; w: number; h: number }) => void;
}

const imageCache = new Map<string, HTMLImageElement>();

function cachedImage(url: string, redraw: () => void): HTMLImageElement | null {
  const cached = imageCache.get(url);
  if (cached) return cached.complete ? cached : null;
  const image = new Image();
  image.onload = redraw;
  image.src = url;
  imageCache.set(url, image);
  return null;
}

/**
 * Titles, text, images and shapes of a menu. In the editor they can be
 * selected and dragged; what is drawn matches `drawMenuItems` (export).
 */
export class ItemsLayer {
  readonly layer = new Konva.Layer();
  private token = "";

  sync(menu: Menu_Serialize, options: ItemsLayerOptions): void {
    const token = JSON.stringify([menu.items, options.selectedId, options.editable, Object.keys(options.images).length]);
    if (token === this.token) return;
    this.token = token;
    this.layer.destroyChildren();
    const redraw = () => {
      this.token = "";
      this.sync(menu, options);
    };
    for (const item of menu.items) {
      const rect = itemRect(item);
      if (rect.w <= 0 || rect.h <= 0) continue;
      const selected = item.id === options.selectedId;
      const group = new Konva.Group({ x: rect.x, y: rect.y, draggable: options.editable, name: "menu-item" });
      if (item.kind === "text") {
        group.add(
          new Konva.Text({
            width: rect.w,
            height: rect.h,
            text: item.text,
            fontFamily: `"${item.fontFamily.replaceAll('"', "")}", "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", system-ui, sans-serif`,
            fontSize: fittedTextSize(item),
            fontStyle: "600",
            fill: item.color || "#ffffff",
            align: item.align,
            verticalAlign: "middle",
            lineHeight: 1.2,
            wrap: "word",
            shadowColor: "black",
            shadowOpacity: 0.55,
            shadowBlur: Math.max(2, fittedTextSize(item) / 10),
            shadowOffsetY: Math.max(1, fittedTextSize(item) / 24),
          }),
        );
        void textFont;
      } else if (item.kind === "shape") {
        group.add(
          new Konva.Rect({
            width: rect.w,
            height: rect.h,
            fill: item.fill,
            stroke: item.stroke ?? undefined,
            strokeWidth: item.strokeWidth ?? 0,
          }),
        );
      } else {
        const url = options.images[item.asset];
        const image = url ? cachedImage(url, redraw) : null;
        if (image) {
          group.add(
            new Konva.Image({
              image,
              width: rect.w,
              height: rect.h,
              opacity: item.kind === "image" ? Math.min(1, Math.max(0, item.opacity ?? 1)) : 1,
            }),
          );
        } else {
          group.add(new Konva.Rect({ width: rect.w, height: rect.h, fill: "rgba(255,255,255,0.06)" }));
        }
      }
      // Hit area and selection frame.
      group.add(
        new Konva.Rect({
          width: rect.w,
          height: rect.h,
          stroke: selected ? "#5390ff" : options.editable ? "rgba(255,255,255,0.18)" : undefined,
          strokeWidth: selected ? 2 : 1,
          dash: selected ? undefined : [4, 4],
          listening: true,
          fill: "rgba(0,0,0,0)",
          visible: options.editable,
        }),
      );
      if (options.editable) {
        group.on("mousedown touchstart", (event) => {
          event.cancelBubble = true;
          options.onSelect?.(item.id);
        });
        group.on("dragend", () => {
          const x = Math.max(0, Math.round(group.x() / 2) * 2);
          const y = Math.max(0, Math.round(group.y() / 2) * 2);
          options.onMove?.(item.id, { x, y, w: rect.w, h: rect.h });
        });
      }
      this.layer.add(group);
    }
    this.layer.batchDraw();
  }
}
