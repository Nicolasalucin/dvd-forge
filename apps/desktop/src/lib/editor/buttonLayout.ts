import type { ButtonStyleKind } from "../api/bindings";

/**
 * Geometría compartida entre la previsualización Konva y la exportación PNG.
 * Mantener ambas rutas alineadas garantiza que lo que se ve en el editor es
 * exactamente lo que se rasteriza en `background.png`.
 */
export interface LayoutRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ButtonLayout {
  thumbnail: LayoutRect | null;
  label: LayoutRect;
  align: "center" | "left";
}

const PADDING = 8;
const CARD_LABEL_HEIGHT = 28;

export function buttonStyleLayout(
  style: ButtonStyleKind,
  width: number,
  height: number,
  hasThumbnail: boolean,
): ButtonLayout {
  if (style === "thumbnailCard") {
    if (!hasThumbnail) {
      return {
        thumbnail: null,
        label: {
          x: PADDING,
          y: PADDING,
          w: Math.max(8, width - PADDING * 2),
          h: Math.max(8, height - PADDING * 2),
        },
        align: "center",
      };
    }
    const labelHeight = Math.min(CARD_LABEL_HEIGHT, Math.max(16, height * 0.22));
    const innerWidth = Math.max(8, width - PADDING * 2);
    const maxThumbHeight = Math.max(8, height - PADDING * 3 - labelHeight);
    const thumbWidth = Math.max(
      8,
      Math.min(innerWidth, Math.round((maxThumbHeight * 16) / 9)),
    );
    const thumbHeight = Math.max(8, Math.round((thumbWidth * 9) / 16));
    return {
      thumbnail: {
        x: Math.round((width - thumbWidth) / 2),
        y: PADDING,
        w: thumbWidth,
        h: thumbHeight,
      },
      label: {
        x: PADDING,
        y: Math.min(height - PADDING - 8, PADDING + thumbHeight + 4),
        w: innerWidth,
        h: Math.max(
          8,
          height -
            PADDING -
            Math.min(height - PADDING - 8, PADDING + thumbHeight + 4),
        ),
      },
      align: "center",
    };
  }

  if (style === "textOnly" || style === "hotspot" || style === "glassPill") {
    return {
      thumbnail: null,
      label: {
        x: style === "glassPill" ? 18 : 12,
        y: PADDING,
        w: Math.max(8, width - (style === "glassPill" ? 36 : 24)),
        h: Math.max(8, height - PADDING * 2),
      },
      align: style === "hotspot" ? "left" : "center",
    };
  }

  if (style === "playBadge") {
    const iconSpace = Math.min(48, Math.max(32, width * 0.18));
    return {
      thumbnail: null,
      label: {
        x: iconSpace,
        y: PADDING,
        w: Math.max(8, width - iconSpace - PADDING),
        h: Math.max(8, height - PADDING * 2),
      },
      align: "left",
    };
  }

  if (!hasThumbnail) {
    return {
      thumbnail: null,
      label: {
        x: 12,
        y: PADDING,
        w: Math.max(8, width - 24),
        h: Math.max(8, height - PADDING * 2),
      },
      align: "center",
    };
  }

  const availableHeight = Math.max(8, height - PADDING * 2);
  const maxThumbWidth = Math.max(16, Math.floor(width * 0.42));
  const thumbWidth = Math.max(8, Math.min(maxThumbWidth, Math.round((availableHeight * 4) / 3)));
  const labelX = PADDING + thumbWidth + PADDING;
  return {
    thumbnail: {
      x: PADDING,
      y: PADDING,
      w: thumbWidth,
      h: availableHeight,
    },
    label: {
      x: labelX,
      y: PADDING,
      w: Math.max(8, width - labelX - PADDING),
      h: availableHeight,
    },
    align: "left",
  };
}

/** Compatibilidad con llamadas antiguas basadas solo en miniatura. */
export function buttonLayout(
  width: number,
  height: number,
  hasThumbnail: boolean,
): ButtonLayout {
  return buttonStyleLayout("classicFrame", width, height, hasThumbnail);
}
