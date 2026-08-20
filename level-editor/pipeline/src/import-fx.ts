// Import a map's ambient FX and patch sprites from the .rhs.d banks into the
// asset library — no segmentation needed, these are already discrete RGBA
// sprites. One asset per element (animations + patches from the proto level).
//
//   pnpm exec tsx src/import-fx.ts Leicester
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor, ProtoLevel } from "@rle/shared";
import { datadirPath } from "./env";
import { fxTopLeft, loadFxSprite, loadKeyedFxPng } from "./fx";
import { writeAsset } from "./library";
import { slugify } from "./extract-core";

async function main() {
  const map = process.argv[2];
  if (!map) throw new Error("usage: tsx src/import-fx.ts <MapName>");

  const levelsDir = path.join(datadirPath(), "Data", "Levels");
  const level: ProtoLevel = JSON.parse(
    await fs.readFile(path.join(levelsDir, `${map}.rhp.json`), "utf8"),
  );

  interface Item {
    bank: string;
    profile: string;
    x: number;
    y: number;
    elevation: number;
    kind: "fx" | "patch";
  }
  const items: Item[] = [];
  for (const a of level.animations) {
    const s = a.sprite;
    items.push({
      bank: s.frame_profile_name,
      profile: s.profile_name,
      x: s.position_x,
      y: s.position_y,
      elevation: s.elevation,
      kind: "fx",
    });
  }
  for (const p of level.patches) {
    const s = p.element_fx.sprite;
    if (s.frame_profile_name === "pixel_vert") continue; // engine dummy
    items.push({
      bank: s.frame_profile_name,
      profile: s.profile_name,
      x: s.position_x,
      y: s.position_y,
      elevation: s.elevation,
      kind: "patch",
    });
  }

  let written = 0;
  const misses: string[] = [];
  for (const item of items) {
    const fx = await loadFxSprite("Day", item.bank, item.profile);
    if (!fx) {
      misses.push(`${item.bank}/${item.profile}`);
      continue;
    }
    const png = await loadKeyedFxPng(fx.framePath);
    const meta = await sharp(png).metadata();
    const w = meta.width!;
    const h = meta.height!;
    const [left, top] = fxTopLeft(fx, item.x, item.y, item.elevation);

    // mask = alpha channel of the sprite
    const maskPng = await sharp(png).ensureAlpha().extractChannel(3).png().toBuffer();

    // strip the map prefix the original profile names carry ("Leicester - fumee01")
    const short = item.profile.replace(new RegExp(`^${map}\\s*-\\s*`, "i"), "");
    const id = `${map.toLowerCase()}-${item.kind}-${slugify(short)}`;

    const desc: AssetDescriptor = {
      id,
      name: short,
      tags: [item.kind, "animated"],
      scale_class: "unique",
      source: {
        map,
        ambiance: "Day",
        bbox: [left, top, w, h],
        extraction: { tool: "rhs-import" },
      },
      origin: [left, top],
      // sprite map anchor = position + center; in asset-local coords that's
      // center - frame offset. Also the engine's draw-order sort key.
      anchor: [Math.round(fx.centerX - fx.offsetX), Math.round(fx.centerY - fx.offsetY)],
      images: { day: "day.png", mask: "mask.png" },
      volumes: { sight_obstacles: [] },
      motion: { obstacles: [], walkable: [] },
      fx: {
        bank: item.bank,
        profile: item.profile,
        action: fx.action,
        frame_count: fx.frameCount,
        position: [item.x, item.y],
        elevation: item.elevation,
        hotspot: [fx.hotspotX, fx.hotspotY],
      },
    };
    await writeAsset(desc, { "day.png": png, "mask.png": maskPng });
    written++;
  }
  console.log(`imported ${written}/${items.length} fx/patch sprites`);
  if (misses.length) console.log(`missing profiles: ${misses.join(", ")}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
