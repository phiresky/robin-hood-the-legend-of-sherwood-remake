// Detect building instances across a whole map with SAM 3 concept prompts.
//
//   node src/detect.ts --map york [--prompts "house,tower,church,building"]
//       [--tile 1024] [--overlap 512] [--min-score 0.4] [--min-area 2500]
//       [--max-masks 32] [--no-patches] [--out work/york-scene]
//
// The map is swept in overlapping tiles; every prompt runs on every tile
// (responses are cached by sam.ts). Masks are merged across tiles/prompts:
// complete instances win over ones truncated at a tile edge, higher scores
// win among duplicates (mask IoU > 0.5), and masks mostly contained in an
// already kept one are dropped as fragments. Output:
//   <out>/detections.json   — list consumed by reconstruct.ts --detections
//   <out>/masks/<id>.png    — 8-bit masks, bbox-sized
//   <out>/overlay.png       — half-size map with numbered mask outlines for review
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import { workDir } from "./env.ts";
import { loadProtoLevel, mapImageSource } from "./asset-writer.ts";
import { segment } from "./sam.ts";
import type { Bbox } from "./clip.ts";
import type { Detection, DetectionsFile } from "./reconstruct.ts";

interface Candidate {
  prompt: string;
  score: number;
  bbox: Bbox;
  /** bbox-sized 0/255 mask */
  mask: Uint8Array;
  area: number;
  /** touched a tile edge that is not a map edge */
  truncated: boolean;
}

function maskBounds(data: Uint8Array, w: number, h: number) {
  let minX = w,
    minY = h,
    maxX = -1,
    maxY = -1,
    area = 0;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      if (!data[y * w + x]) continue;
      area++;
      if (x < minX) minX = x;
      if (x > maxX) maxX = x;
      if (y < minY) minY = y;
      if (y > maxY) maxY = y;
    }
  }
  return maxX < 0 ? null : { minX, minY, maxX, maxY, area };
}

