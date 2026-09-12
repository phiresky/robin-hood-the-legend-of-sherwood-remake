// Merge detections of buildings that are built against each other into one
// "block" detection, so SAM 3D reconstructs the block as a single model
// instead of cutting shared walls and roofs apart.
//
//   node src/merge-detections.ts --detections work/york-scene/detections.json
//       [--auto] [--gap 4] [--min-contact 30] [--max-members 4] [--max-side 900]
//       [--groups b012+b013,b040+b041+b042]
//
// --auto groups masks whose dilated outlines touch (contact length in
// pixels ≥ --min-contact), strongest contacts first, never exceeding
// --max-members per block or --max-side px for the block's bbox. --groups
// adds explicit blocks by detection id suffix. Writes
// <dir>/detections-merged.json (only the blocks, ids <map>-k<N>, with
// `members`), masks under <dir>/masks/, and <dir>/overlay-merged.png.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { Bbox } from "./clip.ts";
import { findMapPng } from "./asset-writer.ts";
import type { Detection, DetectionsFile } from "./reconstruct.ts";
import { parseDetections } from "./reconstruct.ts";
import { readDocument } from "./inputs.ts";

interface Loaded extends Detection {
  data: Uint8Array;
}

function dilate(d: Loaded, gap: number): { data: Uint8Array; bbox: Bbox } {
  const [bx, by, bw, bh] = d.bbox;
  const w = bw + 2 * gap;
  const h = bh + 2 * gap;
  const out = new Uint8Array(w * h);
  for (let y = 0; y < bh; y++) {
    for (let x = 0; x < bw; x++) {
      if (!d.data[y * bw + x]) continue;
      for (let dy = -gap; dy <= gap; dy++) {
        const row = (y + gap + dy) * w;
        out.fill(1, row + x, row + x + 2 * gap + 1);
      }
    }
  }
  return { data: out, bbox: [bx - gap, by - gap, w, h] };
}

/** pixels of b inside the dilated a */
function contact(a: { data: Uint8Array; bbox: Bbox }, b: Loaded): number {
  const x0 = Math.max(a.bbox[0], b.bbox[0]);
  const y0 = Math.max(a.bbox[1], b.bbox[1]);
  const x1 = Math.min(a.bbox[0] + a.bbox[2], b.bbox[0] + b.bbox[2]);
  const y1 = Math.min(a.bbox[1] + a.bbox[3], b.bbox[1] + b.bbox[3]);
  if (x1 <= x0 || y1 <= y0) return 0;
  let n = 0;
  for (let y = y0; y < y1; y++) {
    const ra = (y - a.bbox[1]) * a.bbox[2] - a.bbox[0];
    const rb = (y - b.bbox[1]) * b.bbox[2] - b.bbox[0];
    for (let x = x0; x < x1; x++) if (a.data[ra + x] && b.data[rb + x]) n++;
  }
  return n;
}

