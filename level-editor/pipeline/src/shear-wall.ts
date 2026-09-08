// Synthesize a diagonal wall slice by vertically shearing a horizontal one:
// column x shifts down by tan(angle)*x. Height and texture stay consistent
// with the source family by construction.
//   tsx src/shear-wall.ts <slice-asset-id> <angleDeg>
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor } from "@rle/shared";
import { libraryDir } from "./env";
import { writeAsset } from "./library";

async function shearPng(png: Buffer, slope: number): Promise<Buffer> {
  const img = sharp(png).ensureAlpha();
  const { width: w, height: h } = await img.metadata();
  const raw = await img.raw().toBuffer();
  const extra = Math.ceil(Math.abs(slope) * (w! - 1));
  const H = h! + extra;
  const out = Buffer.alloc(w! * H * 4);
  for (let x = 0; x < w!; x++) {
    const shift = Math.round(slope >= 0 ? slope * x : slope * (x - (w! - 1)));
    for (let y = 0; y < h!; y++) {
      const si = (y * w! + x) * 4;
      const dy = y + shift;
      if (dy < 0 || dy >= H) continue;
      const di = (dy * w! + x) * 4;
      out[di] = raw[si]!;
      out[di + 1] = raw[si + 1]!;
      out[di + 2] = raw[si + 2]!;
      out[di + 3] = raw[si + 3]!;
    }
  }
  return sharp(out, { raw: { width: w!, height: H, channels: 4 } }).png().toBuffer();
}

async function main() {
  const id = process.argv[2];
  const angle = Number(process.argv[3]);
  if (!id || Number.isNaN(angle)) throw new Error("usage: tsx src/shear-wall.ts <id> <angleDeg>");
  const srcDir = path.join(libraryDir, id);
  const desc: AssetDescriptor = JSON.parse(
    await fs.readFile(path.join(srcDir, "asset.json"), "utf8"),
  );
  const baseDeg = desc.wall_direction_deg ?? 0;
  const slope = Math.tan(((angle - baseDeg) * Math.PI) / 180);
  const images: Record<string, Buffer> = {};
  for (const [key, file] of Object.entries(desc.images)) {
    if (!file) continue;
    images[`${key}.png`] = await shearPng(
      await fs.readFile(path.join(srcDir, file)),
      slope,
    );
  }
  const meta = await sharp(images["day.png"]!).metadata();
  const w = desc.source.bbox[2];
  const anchorShift = Math.round(
    slope >= 0 ? slope * desc.anchor[0] : slope * (desc.anchor[0] - (w - 1)),
  );
  const suffix = `sh${angle >= 0 ? "p" : "m"}${Math.abs(angle)}`;
  const sheared: AssetDescriptor = {
    ...desc,
    id: `${id}-${suffix}`,
    name: `${desc.name} ${angle}°`,
    tags: [...desc.tags, "sheared"],
    anchor: [desc.anchor[0], desc.anchor[1] + anchorShift],
    wall_direction_deg: angle,
    source: { ...desc.source, bbox: [desc.source.bbox[0], desc.source.bbox[1], w, meta.height!] },
    images: Object.fromEntries(
      Object.entries(desc.images).map(([k, v]) => [k, v ? `${k}.png` : undefined]),
    ) as AssetDescriptor["images"],
  };
  const dir = await writeAsset(sheared, images);
  console.log(`wrote ${dir} (${w}x${meta.height}, direction ${angle}°)`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
