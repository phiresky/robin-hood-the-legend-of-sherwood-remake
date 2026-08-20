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
import type {
  AssetDescriptor,
  LibraryIndexEntry,
  MapDraft,
  Point,
  ProtoLevel,
} from "@rle/shared";
import { datadirPath, editorRoot, libraryDir, workDir } from "./env";

interface Swatch {
  data: Buffer; // raw RGB
  width: number;
  height: number;
}

async function loadSwatch(id: string): Promise<Swatch | null> {
  try {
    const p = path.join(libraryDir, id, "day.png");
    const img = sharp(p).removeAlpha();
    const { width, height } = await img.metadata();
    return { data: await img.raw().toBuffer(), width: width!, height: height! };
  } catch {
    return null;
  }
}

function fillTiled(canvas: Buffer, W: number, sw: Swatch, x0: number, x1: number, y: number) {
  const sy = ((y % sw.height) + sw.height) % sw.height;
  for (let x = x0; x < x1; x++) {
    const sx = ((x % sw.width) + sw.width) % sw.width;
    const si = (sy * sw.width + sx) * 3;
    const di = (y * W + x) * 3;
    canvas[di] = sw.data[si]!;
    canvas[di + 1] = sw.data[si + 1]!;
    canvas[di + 2] = sw.data[si + 2]!;
  }
}

/** scanline fill of a polygon with a tiled swatch (even-odd rule) */
function fillPolygon(canvas: Buffer, W: number, H: number, poly: Point[], sw: Swatch) {
  if (poly.length < 3) return;
  let minY = Infinity,
    maxY = -Infinity;
  for (const [, y] of poly) {
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  for (let y = Math.max(0, Math.ceil(minY)); y <= Math.min(H - 1, Math.floor(maxY)); y++) {
    const xs: number[] = [];
    for (let i = 0; i < poly.length; i++) {
      const [x1, y1] = poly[i]!;
      const [x2, y2] = poly[(i + 1) % poly.length]!;
      if (y1 === y2) continue;
      if ((y >= y1 && y < y2) || (y >= y2 && y < y1)) {
        xs.push(x1 + ((y - y1) / (y2 - y1)) * (x2 - x1));
      }
    }
    xs.sort((a, b) => a - b);
    for (let k = 0; k + 1 < xs.length; k += 2) {
      const x0 = Math.max(0, Math.ceil(xs[k]!));
      const x1 = Math.min(W, Math.floor(xs[k + 1]!) + 1);
      if (x1 > x0) fillTiled(canvas, W, sw, x0, x1, y);
    }
  }
}

/** grass base + material-sector polygons filled with texture swatches */
async function renderTerrain(level: ProtoLevel, W: number, H: number): Promise<Buffer | null> {
  const grass = await loadSwatch("leicester-grass-swatch");
  if (!grass) return null;
  const byMaterial: Record<number, Swatch | null> = {
    0: await loadSwatch("leicester-courtyard-dirt-swatch"), // Ground
    2: await loadSwatch("leicester-courtyard-dirt-swatch"), // Stone (no swatch yet)
    3: grass, // Grass
    4: await loadSwatch("leicester-forest-canopy-swatch"), // Leaves
    5: await loadSwatch("leicester-moat-water-swatch"), // Water
  };
  const canvas = Buffer.alloc(W * H * 3);
  for (let y = 0; y < H; y++) fillTiled(canvas, W, grass, 0, W, y);
  // water last so grass sectors can't overdraw the moat
  const sectors = [...level.material_sectors].sort(
    (a, b) => Number(a.material === 5) - Number(b.material === 5),
  );
  for (const ms of sectors) {
    const sw = byMaterial[ms.material];
    if (sw) fillPolygon(canvas, W, H, ms.polygon.points, sw);
  }
  return sharp(canvas, { raw: { width: W, height: H, channels: 3 } })
    .png()
    .toBuffer();
}

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
  const terrain = await renderTerrain(level, W, H);
  const base = terrain
    ? sharp(terrain)
    : sharp({
        create: { width: W, height: H, channels: 3, background: { r: 42, g: 51, b: 36 } },
      });
  const recreated = await base.composite(composite).png().toBuffer();
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
