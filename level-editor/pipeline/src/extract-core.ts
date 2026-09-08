// Core asset-extraction flow, shared by the extract CLI and the sweep driver.
//
// Flow: crop the Day map around `bbox` (padded), send the crop to SAM 3 with
// the concept prompt (full-res masks since the crop is small), pick mask(s),
// tighten the bbox to each mask, cut Day/Fog/Night pixels with the mask as
// alpha, clip intersecting level metadata to asset-local coords, and write
// everything to library/<id>/. Review sheets land in work/<id>/.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { LibraryIndexEntry, ProtoLevel } from "@rle/shared";
import { libraryDir, workDir } from "./env";
import { segment, type SamMask } from "./sam";
import type { Bbox } from "./clip";
import { loadProtoLevel, mapImageSource, writeMaskedAsset } from "./asset-writer";

export interface ExtractOptions {
  map: string;
  bbox: Bbox;
  /** concept prompt; optional when boxes/points drive the segmentation */
  prompt?: string;
  /** box prompts in WORLD coordinates [x, y, w, h] */
  boxes?: Bbox[];
  /** point prompts in WORLD coordinates */
  points?: { x: number; y: number; label: 0 | 1 }[];
  /**
   * composite the map's non-integrated patch sprites (roof closers for
   * cutaway buildings) onto the map before cropping and cutting
   */
  applyPatches?: boolean;
  /** "crop" = no segmentation: rectangular cut of bbox with a full mask (terrain swatches) */
  mode?: "sam" | "crop";
  /** fill enclosed background pockets in the mask (window holes etc.) */
  fillHoles?: boolean;
  name: string;
  id: string;
  tags: string[];
  pad: number;
  maxMasks: number;
  /** "best" = highest score; "all" = one asset per surviving mask */
  pick: number | "best" | "all";
  scaleClass: "unique" | "variant" | "spline-segment";
  variantGroup?: string;
  /** drop masks below this score in --pick all mode */
  minScore: number;
  /** skip masks with fewer foreground pixels (pick=all only) */
  minArea: number;
  /** skip masks whose bbox IoU with an existing same-map asset exceeds this */
  dedupeIou: number;
}

export const EXTRACT_DEFAULTS = {
  pad: 48,
  maxMasks: 8,
  pick: "best" as const,
  scaleClass: "unique" as const,
  minScore: 0.5,
  minArea: 2500,
  dedupeIou: 0.45,
};

export interface ExtractSummary {
  written: { id: string; bbox: Bbox; score: number | null }[];
  skipped: { index: number; reason: string }[];
  reviewDir: string;
}