/** intersection pixel count of two bbox-aligned masks in map coords */
function intersection(a: Candidate, b: Candidate): number {
  const x0 = Math.max(a.bbox[0], b.bbox[0]);
  const y0 = Math.max(a.bbox[1], b.bbox[1]);
  const x1 = Math.min(a.bbox[0] + a.bbox[2], b.bbox[0] + b.bbox[2]);
  const y1 = Math.min(a.bbox[1] + a.bbox[3], b.bbox[1] + b.bbox[3]);
  if (x1 <= x0 || y1 <= y0) return 0;
  let n = 0;
  for (let y = y0; y < y1; y++) {
    const ra = (y - a.bbox[1]) * a.bbox[2] - a.bbox[0];
    const rb = (y - b.bbox[1]) * b.bbox[2] - b.bbox[0];
    for (let x = x0; x < x1; x++) {
      if (a.mask[ra + x] && b.mask[rb + x]) n++;
    }
  }
  return n;
}

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const map = get("map");
  if (!map) throw new Error("usage: --map <name> [--prompts a,b] [--tile N] [--overlap N] ...");
  const prompts = (get("prompts") ?? "house,tower,church,building").split(",");
  const tile = Number(get("tile") ?? 1024);
  const overlap = Number(get("overlap") ?? 512);
  const minScore = Number(get("min-score") ?? 0.4);
  const minArea = Number(get("min-area") ?? 2500);
  const maxMasks = Number(get("max-masks") ?? 32); // endpoint limit: 32
  const applyPatches = !argv.includes("--no-patches");
  const outDir = get("out") ?? path.join(workDir, `${map.toLowerCase()}-scene`);

  const level = await loadProtoLevel(map);
  const src = await mapImageSource(map, "Day", applyPatches, level);
  if (!src) throw new Error(`no Day map for ${map}`);
  const mapPng = await sharp(src).png().toBuffer();
  const meta = await sharp(mapPng).metadata();
  const mapW = meta.width!;
  const mapH = meta.height!;

  const stride = tile - overlap;
  const xs: number[] = [];
  for (let x = 0; ; x += stride) {
    xs.push(Math.min(x, Math.max(0, mapW - tile)));
    if (x + tile >= mapW) break;
  }
  const ys: number[] = [];
  for (let y = 0; ; y += stride) {
    ys.push(Math.min(y, Math.max(0, mapH - tile)));
    if (y + tile >= mapH) break;
  }
  console.log(
    `${map}: ${mapW}x${mapH}, ${xs.length}x${ys.length} tiles of ${tile} (stride ${stride}), prompts: ${prompts.join(", ")}`,
  );

  const candidates: Candidate[] = [];
  for (const ty of ys) {
    for (const tx of xs) {
      const tw = Math.min(tile, mapW - tx);
      const th = Math.min(tile, mapH - ty);
      const tilePng = await sharp(mapPng)
        .extract({ left: tx, top: ty, width: tw, height: th })
        .png()
        .toBuffer();
      for (const prompt of prompts) {
        const masks = await segment({
          imagePng: tilePng,
          width: tw,
          height: th,
          prompt,
          maxMasks,
        });
        let kept = 0;
        for (const m of masks) {
          if ((m.score ?? 0) < minScore) continue;
          const b = maskBounds(m.data, m.width, m.height);
          if (!b || b.area < minArea) continue;
          const truncated =
            (b.minX === 0 && tx > 0) ||
            (b.minY === 0 && ty > 0) ||
            (b.maxX === tw - 1 && tx + tw < mapW) ||
            (b.maxY === th - 1 && ty + th < mapH);
          const w = b.maxX - b.minX + 1;
          const h = b.maxY - b.minY + 1;
          const mask = new Uint8Array(w * h);
          for (let y = 0; y < h; y++) {
            for (let x = 0; x < w; x++) {
              mask[y * w + x] = m.data[(y + b.minY) * m.width + x + b.minX]!;
            }
          }
          candidates.push({
            prompt,
            score: m.score ?? 0,
            bbox: [tx + b.minX, ty + b.minY, w, h],
            mask,
            area: b.area,
            truncated,
          });
          kept++;
        }
        console.log(`tile ${tx},${ty} "${prompt}": ${masks.length} masks, ${kept} kept`);
      }
    }
  }

  // merge: complete before truncated, then by score
  candidates.sort((a, b) => Number(a.truncated) - Number(b.truncated) || b.score - a.score);
  const kept: Candidate[] = [];
  const dropped: Record<string, number> = {};
  for (const c of candidates) {
    let reason: string | null = null;
    for (const k of kept) {
      const inter = intersection(c, k);
      if (inter === 0) continue;
      const iou = inter / (c.area + k.area - inter);
      if (iou > 0.5) {
        reason = "duplicate";
        break;
      }
      if (inter / c.area > 0.8) {
        reason = "fragment";
        break;
      }
      if (c.truncated && inter / c.area > 0.3) {
        reason = "truncated overlap";
        break;
      }
    }
    if (reason) dropped[reason] = (dropped[reason] ?? 0) + 1;
    else kept.push(c);
  }
  // reading order by ground contact (bottom of bbox), then x
  kept.sort((a, b) => a.bbox[1] + a.bbox[3] - (b.bbox[1] + b.bbox[3]) || a.bbox[0] - b.bbox[0]);
  console.log(
    `${candidates.length} candidates -> ${kept.length} kept (${Object.entries(dropped)
      .map(([k, v]) => `${v} ${k}`)
      .join(", ")}); ${kept.filter((k) => k.truncated).length} truncated at tile edges`,
  );

  await fs.mkdir(path.join(outDir, "masks"), { recursive: true });
  const detections: Detection[] = [];
  const pad = String(kept.length).length;
  for (let i = 0; i < kept.length; i++) {
    const c = kept[i]!;
    const id = `${map.toLowerCase()}-b${String(i + 1).padStart(pad, "0")}`;
    const maskFile = `masks/${id}.png`;
    await sharp(Buffer.from(c.mask), { raw: { width: c.bbox[2], height: c.bbox[3], channels: 1 } })
      .png()
      .toFile(path.join(outDir, maskFile));
    detections.push({
      id,
      name: `${map} ${c.prompt} ${i + 1}`,
      tags: ["building", c.prompt, ...(c.truncated ? ["truncated"] : [])],
      prompt: c.prompt,
      score: c.score,
      bbox: c.bbox,
      mask: maskFile,
    });
  }
  const file: DetectionsFile = { map, apply_patches: applyPatches, detections };
  await fs.writeFile(path.join(outDir, "detections.json"), JSON.stringify(file, null, 1));

  // review overlay at half size: tinted masks + numbered bboxes
  const scale = 0.5;
  const ow = Math.round(mapW * scale);
  const oh = Math.round(mapH * scale);
  const tint = Buffer.alloc(ow * oh * 4);
  const palette = [
    [255, 80, 80],
    [80, 255, 80],
    [80, 160, 255],
    [255, 220, 60],
    [255, 100, 255],
    [60, 255, 255],
  ];
  let svg = `<svg width="${ow}" height="${oh}" xmlns="http://www.w3.org/2000/svg">`;
  for (let i = 0; i < kept.length; i++) {
    const c = kept[i]!;
    const [r, g, b] = palette[i % palette.length]!;
    const [bx, by, bw, bh] = c.bbox;
    for (let y = 0; y < bh; y++) {
      for (let x = 0; x < bw; x++) {
        if (!c.mask[y * bw + x]) continue;
        const ox = Math.floor((bx + x) * scale);
        const oy = Math.floor((by + y) * scale);
        const o = (oy * ow + ox) * 4;
        tint[o] = r!;
        tint[o + 1] = g!;
        tint[o + 2] = b!;
        tint[o + 3] = 90;
      }
    }
    svg += `<rect x="${bx * scale}" y="${by * scale}" width="${bw * scale}" height="${bh * scale}" fill="none" stroke="rgb(${r},${g},${b})" stroke-width="1"/>`;
    svg += `<text x="${bx * scale + 2}" y="${(by + bh) * scale - 3}" font-size="13" font-family="sans-serif" fill="white" stroke="black" stroke-width="2" paint-order="stroke">${i + 1}${c.truncated ? "T" : ""}</text>`;
  }
  svg += "</svg>";
  await sharp(mapPng)
    .resize(ow, oh)
    .composite([
      { input: tint, raw: { width: ow, height: oh, channels: 4 }, blend: "over" },
      { input: Buffer.from(svg), blend: "over" },
    ])
    .png()
    .toFile(path.join(outDir, "overlay.png"));
  console.log(`wrote ${outDir}/detections.json (${detections.length}) and overlay.png`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
