import Konva from "konva";
import type { Menu_Serialize, MenuButton_Serialize, SubpicturePalette } from "../../api/bindings";
import { buttonStyleLayout, type ButtonLayout } from "../buttonLayout";
import { normalizeRect } from "../exportMenu";
import { hexToRgba } from "../../utils/color";

export type PreviewState = "normal" | "highlight" | "select";

export interface ButtonCallbacks {
  onSelect?: (id: string) => void;
  onChange?: (
    id: string,
    rect: { x: number; y: number; w: number; h: number },
  ) => boolean;
  onActivate?: (id: string) => void;
}

export interface ButtonSyncOptions {
  selectedId: string | null;
  focusId?: string | null;
  previewState?: PreviewState;
  thumbnails: Record<string, string>;
  editable?: boolean;
  callbacks: ButtonCallbacks;
}

const thumbnailCache = new Map<string, HTMLImageElement>();
type ShapeClipContext = Parameters<NonNullable<Konva.GroupConfig["clipFunc"]>>[0];

function shapeClipFunc(
  shape: MenuButton_Serialize["shapeMask"],
  width: number,
  height: number,
): ((context: ShapeClipContext) => void) | undefined {
  if (shape === "rectangle") return undefined;
  return (context) => {
    context.beginPath();
    if (shape === "circle") {
      context.ellipse(width / 2, height / 2, width / 2, height / 2, 0, 0, Math.PI * 2);
      context.closePath();
      return;
    }
    if (shape === "roundedRect") {
      context.roundRect(0, 0, width, height, Math.min(16, width / 4, height / 4));
      context.closePath();
      return;
    }
    const centerX = width / 2;
    const centerY = height / 2;
    const outer = Math.min(width, height) / 2;
    const inner = outer * 0.45;
    for (let point = 0; point < 10; point += 1) {
      const angle = -Math.PI / 2 + (point * Math.PI) / 5;
      const radius = point % 2 === 0 ? outer : inner;
      const x = centerX + Math.cos(angle) * radius;
      const y = centerY + Math.sin(angle) * radius;
      if (point === 0) context.moveTo(x, y);
      else context.lineTo(x, y);
    }
    context.closePath();
  };
}

function loadThumbnail(
  url: string,
  onLoad: (image: HTMLImageElement) => void,
): HTMLImageElement | null {
  const cached = thumbnailCache.get(url);
  if (cached) {
    if (cached.complete && cached.naturalWidth > 0) return cached;
    cached.addEventListener("load", () => onLoad(cached), { once: true });
    return null;
  }
  const image = new Image();
  image.addEventListener("load", () => onLoad(image), { once: true });
  image.src = url;
  thumbnailCache.set(url, image);
  return null;
}

export class ButtonsLayer {
  readonly layer: Konva.Layer;
  private readonly transformer: Konva.Transformer;
  private readonly groups = new Map<string, Konva.Group>();

  constructor() {
    this.layer = new Konva.Layer();
    this.transformer = new Konva.Transformer({
      rotateEnabled: false,
      keepRatio: false,
      borderStroke: "#38bdf8",
      borderDash: [4, 4],
      anchorFill: "#0f172a",
      anchorStroke: "#38bdf8",
      anchorSize: 9,
      boundBoxFunc: (oldBox, newBox) =>
        newBox.width < 8 || newBox.height < 8 ? oldBox : newBox,
    });
    this.layer.add(this.transformer);
  }