export function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function maskBounds(mask: { data: Uint8Array; width: number; height: number }) {
  let minX = Infinity,
    minY = Infinity,
    maxX = -1,
    maxY = -1;
  for (let y = 0; y < mask.height; y++) {
    for (let x = 0; x < mask.width; x++) {
      if (mask.data[y * mask.width + x]) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  if (maxX < 0) return null;
  return { minX, minY, maxX, maxY };
}

/** fill background pockets fully enclosed by mask foreground (flood from border) */
function fillMaskHoles(mask: { data: Uint8Array; width: number; height: number }) {
  const { data, width: w, height: h } = mask;
  const outside = new Uint8Array(w * h);
  const stack: number[] = [];
  const push = (i: number) => {
    if (!outside[i] && !data[i]) {
      outside[i] = 1;
      stack.push(i);
    }
  };
  for (let x = 0; x < w; x++) {
    push(x);
    push((h - 1) * w + x);
  }
  for (let y = 0; y < h; y++) {
    push(y * w);
    push(y * w + w - 1);
  }
  while (stack.length) {
    const i = stack.pop()!;
    const x = i % w;
    if (x > 0) push(i - 1);
    if (x < w - 1) push(i + 1);
    if (i >= w) push(i - w);
    if (i < w * (h - 1)) push(i + w);
  }
  for (let i = 0; i < data.length; i++) {
    if (!data[i] && !outside[i]) data[i] = 255;
  }
}

function bboxIou(a: Bbox, b: Bbox): number {
  const ix = Math.max(0, Math.min(a[0] + a[2], b[0] + b[2]) - Math.max(a[0], b[0]));
  const iy = Math.max(0, Math.min(a[1] + a[3], b[1] + b[3]) - Math.max(a[1], b[1]));
  const inter = ix * iy;
  return inter / (a[2] * a[3] + b[2] * b[3] - inter);
}

async function loadLibraryIndex(): Promise<LibraryIndexEntry[]> {
  try {
    return JSON.parse(await fs.readFile(path.join(libraryDir, "index.json"), "utf8"));
  } catch {
    return [];
  }
}

export async function runExtraction(opts: ExtractOptions): Promise<ExtractSummary> {
  const level: ProtoLevel = await loadProtoLevel(opts.map);
  const daySrc = await mapImageSource(opts.map, "Day", opts.applyPatches ?? false, level);
  if (!daySrc) throw new Error(`no Day/${opts.map}.map.png in the datadir`);

  const meta = await sharp(daySrc).metadata();
  const mapW = meta.width!;
  const mapH = meta.height!;

  // padded crop around the requested bbox, clamped to the map
  const [bx, by, bw, bh] = opts.bbox;
  const cx = Math.max(0, Math.floor(bx - opts.pad));
  const cy = Math.max(0, Math.floor(by - opts.pad));
  const cw = Math.min(mapW - cx, Math.ceil(bw + 2 * opts.pad));
  const ch = Math.min(mapH - cy, Math.ceil(bh + 2 * opts.pad));

  const cropPng = await sharp(daySrc)
    .extract({ left: cx, top: cy, width: cw, height: ch })
    .png()
    .toBuffer();

  let masks: SamMask[];
  if (opts.mode === "crop") {
    // rectangular swatch: full-opaque mask over exactly the requested bbox
    const data = new Uint8Array(cw * ch);
    const rx = Math.floor(bx) - cx;
    const ry = Math.floor(by) - cy;
    for (let y = ry; y < Math.min(ch, ry + Math.ceil(bh)); y++) {
      data.fill(255, y * cw + rx, y * cw + rx + Math.min(cw - rx, Math.ceil(bw)));
    }
    masks = [{ data, width: cw, height: ch, score: null, box: null }];
  } else {
    console.log(
      `SAM 3: ${opts.prompt ? `"${opts.prompt}"` : "(geometric prompts)"} on ${cw}x${ch} crop of ${opts.map} @ ${cx},${cy}`,
    );
    masks = await segment({
      imagePng: cropPng,
      width: cw,
      height: ch,
      prompt: opts.prompt,
      boxes: opts.boxes?.map(([x, y, w, h]) => [x - cx, y - cy, w, h]),
      points: opts.points?.map((p) => ({ x: p.x - cx, y: p.y - cy, label: p.label })),
      maxMasks: opts.maxMasks,
    });
    console.log(
      `got ${masks.length} mask(s), scores: ${masks.map((m) => m.score?.toFixed(3) ?? "?").join(", ")}`,
    );
  }
  if (masks.length === 0) {
    return {
      written: [],
      skipped: [{ index: -1, reason: "no masks for prompt" }],
      reviewDir: path.join(workDir, opts.id),
    };
  }

  // fx/patch sprites legitimately overlap building cutouts — never dedupe
  // against them
  const existing = (await loadLibraryIndex()).filter(
    (e) => e.source_map === opts.map && !e.tags.includes("fx") && !e.tags.includes("patch"),
  );

  const summary: ExtractSummary = {
    written: [],
    skipped: [],
    reviewDir: path.join(workDir, opts.id),
  };

  async function writeAssetForMask(mask: SamMask, assetId: string, assetName: string) {
    // note: sharp's resize() promotes 1-channel raw input to 3 channels;
    // extractChannel(0) forces it back to a single channel
    const maskResized = await sharp(Buffer.from(mask.data), {
      raw: { width: mask.width, height: mask.height, channels: 1 },
    })
      .resize(cw, ch, { kernel: "nearest" })
      .extractChannel(0)
      .raw()
      .toBuffer();
    const cropMask = { data: new Uint8Array(maskResized), width: cw, height: ch };
    if (opts.fillHoles) fillMaskHoles(cropMask);

    const b = maskBounds(cropMask);
    if (!b) return { skip: "empty mask" };
    const ax = cx + b.minX;
    const ay = cy + b.minY;
    const aw = b.maxX - b.minX + 1;
    const ah = b.maxY - b.minY + 1;
    const assetBbox: Bbox = [ax, ay, aw, ah];

    if (opts.pick === "all") {
      let fg = 0;
      for (const v of cropMask.data) if (v) fg++;
      if (fg < opts.minArea) return { skip: `fragment (${fg}px < ${opts.minArea})` };
      const touchesEdge =
        b.minX === 0 || b.minY === 0 || b.maxX === cw - 1 || b.maxY === ch - 1;
      if (touchesEdge && opts.scaleClass !== "spline-segment") {
        return { skip: "truncated at crop edge" };
      }
    }
    for (const e of existing) {
      const iou = bboxIou(assetBbox, e.bbox);
      if (iou > opts.dedupeIou) return { skip: `duplicate of ${e.id} (IoU ${iou.toFixed(2)})` };
    }

    console.log(`${assetId}: bbox ${ax},${ay} ${aw}x${ah}`);

    const assetMask = new Uint8Array(aw * ah);
    for (let y = 0; y < ah; y++) {
      assetMask.set(cropMask.data.subarray((b.minY + y) * cw + b.minX, (b.minY + y) * cw + b.minX + aw), y * aw);
    }
    await writeMaskedAsset({
      map: opts.map,
      level,
      applyPatches: opts.applyPatches ?? false,
      mask: assetMask,
      bbox: assetBbox,
      id: assetId,
      name: assetName,
      tags: opts.tags,
      scaleClass: opts.scaleClass,
      variantGroup: opts.variantGroup,
      extraction: {
        tool: opts.mode === "crop" ? "rect-crop" : "fal-ai/sam-3/image-rle",
        prompt: opts.prompt,
        boxes: opts.boxes,
        points: opts.points?.map((p) => [p.x, p.y] as [number, number]),
        score: mask.score ?? undefined,
      },
    });
    // new assets also dedupe against later masks in this same run
    existing.push({
      id: assetId,
      name: assetName,
      tags: opts.tags,
      scale_class: opts.scaleClass,
      source_map: opts.map,
      bbox: assetBbox,
    });
    summary.written.push({ id: assetId, bbox: assetBbox, score: mask.score });
    return { skip: null };
  }

  let chosen: SamMask | null = null;
  if (opts.pick === "all") {
    let n = 0;
    for (let i = 0; i < masks.length; i++) {
      const m = masks[i]!;
      if ((m.score ?? 0) < opts.minScore) {
        summary.skipped.push({ index: i, reason: `score ${m.score?.toFixed(3)}` });
        continue;
      }
      const res = await writeAssetForMask(m, `${opts.id}-${n + 1}`, `${opts.name} ${n + 1}`);
      if (res.skip) {
        console.log(`mask ${i}: skipped — ${res.skip}`);
        summary.skipped.push({ index: i, reason: res.skip });
      } else {
        n++;
      }
    }
  } else {
    chosen =
      opts.pick === "best"
        ? masks.reduce((a, b) => ((b.score ?? 0) > (a.score ?? 0) ? b : a))
        : (masks[opts.pick] ??
          (() => {
            throw new Error(`--pick ${opts.pick} out of range (${masks.length} masks)`);
          })());
    const res = await writeAssetForMask(chosen, opts.id, opts.name);
    if (res.skip) {
      console.log(`skipped — ${res.skip}`);
      summary.skipped.push({ index: masks.indexOf(chosen), reason: res.skip });
    }
  }

  // review sheets: crop + red overlay per candidate mask
  await fs.mkdir(summary.reviewDir, { recursive: true });
  await fs.writeFile(path.join(summary.reviewDir, "crop.png"), cropPng);
  for (let i = 0; i < masks.length; i++) {
    const m = masks[i]!;
    const resized = await sharp(Buffer.from(m.data), {
      raw: { width: m.width, height: m.height, channels: 1 },
    })
      .resize(cw, ch, { kernel: "nearest" })
      .extractChannel(0)
      .raw()
      .toBuffer();
    const overlay = Buffer.alloc(cw * ch * 4);
    for (let p = 0; p < cw * ch; p++) {
      if (resized[p]) {
        overlay[p * 4] = 255;
        overlay[p * 4 + 3] = 110;
      }
    }
    const sheet = await sharp(cropPng)
      .composite([{ input: overlay, raw: { width: cw, height: ch, channels: 4 }, blend: "over" }])
      .png()
      .toBuffer();
    const tag = chosen !== null && i === masks.indexOf(chosen) ? "-CHOSEN" : "";
    await fs.writeFile(path.join(summary.reviewDir, `mask-${i}${tag}.png`), sheet);
  }
  console.log(`review sheets in ${summary.reviewDir}`);
  return summary;
}
