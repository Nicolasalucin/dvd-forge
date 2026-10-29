import Konva from "konva";
import type { Menu_Serialize } from "../../api/bindings";

/** Decoded backgrounds by data-URL (small LRU): slider drags re-compose only. */
const DECODED_BACKGROUND_LIMIT = 4;
const decodedBackgrounds = new Map<string, HTMLImageElement>();

function rememberBackground(url: string, image: HTMLImageElement): void {
  decodedBackgrounds.delete(url);
  decodedBackgrounds.set(url, image);
  while (decodedBackgrounds.size > DECODED_BACKGROUND_LIMIT) {
    const oldest = decodedBackgrounds.keys().next().value;
    if (oldest === undefined) break;
    decodedBackgrounds.delete(oldest);
  }
}

/**
 * Pinta el fondo del menú en el lienzo nativo. La composición se hace en un
 * canvas 2D para que Cover/Contain/Stretch, el desenfoque y el oscurecido se
 * comporten exactamente igual que en la exportación PNG.
 */
export function drawBackground(
  layer: Konva.Layer,
  menu: Menu_Serialize,
  color: string,
  imageUrl: string | null = null,
  showSafeAreas = true,
): void {
  const width = menu.canvas.width;
  const height = menu.canvas.height;
  // The URL is compared by reference (it is the same string held by the store)
  // instead of being concatenated into the token, which copied it every call.
  const url = imageUrl ?? "";
  const token = [
    menu.scaleMode,
    menu.dimPercent,
    menu.blurPx,
    showSafeAreas ? "guides" : "clean",
    color,
    width,
    height,
  ].join("|");
  if (
    layer.hasChildren() &&
    layer.getAttr("background-token") === token &&
    layer.getAttr("background-url") === url
  ) {
    return;
  }
  layer.destroyChildren();
  layer.setAttr("background-token", token);
  layer.setAttr("background-url", url);

  layer.add(
    new Konva.Rect({
      x: 0,
      y: 0,
      width,
      height,
      fill: color,
      listening: false,
    }),
  );

  if (showSafeAreas) {
    addGuides(layer, width, height);
  }

  if (!imageUrl) {
    layer.batchDraw();
    return;
  }

  const compose = (image: HTMLImageElement) => {
    if (
      layer.getAttr("background-token") !== token ||
      layer.getAttr("background-url") !== url
    ) {
      return;
    }
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) return;

    context.save();
    if (menu.blurPx > 0) {
      context.filter = `blur(${menu.blurPx}px)`;
    }
    drawImageWithScale(context, image, width, height, menu.scaleMode);
    context.restore();

    if (menu.dimPercent > 0) {
      context.fillStyle = `rgba(0, 0, 0, ${Math.min(80, menu.dimPercent) / 100})`;
      context.fillRect(0, 0, width, height);
    }

    const node = new Konva.Image({
      image: canvas,
      x: 0,
      y: 0,
      width,
      height,
      listening: false,
    });
    layer.add(node);
    node.zIndex(1);
    layer.batchDraw();
  };
  const cached = decodedBackgrounds.get(imageUrl);
  if (cached) {
    rememberBackground(imageUrl, cached);
    compose(cached);
    return;
  }
  const image = new Image();
  image.onload = () => {
    rememberBackground(imageUrl, image);
    compose(image);
  };
  image.onerror = () => {
    if (layer.getAttr("background-token") === token) layer.batchDraw();
  };
  image.src = imageUrl;
}

export function drawImageWithScale(
  context: CanvasRenderingContext2D,
  image: HTMLImageElement | HTMLCanvasElement,
  width: number,
  height: number,
  mode: Menu_Serialize["scaleMode"],
): void {
  const imageWidth = image.width;
  const imageHeight = image.height;
  if (imageWidth === 0 || imageHeight === 0) return;

  if (mode === "stretch") {
    context.drawImage(image, 0, 0, width, height);
    return;
  }
  if (mode === "contain") {
    const scale = Math.min(width / imageWidth, height / imageHeight);
    const drawWidth = imageWidth * scale;
    const drawHeight = imageHeight * scale;
    context.drawImage(
      image,
      (width - drawWidth) / 2,
      (height - drawHeight) / 2,
      drawWidth,
      drawHeight,
    );
    return;
  }
  const scale = Math.max(width / imageWidth, height / imageHeight);
  const sourceWidth = width / scale;
  const sourceHeight = height / scale;
  context.drawImage(
    image,
    (imageWidth - sourceWidth) / 2,
    (imageHeight - sourceHeight) / 2,
    sourceWidth,
    sourceHeight,
    0,
    0,
    width,
    height,
  );
}

function addGuides(layer: Konva.Layer, width: number, height: number): void {
  const actionX = Math.round(width * 0.05);
  const actionY = Math.round(height * 0.05);
  const titleX = Math.round(width * 0.1);
  const titleY = Math.round(height * 0.1);

  addGuide(
    layer,
    actionX,
    actionY,
    width - actionX * 2,
    height - actionY * 2,
    "ACCIÓN 5%",
    "#38bdf8",
  );
  addGuide(
    layer,
    titleX,
    titleY,
    width - titleX * 2,
    height - titleY * 2,
    "TÍTULO 10%",
    "#fbbf24",
  );
}

function addGuide(
  layer: Konva.Layer,
  x: number,
  y: number,
  width: number,
  height: number,
  label: string,
  color: string,
): void {
  layer.add(
    new Konva.Rect({
      x,
      y,
      width,
      height,
      stroke: color,
      strokeWidth: 1,
      dash: [6, 6],
      opacity: 0.6,
      listening: false,
    }),
  );
  layer.add(
    new Konva.Text({
      x: x + 6,
      y: y + 6,
      text: label,
      fontSize: 12,
      fontFamily: "sans-serif",
      fill: color,
      opacity: 0.7,
      listening: false,
    }),
  );
}
