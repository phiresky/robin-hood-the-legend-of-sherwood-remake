// Side-by-side comparison of image-to-3D backends on the same assets.
//
//   tsx src/compare-backends.ts --assets york-tower-house,york-b038 [--cell 260] [--out name.png]
//   -> work/<map>-scene/backends.png (or --out) and a markdown table on stdout
//
// One row per asset: the map crop, then per backend (asset.model = SAM 3D,
// asset.alt_models[*]) the placed model over the crop from the map camera
// (unlit) above an orbit view (shaded), labelled with fit IoU, colour
// agreement, triangle count, request time and price.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor, AssetModel, MapCamera } from "@rle/shared";
import { libraryDir, workDir } from "./env";
import { loadProtoLevel, mapImageSource } from "./asset-writer";
import { fitMapCamera } from "./map-camera";
import { bounds, loadGlb, transformPositions } from "./mesh";
import { mapView, orbitView, render } from "./render";
import type { Bbox } from "./clip";

const ORDER = ["sam3d", "trellis2", "tripo", "hunyuan"];

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const ids = (get("assets") ?? "").split(",").filter(Boolean);
  if (ids.length === 0) throw new Error("usage: --assets id,id [--cell px]");
  const cell = Number(get("cell") ?? 260);

  const levels = new Map<string, { cam: MapCamera; src: string | Buffer }>();
  const rows: { id: string; tiles: sharp.OverlayOptions[]; labels: string[]; table: string[] }[] = [];
  let map = "";
  for (const id of ids) {
    const desc: AssetDescriptor = JSON.parse(
      await fs.readFile(path.join(libraryDir, id, "asset.json"), "utf8"),
    );
    map = desc.source.map;
    let lv = levels.get(map);
    if (!lv) {
      const level = await loadProtoLevel(map);
      const fit = fitMapCamera(level);
      const src = await mapImageSource(map, "Day", true, level);
      if (!src) throw new Error(`no Day map for ${map}`);
      lv = { cam: { kind: fit.kind, elevation_deg: fit.elevation_deg }, src };
      levels.set(map, lv);
    }
    const models: [string, AssetModel][] = [];
    if (desc.model) models.push(["sam3d", desc.model]);
    for (const [k, m] of Object.entries(desc.alt_models ?? {})) models.push([k, m]);
    // known backends first (in ORDER), their cutout variants right after them
    const rank = (k: string) => {
      const base = k.split("-")[0]!;
      const i = ORDER.indexOf(base);
      return (i < 0 ? 99 : i) * 10 + (k.includes("-") ? 1 : 0);
    };
    models.sort((a, b) => rank(a[0]) - rank(b[0]) || a[0].localeCompare(b[0]));

    // common crop: the mask bbox with 25% context
    const [ax, ay, aw, ah] = desc.source.bbox;
    const pad = Math.round(0.25 * Math.max(aw, ah));
    const meta = await sharp(lv.src).metadata();
    const cx = Math.max(0, ax - pad);
    const cy = Math.max(0, ay - pad);
    const cw = Math.min(meta.width! - cx, aw + 2 * pad);
    const ch = Math.min(meta.height! - cy, ah + 2 * pad);
    const rect: Bbox = [cx, cy, cw, ch];
    const cropPng = await sharp(lv.src).extract({ left: cx, top: cy, width: cw, height: ch }).png().toBuffer();
    const fitCell = (buf: Buffer) =>
      sharp(buf).resize(cell, cell, { fit: "contain", background: "#202020" }).png().toBuffer();

    const tiles: sharp.OverlayOptions[] = [{ input: await fitCell(cropPng), left: 0, top: 0 }];
    const labels: string[] = [`${id}`];
    const table: string[] = [];
    let col = 1;
    for (const [backend, m] of models) {
      const mesh = await loadGlb(path.join(libraryDir, id, m.glb));
      const p = m.placement;
      const positions = transformPositions(mesh.positions, p.rotation, p.scale, p.position);
      const over = render([{ mesh, positions }], { ...mapView(lv.cam, rect), unlit: true });
      const overlay = await sharp(cropPng)
        .composite([{ input: over, raw: { width: cw, height: ch, channels: 4 }, blend: "over" }])
        .png()
        .toBuffer();
      const b = bounds(positions);
      const center: [number, number, number] = [
        (b.min[0] + b.max[0]) / 2,
        (b.min[1] + b.max[1]) / 2,
        (b.min[2] + b.max[2]) / 2,
      ];
      const extent = Math.max(b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]);
      const orbit = await sharp(
        render([{ mesh, positions }], orbitView(center, 35, 30, cell, cell, (cell * 0.8) / extent)),
        { raw: { width: cell, height: cell, channels: 4 } },
      )
        .flatten({ background: "#303030" })
        .png()
        .toBuffer();
      tiles.push({ input: await fitCell(overlay), left: col * cell, top: 0 });
      tiles.push({ input: orbit, left: col * cell, top: cell });
      const tris = mesh.indices.length / 3;
      const secs = m.extraction.seconds !== undefined ? `${m.extraction.seconds.toFixed(0)}s` : "";
      const price = m.extraction.price_usd !== undefined ? `$${m.extraction.price_usd.toFixed(2)}` : "";
      labels.push(
        `${backend}: IoU ${m.fit_iou.toFixed(2)} app ${m.fit_appearance?.toFixed(2) ?? "-"} ${Math.round(tris / 1000)}k tris ${secs} ${price}`,
      );
      table.push(
        `| ${id} | ${backend} | ${m.fit_iou.toFixed(3)} | ${m.fit_appearance?.toFixed(3) ?? "-"} | ${Math.round(tris / 1000)}k | ${mesh.textures.map((t) => `${t.width}`).join("+") || "vertex"} | ${secs} | ${price} |`,
      );
      col++;
    }
    rows.push({ id, tiles, labels, table });
  }

  const cols = Math.max(...rows.map((r) => r.labels.length));
  const rowH = 2 * cell + 20;
  const composites: sharp.OverlayOptions[] = [];
  let svg = `<svg width="${cols * cell}" height="${rows.length * rowH}" xmlns="http://www.w3.org/2000/svg">`;
  rows.forEach((r, i) => {
    for (const t of r.tiles) composites.push({ ...t, top: (t.top as number) + i * rowH });
    r.labels.forEach((l, j) => {
      svg += `<text x="${j * cell + 4}" y="${i * rowH + 2 * cell + 14}" font-size="11" font-family="sans-serif" fill="#ddd">${l.replace(/&/g, "&amp;")}</text>`;
    });
  });
  svg += "</svg>";
  composites.push({ input: Buffer.from(svg), left: 0, top: 0 });
  const out = path.join(workDir, `${map.toLowerCase()}-scene`, get("out") ?? "backends.png");
  await sharp({ create: { width: cols * cell, height: rows.length * rowH, channels: 3, background: "#181818" } })
    .composite(composites)
    .png()
    .toFile(out);

  console.log("| asset | backend | IoU | appearance | tris | texture px | time | price |");
  console.log("|---|---|---|---|---|---|---|---|");
  for (const r of rows) for (const line of r.table) console.log(line);
  console.log(`wrote ${out}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
