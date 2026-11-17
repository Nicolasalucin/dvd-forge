/** The empty background a new project starts with: the project's own
 * `assets/background.png` with no file on disk, size 0 and an all-zero hash
 * (`MediaAsset::is_placeholder` in kinodisc-domain). Imported files also get
 * an all-zero hash, so the source and path are part of the test. The menu is
 * drawn on its background colour instead; never read or listed. */
export function isPlaceholderAsset(asset: {
  bytes: number;
  sha256: string;
  source: { kind: string };
  relativePath?: string | null;
}): boolean {
  return (
    asset.source.kind === "project" &&
    asset.relativePath === "assets/background.png" &&
    asset.bytes === 0 &&
    /^0+$/.test(asset.sha256)
  );
}