  sync(menu: Menu_Serialize, options: ButtonSyncOptions): void {
    this.transformer.nodes([]);
    this.transformer.visible(options.editable !== false);
    for (const child of this.layer.getChildren().slice()) {
      if (child !== this.transformer) child.destroy();
    }
    this.groups.clear();

    const focusId = options.focusId ?? options.selectedId;
    let selectedGroup: Konva.Group | null = null;
    for (const button of menu.buttons) {
      const rect = normalizeRect(button.rect, menu.canvas.width, menu.canvas.height);
      const width = rect.x1 - rect.x0;
      const height = rect.y1 - rect.y0;
      const thumbnailUrl = button.thumbnail
        ? (options.thumbnails[button.thumbnail.asset] ?? null)
        : null;
      const group = new Konva.Group({
        id: button.id,
        name: "menu-button",
        x: rect.x0,
        y: rect.y0,
        draggable: options.editable !== false && button.enabled,
        opacity: button.enabled ? 1 : 0.38,
        listening: button.enabled,
        clipFunc: shapeClipFunc(button.shapeMask, width, height),
      });

      const hit = new Konva.Rect({
        x: 0,
        y: 0,
        width,
        height,
        fill: "rgba(0, 0, 0, 0.001)",
        listening: true,
      });
      group.add(hit);

      const hasThumbnail = thumbnailUrl !== null;
      const layout = buttonStyleLayout(button.styleKind, width, height, hasThumbnail);
      const styleNodes = drawStyle(
        group,
        button,
        layout,
        width,
        height,
        thumbnailUrl,
        hasThumbnail,
        this.layer,
      );
      styleNodes.forEach((node) => {
        node.listening(false);
      });

      if (button.styleKind === "thumbnailCard") {
        group.add(
          new Konva.Rect({
            name: "button-label-background",
            x: layout.label.x - 4,
            y: layout.label.y - 2,
            width: layout.label.w + 8,
            height: layout.label.h + 4,
            fill: "rgba(2, 6, 23, 0.72)",
            cornerRadius: 6,
            listening: false,
          }),
        );
      }

      const label = new Konva.Text({
        name: "button-label",
        x: layout.label.x,
        y: layout.label.y,
        width: layout.label.w,
        height: layout.label.h,
        text: button.name,
        fontSize:
          button.styleKind === "thumbnailCard"
            ? Math.min(button.fontSize, 18)
            : button.fontSize,
        fontFamily: button.fontFamily || "sans-serif",
        fontStyle: button.fontWeight === "bold" ? "bold" : "normal",
        fill: button.textColor,
        align: layout.align,
        verticalAlign: "middle",
        wrap: "none",
        ellipsis: true,
        shadowColor: button.shadowColor,
        shadowBlur: 4,
        shadowOffsetY: 2,
        listening: false,
      });
      group.add(label);

      const isFocus = focusId === button.id;
      if (options.previewState && options.previewState !== "normal" && isFocus) {
        addPreviewOverlay(
          group,
          button.styleKind,
          width,
          height,
          options.previewState,
          menu.subpicture.palette,
        );
      } else if (options.editable !== false && button.id === options.selectedId) {
        group.add(
          new Konva.Rect({
            name: "selection",
            x: 0,
            y: 0,
            width,
            height,
            stroke: "#38bdf8",
            strokeWidth: 3,
            dash: [5, 4],
            listening: false,
          }),
        );
      }

      if (options.editable !== false) {
        group.on("click tap", (event) => {
          event.cancelBubble = true;
          options.callbacks.onSelect?.(button.id);
        });
        group.on("dragmove", () => {
          const x = snap(group.x(), menu.canvas.width, width);
          const y = snap(group.y(), menu.canvas.height, height);
          group.position({ x, y });
        });
        group.on("dragend", () => {
          const x = snap(group.x(), menu.canvas.width, width);
          const y = snap(group.y(), menu.canvas.height, height);
          group.position({ x, y });
          const accepted =
            options.callbacks.onChange?.(button.id, { x, y, w: width, h: height }) ?? false;
          if (!accepted) {
            group.position({ x: rect.x0, y: rect.y0 });
          }
          this.layer.batchDraw();
        });
        group.on("transformend", () => {
          const scaleX = group.scaleX();
          const scaleY = group.scaleY();
          group.scaleX(1);
          group.scaleY(1);
          const nextWidth = Math.max(8, Math.round(((width * scaleX) / 2) * 2));
          const nextHeight = Math.max(8, Math.round(((height * scaleY) / 2) * 2));
          const x = snap(group.x(), menu.canvas.width, nextWidth);
          const y = snap(group.y(), menu.canvas.height, nextHeight);
          group.position({ x, y });
          const accepted =
            options.callbacks.onChange?.(button.id, {
              x,
              y,
              w: nextWidth,
              h: nextHeight,
            }) ?? false;
          if (!accepted) {
            group.position({ x: rect.x0, y: rect.y0 });
          }
          this.layer.batchDraw();
        });
      } else {
        group.on("click tap", (event) => {
          event.cancelBubble = true;
          options.callbacks.onActivate?.(button.id);
        });
      }

      this.layer.add(group);
      this.groups.set(button.id, group);
      if (button.id === options.selectedId) selectedGroup = group;
    }

    if (options.editable !== false) {
      this.transformer.nodes(selectedGroup ? [selectedGroup] : []);
      this.transformer.moveToTop();
    }
    this.layer.batchDraw();
  }
}

