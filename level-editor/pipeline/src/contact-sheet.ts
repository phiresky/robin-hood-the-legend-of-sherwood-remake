// Review grid of every reconstructed asset of a map: the "model over crop"
// panel of each work/<id>/fit.png, labelled with id and fit IoU, sorted by
// IoU ascending so the weakest fits come first.
//
//   node src/contact-sheet.ts --map york [--cell 220] [--cols 8]
//   -> work/<map>-scene/contact.png
import path from "node:path";
import sharp from "sharp";
import { readAssetDescriptor, readLibraryIndex } from "./library.ts";
import { libraryDir, workDir } from "./env.ts";
import { fileExists } from "./mesh.ts";

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const map = get("map");
  if (!map) throw new Error("usage: --map <name> [--cell px] [--cols n]");
  const cell = Number(get("cell") ?? 220);
  const cols = Number(get("cols") ?? 8);

  const index = await readLibraryIndex(libraryDir, true);
  const items: { id: string; iou: number; tilt: number; panel: Buffer }[] = [];
  for (const e of index) {
    if (e.source_map.toLowerCase() !== map.toLowerCase()) continue;
    const desc = await readAssetDescriptor(path.join(libraryDir, e.id, "asset.json"));
    if (!desc.model) continue;
    const sheet = path.join(workDir, e.id, "fit.png");
    if (!(await fileExists(sheet))) continue;
    const [, , cw, ch] = desc.model.extraction.crop;
    const panel = await sharp(sheet)
      .extract({ left: cw, top: 0, width: cw, height: ch })
      .resize(cell, cell, { fit: "contain", background: "#202020" })
      .png()
      .toBuffer();
    items.push({ id: e.id, iou: desc.model.fit_iou, tilt: 0, panel });
  }
  if (items.length === 0) throw new Error(`no reconstructed assets with review sheets for ${map}`);
  items.sort((a, b) => a.iou - b.iou);

  const rows = Math.ceil(items.length / cols);
  const label = 18;
  const composites: sharp.OverlayOptions[] = [];
  let svg = `<svg width="${cols * cell}" height="${rows * (cell + label)}" xmlns="http://www.w3.org/2000/svg">`;
  items.forEach((it, i) => {
    const x = (i % cols) * cell;
    const y = Math.floor(i / cols) * (cell + label);
    composites.push({ input: it.panel, left: x, top: y });
    const color = it.iou < 0.5 ? "#ff6060" : it.iou < 0.7 ? "#ffd060" : "#a0e0a0";
    svg += `<text x="${x + 4}" y="${y + cell + 13}" font-size="12" font-family="sans-serif" fill="${color}">${it.id.replace(`${map.toLowerCase()}-`, "")} IoU ${it.iou.toFixed(2)}</text>`;
  });
  svg += "</svg>";
  composites.push({ input: Buffer.from(svg), left: 0, top: 0 });
  const out = path.join(workDir, `${map.toLowerCase()}-scene`, "contact.png");
  await sharp({
    create: { width: cols * cell, height: rows * (cell + label), channels: 3, background: "#181818" },
  })
    .composite(composites)
    .png()
    .toFile(out);
  console.log(`wrote ${out} (${items.length} assets, ${items.filter((i) => i.iou < 0.5).length} with IoU < 0.5)`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
