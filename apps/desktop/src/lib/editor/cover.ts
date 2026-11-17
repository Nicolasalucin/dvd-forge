/**
 * Printable DVD/Blu-ray case cover and round disc label, drawn with Canvas
 * 2D at 300 dpi from the project (title, menu colours and font, episode
 * thumbnails and durations). Text only for formats: no official logos.
 */

export const DPI = 300;
const MM = DPI / 25.4;

export interface CoverSize {
  widthMm: number;
  heightMm: number;
  spineMm: number;
}

/** Standard insert sizes: DVD case 273×183 mm (14 mm spine); Blu-ray case
 *  insert 262×148 mm (11 mm spine). */
export function coverSize(bluRay: boolean): CoverSize {
  return bluRay
    ? { widthMm: 262, heightMm: 148, spineMm: 11 }
    : { widthMm: 273, heightMm: 183, spineMm: 14 };
}

/** Disc label: 118 mm across with a 41 mm unprintable hub (22 mm hole). */
export const LABEL_MM = 118;
export const LABEL_HUB_MM = 41;
export const LABEL_HOLE_MM = 22;

export interface CoverEpisode {
  title: string;
  durationMs: number;
  preview: string | null;
}

export interface CoverContent {
  title: string;
  subtitle: string;
  description: string;
  episodes: CoverEpisode[];
  /** "DVD-Video · PAL · 16:9 · 1:50" */
  specs: string;
  background: string;
  text: string;
  accent: string;
  font: string;
}

function px(mm: number): number {
  return Math.round(mm * MM);
}

function loadImage(url: string): Promise<HTMLImageElement | null> {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => resolve(null);
    image.src = url;
  });
}