function drawStyle(
  group: Konva.Group,
  button: MenuButton_Serialize,
  layout: ButtonLayout,
  width: number,
  height: number,
  thumbnailUrl: string | null,
  hasThumbnail: boolean,
  layer: Konva.Layer,
): Array<Konva.Group | Konva.Shape> {
  const nodes: Array<Konva.Group | Konva.Shape> = [];
  const radius = Math.min(14, height / 4);

  switch (button.styleKind) {
    case "hotspot":
      break;
    case "textOnly":
      break;
    case "thumbnailCard": {
      nodes.push(
        roundedRectNode({
          width,
          height,
          radius,
          fill: hexToRgba(button.fillColor, 0.82),
          stroke: "rgba(148, 163, 184, 0.55)",
          strokeWidth: 2,
        }),
      );
      if (layout.thumbnail) {
        if (thumbnailUrl && hasThumbnail) {
          nodes.push(
            thumbnailNode(thumbnailUrl, layout.thumbnail, layer),
          );
        } else {
          nodes.push(
            new Konva.Rect({
              x: layout.thumbnail.x,
              y: layout.thumbnail.y,
              width: layout.thumbnail.w,
              height: layout.thumbnail.h,
              fill: "rgba(30, 41, 59, 0.9)",
            }),
          );
        }
      }
      break;
    }
    case "glassPill":
      nodes.push(
        roundedRectNode({
          width,
          height,
          radius: height / 2,
          fill: hexToRgba(button.fillColor, 0.48),
          stroke: "rgba(255, 255, 255, 0.28)",
          strokeWidth: 1.5,
        }),
      );
      break;
    case "classicFrame":
      nodes.push(
        roundedRectNode({
          width,
          height,
          radius,
          fill: hexToRgba(button.fillColor, 0.55),
          stroke: "rgba(226, 232, 240, 0.65)",
          strokeWidth: 2,
        }),
      );
      if (layout.thumbnail && thumbnailUrl && hasThumbnail) {
        nodes.push(thumbnailNode(thumbnailUrl, layout.thumbnail, layer));
      }
      break;
    case "playBadge": {
      nodes.push(
        roundedRectNode({
          width,
          height,
          radius: height / 2,
          fill: hexToRgba(button.fillColor, 0.62),
          stroke: "rgba(255, 255, 255, 0.25)",
          strokeWidth: 1.5,
        }),
      );
      const badgeRadius = Math.min(height * 0.3, 18);
      const centerX = 10 + badgeRadius;
      const centerY = height / 2;
      nodes.push(
        new Konva.Circle({
          x: centerX,
          y: centerY,
          radius: badgeRadius,
          fill: button.textColor,
        }),
      );
      nodes.push(
        new Konva.Line({
          points: [
            centerX - badgeRadius * 0.28,
            centerY - badgeRadius * 0.45,
            centerX + badgeRadius * 0.5,
            centerY,
            centerX - badgeRadius * 0.28,
            centerY + badgeRadius * 0.45,
          ],
          closed: true,
          fill: "#0b1220",
        }),
      );
      break;
    }
  }

  group.add(...nodes);
  return nodes;
}

function roundedRectNode(options: {
  width: number;
  height: number;
  radius: number;
  fill: string;
  stroke: string;
  strokeWidth: number;
}): Konva.Rect {
  return new Konva.Rect({
    x: 0,
    y: 0,
    width: options.width,
    height: options.height,
    cornerRadius: options.radius,
    fill: options.fill,
    stroke: options.stroke,
    strokeWidth: options.strokeWidth,
  });
}

function thumbnailNode(
  url: string,
  area: { x: number; y: number; w: number; h: number },
  layer: Konva.Layer,
): Konva.Group {
  const group = new Konva.Group({
    x: area.x,
    y: area.y,
    clipFunc: (context: ShapeClipContext) => {
      context.beginPath();
      context.roundRect(0, 0, area.w, area.h, Math.min(10, area.h / 4));
      context.closePath();
    },
  });
  const node = new Konva.Image({
    x: 0,
    y: 0,
    width: area.w,
    height: area.h,
    image: undefined,
  });
  const apply = (image: HTMLImageElement) => {
    node.image(image);
    const scale = Math.max(area.w / image.width, area.h / image.height);
    const cropWidth = area.w / scale;
    const cropHeight = area.h / scale;
    node.crop({
      x: Math.max(0, (image.width - cropWidth) / 2),
      y: Math.max(0, (image.height - cropHeight) / 2),
      width: cropWidth,
      height: cropHeight,
    });
    layer.batchDraw();
  };
  const cached = loadThumbnail(url, apply);
  if (cached) apply(cached);
  group.add(node);
  return group;
}

function addPreviewOverlay(
  group: Konva.Group,
  styleKind: MenuButton_Serialize["styleKind"],
  width: number,
  height: number,
  state: PreviewState,
  palette: SubpicturePalette,
): void {
  const highlight = paletteColor(palette, 1);
  const select = paletteColor(palette, 2);
  const border = 4;
  if (styleKind === "textOnly" || styleKind === "hotspot") {
    group.add(
      new Konva.Rect({
        x: 0,
        y: Math.max(0, height - border),
        width,
        height: border,
        fill: state === "select" ? select : highlight,
        listening: false,
      }),
    );
    if (styleKind === "hotspot") {
      group.add(
        new Konva.Rect({
          x: 0,
          y: 0,
          width: 3,
          height,
          fill: state === "select" ? select : highlight,
          listening: false,
        }),
      );
    }
    return;
  }
  if (state === "select") {
    group.add(
      new Konva.Rect({
        x: 0,
        y: 0,
        width,
        height,
        fill: select,
        opacity: 0.55,
        cornerRadius: Math.min(14, height / 4),
        listening: false,
      }),
    );
  }
  group.add(
    new Konva.Rect({
      x: 0,
      y: 0,
      width,
      height,
      stroke: highlight,
      strokeWidth: border,
      cornerRadius: Math.min(14, height / 4),
      listening: false,
    }),
  );
}

function paletteColor(palette: SubpicturePalette, index: 0 | 1 | 2 | 3): string {
  const color = palette.colors[index];
  return `rgb(${color.rgb[0]}, ${color.rgb[1]}, ${color.rgb[2]})`;
}

function snap(value: number, limit: number, size: number): number {
  const max = Math.max(0, limit - size);
  return Math.min(max, Math.max(0, Math.round(value / 2) * 2));
}
