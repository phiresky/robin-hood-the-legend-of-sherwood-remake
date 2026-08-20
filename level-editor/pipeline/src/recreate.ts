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
import type { AssetDescriptor, LibraryIndexEntry, MapDraft } from "@rle/shared";
import { datadirPath, editorRoot, libraryDir, workDir } from "./env";

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

  // offline composite in draw order (ascending world anchor Y)
  const ordered = [...assets].sort(
    (a, b) => a.origin[1] + a.anchor[1] - (b.origin[1] + b.anchor[1]),
  );
  const composite = ordered.map((a) => ({
    input: path.join(libraryDir, a.id, a.images.day),
    left: a.origin[0],
    top: a.origin[1],
  }));
  const recreated = await sharp({
    create: { width: W, height: H, channels: 3, background: { r: 42, g: 51, b: 36 } },
  })
    .composite(composite)
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