function unionBbox(ds: Loaded[]): Bbox {
  const x0 = Math.min(...ds.map((d) => d.bbox[0]));
  const y0 = Math.min(...ds.map((d) => d.bbox[1]));
  const x1 = Math.max(...ds.map((d) => d.bbox[0] + d.bbox[2]));
  const y1 = Math.max(...ds.map((d) => d.bbox[1] + d.bbox[3]));
  return [x0, y0, x1 - x0, y1 - y0];
}

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const file = get("detections");
  if (!file) throw new Error("usage: --detections <file> [--auto] [--groups a+b,c+d]");
  const dir = path.dirname(file);
  const gap = Number(get("gap") ?? 4);
  const minContact = Number(get("min-contact") ?? 30);
  const maxMembers = Number(get("max-members") ?? 4);
  const maxSide = Number(get("max-side") ?? 900);

  const det = parseDetections(await readDocument(file, true));
  const items: Loaded[] = [];
  for (const d of det.detections) {
    const { data } = await sharp(path.join(dir, d.mask)).extractChannel(0).raw().toBuffer({ resolveWithObject: true });
    items.push({ ...d, data: new Uint8Array(data) });
  }
  const byId = new Map(items.map((d) => [d.id, d]));
  const prefix = `${det.map.toLowerCase()}-`;

  // union-find over detections
  const parent = new Map<string, string>(items.map((d) => [d.id, d.id]));
  const find = (id: string): string => {
    const p = parent.get(id)!;
    if (p === id) return id;
    const r = find(p);
    parent.set(id, r);
    return r;
  };
  const members = (root: string) => items.filter((d) => find(d.id) === root);
  const tryUnion = (a: string, b: string, why: string): boolean => {
    const ra = find(a);
    const rb = find(b);
    if (ra === rb) return true;
    const all = [...members(ra), ...members(rb)];
    const [, , w, h] = unionBbox(all);
    if (all.length > maxMembers || Math.max(w, h) > maxSide) {
      console.log(`skip ${a}+${b} (${why}): ${all.length} members, ${w}x${h}`);
      return false;
    }
    parent.set(ra, rb);
    return true;
  };

  for (const g of (get("groups") ?? "").split(",").filter(Boolean)) {
    const ids = g.split("+").map((s) => (s.startsWith(prefix) ? s : prefix + s));
    for (const id of ids) if (!byId.has(id)) throw new Error(`unknown detection ${id}`);
    for (let i = 1; i < ids.length; i++) tryUnion(ids[0]!, ids[i]!, "explicit");
  }

  if (argv.includes("--auto")) {
    const pairs: { a: string; b: string; n: number }[] = [];
    const dilated = new Map(items.map((d) => [d.id, dilate(d, gap)]));
    for (let i = 0; i < items.length; i++) {
      for (let j = i + 1; j < items.length; j++) {
        const a = items[i]!;
        const b = items[j]!;
        const n = contact(dilated.get(a.id)!, b);
        if (n >= minContact) pairs.push({ a: a.id, b: b.id, n });
      }
    }
    pairs.sort((x, y) => y.n - x.n);
    for (const p of pairs) tryUnion(p.a, p.b, `contact ${p.n}px`);
  }

  const roots = [...new Set(items.map((d) => find(d.id)))];
  const blocks = roots.map((r) => members(r)).filter((m) => m.length > 1);
  blocks.sort((a, b) => unionBbox(a)[1] + unionBbox(a)[3] - (unionBbox(b)[1] + unionBbox(b)[3]));
  console.log(`${blocks.length} blocks from ${blocks.reduce((n, b) => n + b.length, 0)} detections`);

  const merged: Detection[] = [];
  const pad = String(blocks.length).length;
  for (const [i, group] of blocks.entries()) {
    const bbox = unionBbox(group);
    const [bx, by, bw, bh] = bbox;
    const mask = new Uint8Array(bw * bh);
    for (const d of group) {
      for (let y = 0; y < d.bbox[3]; y++) {
        for (let x = 0; x < d.bbox[2]; x++) {
          if (d.data[y * d.bbox[2] + x]) mask[(y + d.bbox[1] - by) * bw + (x + d.bbox[0] - bx)] = 255;
        }
      }
    }
    const id = `${prefix}k${String(i + 1).padStart(pad, "0")}`;
    const maskFile = `masks/${id}.png`;
    await sharp(Buffer.from(mask), { raw: { width: bw, height: bh, channels: 1 } })
      .png()
      .toFile(path.join(dir, maskFile));
    const ids = group.map((d) => d.id);
    merged.push({
      id,
      name: `${det.map} block ${i + 1} (${ids.map((s) => s.slice(prefix.length)).join("+")})`,
      tags: ["building", "block"],
      prompt: [...new Set(group.map((d) => d.prompt))].join("+"),
      score: Math.min(...group.map((d) => d.score ?? 0)),
      bbox,
      mask: maskFile,
      members: ids,
    });
    console.log(`${id}: ${ids.join(" + ")} -> ${bw}x${bh}`);
  }
  const out: DetectionsFile = { map: det.map, apply_patches: det.apply_patches, detections: merged };
  const outFile = path.join(dir, "detections-merged.json");
  await fs.writeFile(outFile, JSON.stringify(out, null, 1));

  // overlay: block bboxes + member ids on the half-size map
  const dayPath = await findMapPng("Day", det.map);
  if (dayPath) {
    const meta = await sharp(dayPath).metadata();
    const scale = 0.5;
    const ow = Math.round(meta.width! * scale);
    const oh = Math.round(meta.height! * scale);
    let svg = `<svg width="${ow}" height="${oh}" xmlns="http://www.w3.org/2000/svg">`;
    for (const m of merged) {
      const [bx, by, bw, bh] = m.bbox;
      svg += `<rect x="${bx * scale}" y="${by * scale}" width="${bw * scale}" height="${bh * scale}" fill="rgba(80,200,255,0.15)" stroke="#50c8ff" stroke-width="2"/>`;
      svg += `<text x="${bx * scale + 3}" y="${by * scale + 14}" font-size="13" font-family="sans-serif" fill="white" stroke="black" stroke-width="2" paint-order="stroke">${m.id.slice(prefix.length)}: ${m.members!.map((s) => s.slice(prefix.length + 1)).join("+")}</text>`;
    }
    svg += "</svg>";
    await sharp(dayPath)
      .resize(ow, oh)
      .composite([{ input: Buffer.from(svg), blend: "over" }])
      .png()
      .toFile(path.join(dir, "overlay-merged.png"));
  }
  console.log(`wrote ${outFile} (${merged.length} blocks) and overlay-merged.png`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
