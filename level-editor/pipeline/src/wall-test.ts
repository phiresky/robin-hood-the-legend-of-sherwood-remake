// Render isolated wall runs at various directions to a test sheet.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor } from "@rle/shared";
import { expandWallRunDirectional, type WallSegmentSpec } from "@rle/shared";
import { libraryDir, workDir } from "./env";

const SEGS = [
  "leicester-castle-wall-south-slice-shp0",
  "leicester-castle-wall-south-slice-shp35",
  "leicester-castle-wall-south-slice-shm35",
  "leicester-castle-wall-south-slice-shp50",
  "leicester-castle-wall-south-slice-shm50",
];

async function main() {
  const specs: WallSegmentSpec[] = [];
  const imgs = new Map<string, { path: string; w: number; h: number }>();
  for (const id of SEGS) {
    const a: AssetDescriptor = JSON.parse(
      await fs.readFile(path.join(libraryDir, id, "asset.json"), "utf8"),
    );
    const p = path.join(libraryDir, id, "day.png");
    const m = await sharp(p).metadata();
    specs.push({
      id,
      size: [a.source.bbox[2], a.source.bbox[3]],
      anchor: a.anchor,
      directionDeg: a.wall_direction_deg ?? 0,
    });
    imgs.set(id, { path: p, w: m.width!, h: m.height! });
  }

  const W = 2000, H = 1500;
  const runs: { label: string; points: [number, number][] }[] = [
    { label: "horizontal", points: [[100, 250], [900, 250]] },
    { label: "down-right 35", points: [[1100, 150], [1800, 640]] },
    { label: "up-right -35", points: [[100, 900], [800, 410]] },
    { label: "up-right -51", points: [[1100, 1450], [1500, 950]] },
    { label: "zigzag", points: [[100, 1400], [500, 1150], [900, 1400], [1300, 1420], [1600, 1200]] },
  ];
  const comps: sharp.OverlayOptions[] = [];
  let svg = `<svg width="${W}" height="${H}">`;
  for (const r of runs) {
    const stamps = expandWallRunDirectional(r.points, specs);
    for (const s of stamps) {
      const img = imgs.get(s.asset)!;
      if (s.pos[0] < 0 || s.pos[1] < 0 || s.pos[0] + img.w > W || s.pos[1] + img.h > H) continue;
      comps.push({ input: img.path, left: s.pos[0], top: s.pos[1] });
    }
    svg += `<polyline points="${r.points.map((p) => p.join(",")).join(" ")}" stroke="cyan" stroke-width="2" fill="none" opacity="0.7"/>`;
    svg += `<text x="${r.points[0]![0]}" y="${r.points[0]![1] - 12}" fill="cyan" font-size="22">${r.label}</text>`;
  }
  svg += "</svg>";
  comps.push({ input: Buffer.from(svg), left: 0, top: 0 });
  const out = await sharp({
    create: { width: W, height: H, channels: 3, background: { r: 60, g: 70, b: 50 } },
  })
    .composite(comps)
    .png()
    .toBuffer();
  await fs.writeFile(path.join(workDir, "wall-test.png"), out);
  console.log("wrote work/wall-test.png");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
