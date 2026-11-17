/** The empty background a new project starts with: no file on disk, size 0,
 * all-zero hash (`MediaAsset::is_placeholder` in kinodisc-domain). The menu
 * is drawn on its background colour instead; never read or listed. */
export function isPlaceholderAsset(asset: { bytes: number; sha256: string }): boolean {
  return asset.bytes === 0 && /^0+$/.test(asset.sha256);
}