function duration(ms: number): string {
  const total = Math.round(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
    : `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function font(content: CoverContent, size: number, weight = 600): string {
  return `${weight} ${Math.round(size)}px "${content.font.replaceAll('"', "")}", "Segoe UI", system-ui, sans-serif`;
}

/** Fits `text` on one line in `width` by shrinking the size (min 40 %). */
function fitText(context: CanvasRenderingContext2D, content: CoverContent, text: string, size: number, width: number, weight = 600): number {
  let current = size;
  context.font = font(content, current, weight);
  while (context.measureText(text).width > width && current > size * 0.4) {
    current *= 0.92;
    context.font = font(content, current, weight);
  }
  return current;
}

/**
 * A title in at most two lines: shrinks to fit one line down to 70 %, then
 * wraps into two lines (shrinking further if needed). Returns the y below it.
 */
function drawTitle(
  context: CanvasRenderingContext2D,
  content: CoverContent,
  text: string,
  x: number,
  top: number,
  width: number,
  size: number,
): number {
  let current = size;
  context.font = font(content, current, 700);
  while (context.measureText(text).width > width && current > size * 0.7) {
    current *= 0.94;
    context.font = font(content, current, 700);
  }
  let lines = [text];
  if (context.measureText(text).width > width) {
    lines = wrap(context, text, width);
    // Shrink until the whole title fits in two lines (never cut it).
    while (lines.length > 2 && current > size * 0.35) {
      current *= 0.92;
      context.font = font(content, current, 700);
      lines = wrap(context, text, width);
    }
    lines = lines.slice(0, 2);
  }
  const lineHeight = current * 1.15;
  lines.forEach((line, index) => context.fillText(line, x, top + current + index * lineHeight));
  return top + current + (lines.length - 1) * lineHeight;
}

function wrap(context: CanvasRenderingContext2D, text: string, width: number): string[] {
  const lines: string[] = [];
  for (const paragraph of text.split("\n")) {
    let line = "";
    for (const word of paragraph.split(/\s+/).filter(Boolean)) {
      const next = line ? `${line} ${word}` : word;
      if (context.measureText(next).width > width && line) {
        lines.push(line);
        line = word;
      } else {
        line = next;
      }
    }
    lines.push(line);
  }
  return lines;
}

function drawCover(context: CanvasRenderingContext2D, image: HTMLImageElement, x: number, y: number, w: number, h: number) {
  const scale = Math.max(w / image.width, h / image.height);
  const sw = w / scale;
  const sh = h / scale;
  context.drawImage(image, (image.width - sw) / 2, (image.height - sh) / 2, sw, sh, x, y, w, h);
}

/** The full insert: back | spine | front (as it lies flat in the case). */
export async function renderCover(content: CoverContent, size: CoverSize): Promise<HTMLCanvasElement> {
  const width = px(size.widthMm);
  const height = px(size.heightMm);
  const spine = px(size.spineMm);
  const panel = Math.round((width - spine) / 2);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return canvas;
  context.fillStyle = content.background;
  context.fillRect(0, 0, width, height);
  const images = await Promise.all(
    content.episodes.map((episode) => (episode.preview ? loadImage(episode.preview) : Promise.resolve(null))),
  );
  const margin = px(12);

  // Front: title, subtitle and a collage of up to four episodes.
  const frontX = panel + spine;
  context.textAlign = "center";
  context.textBaseline = "alphabetic";
  context.fillStyle = content.text;
  let cursor = drawTitle(context, content, content.title, frontX + panel / 2, margin, panel - margin * 2, px(14)) + px(4);
  if (content.subtitle.trim()) {
    const subtitleSize = fitText(context, content, content.subtitle, px(6.5), panel - margin * 2, 400);
    context.font = font(content, subtitleSize, 400);
    context.fillStyle = content.accent;
    context.fillText(content.subtitle, frontX + panel / 2, cursor + subtitleSize);
    cursor += subtitleSize + px(4);
  }
  const shown = images.filter((image): image is HTMLImageElement => image !== null).slice(0, 4);
  if (shown.length > 0) {
    const area = { x: frontX + margin, y: cursor + px(4), w: panel - margin * 2, h: height - cursor - margin * 2 - px(10) };
    const columns = shown.length === 1 ? 1 : 2;
    const rows = Math.ceil(shown.length / columns);
    const gap = px(2);
    const cellW = (area.w - gap * (columns - 1)) / columns;
    const cellH = Math.min((area.h - gap * (rows - 1)) / rows, cellW * 0.75);
    const top = area.y + (area.h - (cellH * rows + gap * (rows - 1))) / 2;
    shown.forEach((image, index) => {
      const column = index % columns;
      const row = Math.floor(index / columns);
      const inRow = Math.min(columns, shown.length - row * columns);
      const offset = ((columns - inRow) * (cellW + gap)) / 2;
      drawCover(context, image, area.x + offset + column * (cellW + gap), top + row * (cellH + gap), cellW, cellH);
    });
  }
  context.font = font(content, px(4), 600);
  context.fillStyle = content.accent;
  context.fillText(content.specs, frontX + panel / 2, height - margin);

  // Spine: the title, rotated, read from top to bottom.
  context.save();
  context.fillStyle = content.accent;
  context.globalAlpha = 0.18;
  context.fillRect(panel, 0, spine, height);
  context.globalAlpha = 1;
  context.translate(panel + spine / 2, height / 2);
  context.rotate(Math.PI / 2);
  context.fillStyle = content.text;
  const spineSize = fitText(context, content, content.title, spine * 0.55, height - margin * 2, 700);
  context.font = font(content, spineSize, 700);
  context.textBaseline = "middle";
  context.fillText(content.title, 0, 0);
  context.restore();

  // Back: description, episode list with durations, specs.
  context.textAlign = "left";
  context.textBaseline = "alphabetic";
  context.fillStyle = content.text;
  let y = drawTitle(context, content, content.title, margin, margin, panel - margin * 2, px(7)) + px(6);
  if (content.description.trim()) {
    context.font = font(content, px(3.6), 400);
    for (const line of wrap(context, content.description, panel - margin * 2).slice(0, 8)) {
      context.fillText(line, margin, y + px(3.6));
      y += px(5.2);
    }
    y += px(3);
  }
  const lineHeight = px(5.4);
  const listed = content.episodes.slice(0, Math.max(1, Math.floor((height - y - margin * 2 - px(8)) / lineHeight)));
  listed.forEach((episode, index) => {
    const baseline = y + lineHeight * (index + 1) - px(1.4);
    context.font = font(content, px(3.6), 600);
    context.fillStyle = content.accent;
    context.fillText(String(index + 1).padStart(2, "0"), margin, baseline);
    context.fillStyle = content.text;
    context.font = font(content, px(3.6), 400);
    const name = episode.title;
    const time = duration(episode.durationMs);
    const right = panel - margin;
    const timeWidth = context.measureText(time).width;
    let label = name;
    while (label.length > 1 && context.measureText(label).width > right - margin - px(9) - timeWidth - px(4)) {
      label = `${label.slice(0, -2)}…`;
    }
    context.fillText(label, margin + px(9), baseline);
    context.textAlign = "right";
    context.fillText(time, right, baseline);
    context.textAlign = "left";
  });
  if (content.episodes.length > listed.length) {
    context.fillText(`+${content.episodes.length - listed.length}`, margin + px(9), y + lineHeight * (listed.length + 1));
  }
  context.font = font(content, px(3.2), 400);
  context.fillStyle = content.accent;
  context.fillText(content.specs, margin, height - margin);
  return canvas;
}

/** Round label with the hub left blank; outside the circle is transparent. */
export async function renderLabel(content: CoverContent): Promise<HTMLCanvasElement> {
  const size = px(LABEL_MM);
  const canvas = document.createElement("canvas");
  canvas.width = size;
  canvas.height = size;
  const context = canvas.getContext("2d");
  if (!context) return canvas;
  const center = size / 2;
  context.save();
  context.beginPath();
  context.arc(center, center, center, 0, Math.PI * 2);
  context.clip();
  context.fillStyle = content.background;
  context.fillRect(0, 0, size, size);
  const first = content.episodes.find((episode) => episode.preview)?.preview;
  const image = first ? await loadImage(first) : null;
  if (image) {
    context.globalAlpha = 0.35;
    drawCover(context, image, 0, 0, size, size);
    context.globalAlpha = 1;
  }
  context.restore();

  const hub = px(LABEL_HUB_MM) / 2;
  context.textAlign = "center";
  context.fillStyle = content.text;
  // Title above the hub, in at most two lines.
  context.textBaseline = "alphabetic";
  const probeSize = px(8);
  context.font = font(content, probeSize, 700);
  const twoLines = context.measureText(content.title).width > size * 0.62;
  drawTitle(context, content, content.title, center, center - hub - px(twoLines ? 24 : 17), size * 0.62, probeSize);
  if (content.subtitle.trim()) {
    const subtitleSize = fitText(context, content, content.subtitle, px(4.5), size * 0.6, 400);
    context.font = font(content, subtitleSize, 400);
    context.fillStyle = content.accent;
    context.fillText(content.subtitle, center, center - hub - px(3));
  }
  context.font = font(content, px(3.4), 600);
  context.fillStyle = content.accent;
  context.fillText(content.specs, center, center + hub + px(7));

  // Hub: blank (white) with the hole cut out.
  context.beginPath();
  context.arc(center, center, hub, 0, Math.PI * 2);
  context.fillStyle = "#ffffff";
  context.fill();
  context.globalCompositeOperation = "destination-out";
  context.beginPath();
  context.arc(center, center, px(LABEL_HOLE_MM) / 2, 0, Math.PI * 2);
  context.fill();
  context.globalCompositeOperation = "source-over";
  return canvas;
}
