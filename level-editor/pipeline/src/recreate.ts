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
import { expandWallRun, expandWallRunDirectional, type WallSegmentSpec } from "@rle/shared";


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

  // draft: every asset at its original source position, plus authored walls
  const draft: MapDraft = {
    version: 1,
    name: `${map} recreation`,
    size: [W, H],
    background_color: "#2a3324",
    placements: assets
      .filter((a) => a.scale_class !== "texture" && a.scale_class !== "spline-segment")
      .map((a) => ({ asset: a.id, pos: a.origin })),
    walls: spec.walls,
    terrain: {
      ...spec,
      walls: undefined, // already lifted to draft.walls
      swatches: {
        grass: `${prefix}-grass-swatch`,
        dirt: `${prefix}-courtyard-dirt-swatch`,
        road: `${prefix}-dirt-road-swatch`,
        canopy: `${prefix}-forest-canopy-swatch`,
        water: `${prefix}-moat-water-swatch`,
        ...spec.swatches,
      },
    },
    notes: `Auto-generated litmus test: all ${assets.length} ${map} library assets at their source positions.`,
  };
  const draftsDir = path.join(editorRoot, "drafts");
  await fs.mkdir(draftsDir, { recursive: true });
  const draftPath = path.join(draftsDir, `${map.toLowerCase()}-recreation.json`);
  await fs.writeFile(draftPath, JSON.stringify(draft, null, 2));
  console.log(`draft: ${draftPath} (${assets.length} placements)`);

  // draw order: static cutouts and wall stamps interleaved by world anchor Y,
  // then FX/patch sprites on top (in-game, patch roofs draw over buildings)
  interface Item {
    input: string;
    left: number;
    top: number;
    sortY: number;
  }
  const byId = new Map(assets.map((a) => [a.id, a]));
  // spline-segments are stitching material for wall runs, not scene objects
  const statics: Item[] = assets
    .filter((a) => !a.fx && a.scale_class !== "texture" && a.scale_class !== "spline-segment")
    .map((a) => ({
      input: path.join(libraryDir, a.id, a.images.day),
      left: a.origin[0],
      top: a.origin[1],
      sortY: a.origin[1] + a.anchor[1],
    }));
  for (const run of spec.walls ?? []) {
    if (run.segment_set?.length) {
      const segs: WallSegmentSpec[] = [];
      for (const id of run.segment_set) {
        const a = byId.get(id);
        if (!a) {
          console.warn(`wall segment set references unknown asset ${id}`);
          continue;
        }
        segs.push({
          id,
          size: [a.source.bbox[2], a.source.bbox[3]],
          anchor: a.anchor,
          directionDeg: a.wall_direction_deg ?? 0,
        });
      }
      for (const s of expandWallRunDirectional(run.points, segs, run.spacing)) {
        const a = byId.get(s.asset)!;
        statics.push({
          input: path.join(libraryDir, a.id, a.images.day),
          left: s.pos[0],
          top: s.pos[1],
          sortY: s.sortY,
        });
      }
      continue;
    }
    const wa = byId.get(run.asset);
    if (!wa) {
      console.warn(`wall run references unknown asset ${run.asset}`);
      continue;
    }
    const stamps = expandWallRun(
      run.points,
      [wa.source.bbox[2], wa.source.bbox[3]],
      wa.anchor,
      run.spacing,
    );
    for (const s of stamps) {
      statics.push({
        input: path.join(libraryDir, wa.id, wa.images.day),
        left: s.pos[0],
        top: s.pos[1],
        sortY: s.sortY,
      });
    }
  }
  statics.sort((a, b) => a.sortY - b.sortY);
  const fxItems: Item[] = assets
    .filter((a) => a.fx)
    .sort((a, b) => a.origin[1] + a.anchor[1] - (b.origin[1] + b.anchor[1]))
    .map((a) => ({
      input: path.join(libraryDir, a.id, a.images.day),
      left: a.origin[0],
      top: a.origin[1],
      sortY: 0,
    }));
  const composite = [...statics, ...fxItems].map(({ input, left, top }) => ({
    input,
    left,
    top,
  }));
  const level: ProtoLevel = JSON.parse(
    await fs.readFile(path.join(levelsDir, `${map}.rhp.json`), "utf8"),
  );
  const terrain = await renderTerrain(level, W, H, spec, {
    grass: `${prefix}-grass-swatch`,
    dirt: `${prefix}-courtyard-dirt-swatch`,
    road: `${prefix}-dirt-road-swatch`,
    canopy: `${prefix}-forest-canopy-swatch`,
    water: `${prefix}-moat-water-swatch`,
  });

  // tree scatter in and along forests, behind everything else, y-sorted with
  // per-point variety (two tree assets, three sizes each)
  const scatter: { input: Buffer; left: number; top: number; sortY: number }[] = [];
  if (terrain) {
    const variants: { png: Buffer; w: number; h: number }[] = [];
    for (const id of [`${prefix}-orchard-tree-1`, `${prefix}-lone-tree`]) {
      for (const width of [88, 116, 148]) {
        try {
          const png = await sharp(path.join(libraryDir, id, "day.png"))
            .resize({ width })
            .png()
            .toBuffer();
          const m = await sharp(png).metadata();
          variants.push({ png, w: m.width!, h: m.height! });
        } catch {
          // asset missing
        }
      }
    }
    if (variants.length > 0) {
      terrain.scatterPoints.forEach(([x, y], i) => {
        const v = variants[Math.abs(x * 31 + y * 17 + i) % variants.length]!;
        const left = Math.round(x - v.w / 2);
        const top = Math.round(y - v.h + 8);
        if (left < 0 || top < 0 || left + v.w > W || top + v.h > H) return;
        scatter.push({ input: v.png, left, top, sortY: y });
      });
      scatter.sort((a, b) => a.sortY - b.sortY);
    }
  }

  const base = terrain
    ? sharp(terrain.png)
    : sharp({
        create: { width: W, height: H, channels: 3, background: { r: 42, g: 51, b: 36 } },
      });
  const recreated = await base
    .composite([...scatter.map(({ input, left, top }) => ({ input, left, top })), ...composite])
    .png()
    .toBuffer();
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
