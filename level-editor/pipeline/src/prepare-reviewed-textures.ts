/** Convert a reviewed source-only packet without rerendering its approved pixels. */
import fs from "node:fs/promises";
import path from "node:path";
import crypto from "node:crypto";
import sharp from "sharp";

const [reviewArg, outputArg, geometryRevision] = process.argv.slice(2);
if (!reviewArg || !outputArg || !geometryRevision)
  throw new Error("Usage: node prepare-reviewed-textures.ts <review-dir> <new-output-dir> <geometry-revision>");
const review = path.resolve(reviewArg), output = path.resolve(outputArg);
const originalManifest = await fs.readFile(path.join(review, "views.json"));
const manifest = JSON.parse(originalManifest.toString()) as {
  asset_id: string; tile_size: [number, number]; source_image: string;
  source_mask_manifest?: string | null; source_mask_evidence?: Record<string,string> | null;
  views: Array<Record<string, unknown> & { index: number }>;
};
const [width, height] = manifest.tile_size;
if (Boolean(manifest.source_mask_manifest) !== Boolean(manifest.source_mask_evidence))
  throw new Error("Source mask assignments and immutable evidence must travel together");
for (const [file, expected] of Object.entries(manifest.source_mask_evidence ?? {}))
  if (crypto.createHash("sha256").update(await fs.readFile(file)).digest("hex") !== expected)
    throw new Error(`Reviewed source-mask evidence changed: ${file}`);
if (!Number.isInteger(width) || !Number.isInteger(height) || width <= 0 || height <= 0 ||
    manifest.views.length !== 8 || manifest.views.some((view, index) => view.index !== index))
  throw new Error("Expected eight ordered review views with valid tile dimensions");
const input = await fs.readFile(path.join(review, "textured.png"));
const size = await sharp(input).metadata();
if (size.width !== width * 4 || size.height !== height * 2)
  throw new Error("Review sheet dimensions do not match its cameras");
// Read all evidence before creating an output directory. Unknownness comes from
// explicit ownership and silhouette buffers, never a gray-pixel classifier.
const tiles = await Promise.all(manifest.views.map(async view => {
  const prefix = `views/view-${view.index}`;
  const ownershipBytes = await fs.readFile(path.join(review, `${prefix}-known.png`));
  if ((manifest.source_mask_manifest && !view.ownership_sha256) ||
      (view.ownership_sha256 && crypto.createHash("sha256").update(ownershipBytes).digest("hex") !== view.ownership_sha256))
    throw new Error(`View ${view.index} source ownership changed or lacks an immutable hash`);
  const [solid, known] = await Promise.all(["solid", "known"].map(async kind => {
    const result = await sharp(path.join(review, `${prefix}-${kind}.png`)).ensureAlpha()
      .raw().toBuffer({ resolveWithObject: true });
    if (result.info.width !== width || result.info.height !== height)
      throw new Error(`View ${view.index} ${kind} dimensions differ from the approved framing`);
    return result.data;
  }));
  if (!solid || !known) throw new Error("Missing ownership or silhouette buffer");
  const pixels = Buffer.alloc(width * height * 4, 255);
  for (let offset = 0; offset < pixels.length; offset += 4)
    if (solid[offset + 3]! > 0 && known[offset]! < 128) pixels[offset + 3] = 0;
  return pixels;
}));
await fs.mkdir(output, { recursive: false });
await fs.mkdir(path.join(output, "views"));
await fs.writeFile(path.join(output, "input.png"), input);
const views = [];
for (const [index, view] of manifest.views.entries()) {
  const inputName = `views/view-${index}-input.png`, maskName = `views/view-${index}-mask.png`;
  await fs.copyFile(path.join(review, `views/view-${index}-textured.png`), path.join(output, inputName));
  await sharp(tiles[index]!, { raw: { width, height, channels: 4 } }).png().toFile(path.join(output, maskName));
  views.push({ ...view, input: inputName, mask: maskName,
    crop: { left: index % 4 * width, top: Math.floor(index / 4) * height, width, height } });
}
const sheetMask = Buffer.alloc(width * height * 8 * 4, 255);
for (const [index, tile] of tiles.entries())
  for (let row = 0; row < height; row++) {
    const start = ((Math.floor(index / 4) * height + row) * width * 4 + index % 4 * width) * 4;
    tile.copy(sheetMask, start, row * width * 4, (row + 1) * width * 4);
  }
await sharp(sheetMask, { raw: { width: width * 4, height: height * 2, channels: 4 } })
  .png().toFile(path.join(output, "mask.png"));
const inputHash = crypto.createHash("sha256").update(input).digest("hex");
await fs.writeFile(path.join(output, "views.json"), JSON.stringify({ ...manifest, views,
  layout: { columns: 4, rows: 2, width: width * 4, height: height * 2 },
  reviewed_packet: review, geometry_revision: geometryRevision,
  reviewed_manifest_sha256: crypto.createHash("sha256").update(originalManifest).digest("hex"),
  input_sha256: inputHash }, null, 2) + "\n");
await fs.writeFile(path.join(output, "approval.json"), JSON.stringify({ status: "pending",
  approved_by: null, asset_id: manifest.asset_id, geometry_revision: geometryRevision,
  input_sha256: inputHash }, null, 2) + "\n");
console.log(JSON.stringify({ output, asset: manifest.asset_id, input_sha256: inputHash,
  approval: "pending", mask_sent_to_api: false }));
