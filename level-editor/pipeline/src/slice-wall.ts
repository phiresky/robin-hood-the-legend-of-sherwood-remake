// Cut a clean tileable slice out of a wall asset: a central vertical band
// with straight left/right cut edges, so repeated stamps butt seamlessly.
//
//   tsx src/slice-wall.ts <asset-id> [sliceWidthPx]
//
// Writes <asset-id>-slice with the same wall_direction_deg; anchor is the
// slice's bottom-center on the mask baseline.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor } from "@rle/shared";
import { libraryDir } from "./env";
import { writeAsset } from "./library";

async function main() {
  const id = process.argv[2];
  if (!id) throw new Error("usage: tsx src/slice-wall.ts <asset-id> [sliceWidthPx]");
  const srcDir = path.join(libraryDir, id);
  const desc: AssetDescriptor = JSON.parse(
    await fs.readFile(path.join(srcDir, "asset.json"), "utf8"),
  );
  const day = sharp(path.join(srcDir, desc.images.day));
  const { width: W, height: H } = await day.metadata();
  const sliceW = Number(process.argv[3] ?? Math.round(W! / 3));
  const x0 = Math.round((W! - sliceW) / 2);

  const images: Record<string, Buffer> = {};
  for (const key of ["day", "fog", "night"] as const) {
    const file = desc.images[key];
    if (!file) continue;
    images[`${key}.png`] = await sharp(path.join(srcDir, file))
      .extract({ left: x0, top: 0, width: sliceW, height: H! })
      .png()
      .toBuffer();
  }
  images["mask.png"] = await sharp(path.join(srcDir, desc.images.mask))
    .extract({ left: x0, top: 0, width: sliceW, height: H! })
    .png()
    .toBuffer();

  // trim empty rows and find the baseline (lowest fg row)
  const maskRaw = await sharp(images["mask.png"]!).extractChannel(0).raw().toBuffer();
  let top = H!,
    bottom = -1;
  for (let y = 0; y < H!; y++) {
    for (let x = 0; x < sliceW; x++) {
      if (maskRaw[y * sliceW + x]! > 127) {
        top = Math.min(top, y);
        bottom = Math.max(bottom, y);
        break;
      }
    }
  }
  if (bottom < 0) throw new Error("slice mask is empty");
  const h = bottom - top + 1;
  for (const name of Object.keys(images)) {
    images[name] = await sharp(images[name]!)
      .extract({ left: 0, top, width: sliceW, height: h })
      .png()
      .toBuffer();
  }

  const sliceId = `${id}-slice`;
  const sliceDesc: AssetDescriptor = {
    ...desc,
    id: sliceId,
    name: `${desc.name} slice`,
    tags: [...desc.tags, "slice"],
    source: {
      ...desc.source,
      bbox: [desc.source.bbox[0] + x0, desc.source.bbox[1] + top, sliceW, h],
      extraction: { tool: "wall-slice", prompt: `sliced from ${id}` },
    },
    origin: [desc.origin[0] + x0, desc.origin[1] + top],
    anchor: [Math.round(sliceW / 2), h - 1],
    images: {
      day: "day.png",
      mask: "mask.png",
      fog: desc.images.fog ? "fog.png" : undefined,
      night: desc.images.night ? "night.png" : undefined,
    },
    // slices carry no clipped gameplay metadata; author in editor if needed
    volumes: { sight_obstacles: [] },
    motion: { obstacles: [], walkable: [] },
    jump_zones: [],
    jump_line_pairs: [],
    lifts: [],
    material_sectors: [],
    occlusion_masks: [],
  };
  const dir = await writeAsset(sliceDesc, images);
  console.log(`wrote ${dir} (${sliceW}x${h}, direction ${desc.wall_direction_deg}°)`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
