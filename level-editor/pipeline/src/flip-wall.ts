// Create a horizontally mirrored variant of a wall slice (direction negates).
//   tsx src/flip-wall.ts <slice-asset-id>
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor } from "@rle/shared";
import { libraryDir } from "./env";
import { writeAsset } from "./library";

async function main() {
  const id = process.argv[2];
  if (!id) throw new Error("usage: tsx src/flip-wall.ts <asset-id>");
  const srcDir = path.join(libraryDir, id);
  const desc: AssetDescriptor = JSON.parse(
    await fs.readFile(path.join(srcDir, "asset.json"), "utf8"),
  );
  const images: Record<string, Buffer> = {};
  for (const [key, file] of Object.entries(desc.images)) {
    if (!file) continue;
    images[`${key}.png`] = await sharp(path.join(srcDir, file)).flop().png().toBuffer();
  }
  const flipId = `${id}-flip`;
  const [, , w] = desc.source.bbox;
  const flipped: AssetDescriptor = {
    ...desc,
    id: flipId,
    name: `${desc.name} (mirrored)`,
    tags: [...desc.tags, "mirrored"],
    anchor: [w - desc.anchor[0], desc.anchor[1]],
    wall_direction_deg:
      desc.wall_direction_deg === undefined ? undefined : -desc.wall_direction_deg,
    images: Object.fromEntries(
      Object.entries(desc.images).map(([k, v]) => [k, v ? `${k}.png` : undefined]),
    ) as AssetDescriptor["images"],
  };
  const dir = await writeAsset(flipped, images);
  console.log(`wrote ${dir} (direction ${flipped.wall_direction_deg}°)`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
