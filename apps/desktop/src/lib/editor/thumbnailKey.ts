/**
 * Key of a button thumbnail in the preview tables. A thumbnail at frame 0
 * uses the asset id (the poster frame every screen already loads); scene
 * buttons point at a chapter frame and get their own entry.
 */
export interface ThumbnailRef {
  asset: string;
  inFrame?: number;
  fps?: { num: number; den: number };
}

export function thumbnailKey(thumbnail: ThumbnailRef): string {
  const frame = thumbnail.inFrame ?? 0;
  return frame > 0 ? `${thumbnail.asset}@${frame}` : thumbnail.asset;
}

/** Milliseconds of the thumbnail frame, or null for the poster frame. */
export function thumbnailTimeMs(thumbnail: ThumbnailRef): number | null {
  const frame = thumbnail.inFrame ?? 0;
  const fps = thumbnail.fps;
  if (frame <= 0 || !fps || fps.num <= 0) return null;
  return Math.round((frame * fps.den * 1000) / fps.num);
}
