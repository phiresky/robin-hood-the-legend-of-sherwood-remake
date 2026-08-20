// Litmus test: recreate a source map as a MapDraft from the extracted assets,
// and render an offline composite PNG to visualize coverage gaps.
//
//   pnpm exec tsx src/recreate.ts Leicester
//
// Writes ../drafts/<map>-recreation.json and ../work/<map>-recreated.png
// (plus a side-by-side comparison against the original map).
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor, LibraryIndexEntry, MapDraft, ProtoLevel } from "@rle/shared";
import { datadirPath, editorRoot, libraryDir, workDir } from "./env";
import { renderTerrain, type TerrainSpec } from "./terrain";


async function main() {
  const map = process.argv[2];
  if (!map) throw new Error("usage: tsx src/recreate.ts <MapName>");

  const index: LibraryIndexEntry[] = JSON.parse(
    await fs.readFile(path.join(libraryDir, "index.json"), "utf8"),
  );
  const assets: AssetDescriptor[] = [];
  for (const e of index) {
    if (e.source_map !== map) continue;
    assets.push(
      JSON.parse(await fs.readFile(path.join(libraryDir, e.id, "asset.json"), "utf8")),
    );
  }
  if (assets.length === 0) throw new Error(`no assets from map ${map} in the library`);

  // original map dimensions
  const levelsDir = path.join(datadirPath(), "Data", "Levels");
  const dayDir = path.join(levelsDir, "Day");
  const dayFile = (await fs.readdir(dayDir)).find(
    (f) => f.toLowerCase() === `${map.toLowerCase()}.map.png`,
  );
  if (!dayFile) throw new Error(`no Day map png for ${map}`);
  const origPath = path.join(dayDir, dayFile);
  const meta = await sharp(origPath).metadata();
  const W = meta.width!;
  const H = meta.height!;

  // draft: every asset at its original source position
  const draft: MapDraft = {
    version: 1,
    name: `${map} recreation`,
    size: [W, H],
    background_color: "#2a3324",
    placements: assets.map((a) => ({ asset: a.id, pos: a.origin })),
    notes: `Auto-generated litmus test: all ${assets.length} ${map} library assets at their source positions.`,
  };
  const draftsDir = path.join(editorRoot, "drafts");
  await fs.mkdir(draftsDir, { recursive: true });
  const draftPath = path.join(draftsDir, `${map.toLowerCase()}-recreation.json`);
  await fs.writeFile(draftPath, JSON.stringify(draft, null, 2));
  console.log(`draft: ${draftPath} (${assets.length} placements)`);

  // offline composite in draw order: static cutouts first, then FX/patch
  // sprites (in-game, patch roofs draw over the background/buildings), each
  // group sorted ascending by world anchor Y
  const byAnchorY = (a: AssetDescriptor, b: AssetDescriptor) =>
    a.origin[1] + a.anchor[1] - (b.origin[1] + b.anchor[1]);
  const ordered = [
    ...assets.filter((a) => !a.fx).sort(byAnchorY),
    ...assets.filter((a) => a.fx).sort(byAnchorY),
  ];
  const composite = ordered
    .filter((a) => a.scale_class !== "texture") // swatches feed the terrain layer
    .map((a) => ({
      input: path.join(libraryDir, a.id, a.images.day),
      left: a.origin[0],
      top: a.origin[1],
    }));
  const level: ProtoLevel = JSON.parse(
    await fs.readFile(path.join(levelsDir, `${map}.rhp.json`), "utf8"),
  );
  let spec: TerrainSpec = {};
  try {
    spec = JSON.parse(
      await fs.readFile(
        path.join(editorRoot, "drafts", `${map.toLowerCase()}-terrain.json`),
        "utf8",
      ),
    );
  } catch {
    // no authored terrain spec for this map yet
  }
  const prefix = map.toLowerCase();
  const terrain = await renderTerrain(level, W, H, spec, {
    grass: `${prefix}-grass-swatch`,
    dirt: `${prefix}-courtyard-dirt-swatch`,
    road: `${prefix}-dirt-road-swatch`,
    canopy: `${prefix}-forest-canopy-swatch`,
    water: `${prefix}-moat-water-swatch`,
  });

  // tree scatter along forest edges, behind everything else
  const scatter: { input: Buffer; left: number; top: number }[] = [];
  if (terrain) {
    try {
      const treePng = await sharp(path.join(libraryDir, `${prefix}-orchard-tree-1`, "day.png"))
        .resize({ width: 96 })
        .png()
        .toBuffer();
      const tm = await sharp(treePng).metadata();
      for (const [x, y] of terrain.scatterPoints) {
        const left = Math.round(x - tm.width! / 2);
        const top = Math.round(y - tm.height! + 8);
        if (left < 0 || top < 0 || left + tm.width! > W || top + tm.height! > H) continue;
        scatter.push({ input: treePng, left, top });
      }
    } catch {
      // no tree asset to scatter
    }
  }

  const base = terrain
    ? sharp(terrain.png)
    : sharp({
        create: { width: W, height: H, channels: 3, background: { r: 42, g: 51, b: 36 } },
      });
  const recreated = await base.composite([...scatter, ...composite]).png().toBuffer();
  const outPath = path.join(workDir, `${map.toLowerCase()}-recreated.png`);
  await fs.writeFile(outPath, recreated);

  // side-by-side with the original, half scale
  // sharp runs resize before composite in one pipeline, so composite first
  const sideFull = await sharp({
    create: { width: W, height: H * 2 + 16, channels: 3, background: { r: 20, g: 20, b: 20 } },
  })
    .composite([
      { input: origPath, left: 0, top: 0 },
      { input: recreated, left: 0, top: H + 16 },
    ])
    .png()
    .toBuffer();
  const side = await sharp(sideFull).resize(Math.min(W, 1600)).png().toBuffer();
  const sidePath = path.join(workDir, `${map.toLowerCase()}-comparison.png`);
  await fs.writeFile(sidePath, side);
  console.log(`renders: ${outPath}, ${sidePath}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
