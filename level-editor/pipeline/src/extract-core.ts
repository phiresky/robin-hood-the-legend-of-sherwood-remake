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
import type { AssetDescriptor, LibraryIndexEntry, ProtoLevel } from "@rle/shared";
import { datadirPath, libraryDir, workDir } from "./env";
import { segment, type SamMask } from "./sam";
import { clipLevel, type Bbox } from "./clip";
import { writeAsset } from "./library";
import { fxTopLeft, loadFxSprite, loadKeyedFxPng } from "./fx";

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

async function findMapPng(levelsDir: string, ambiance: string, map: string) {
  const dir = path.join(levelsDir, ambiance);
  try {
    const files = await fs.readdir(dir);
    const hit = files.find((f) => f.toLowerCase() === `${map.toLowerCase()}.map.png`);
    return hit ? path.join(dir, hit) : null;
  } catch {
    return null;
  }
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

// full-map images with the non-integrated patch sprites (roof closers)
// composited on, cached per map+ambiance for the run
const patchedMapCache = new Map<string, Buffer>();

async function patchedMapImage(
  levelsDir: string,
  map: string,
  ambiance: string,
  mapPath: string,
  level: ProtoLevel,
): Promise<Buffer> {
  const key = `${map}/${ambiance}`;
  const cached = patchedMapCache.get(key);
  if (cached) return cached;
  const composites: { input: Buffer; left: number; top: number }[] = [];
  for (const p of level.patches) {
    const s = p.element_fx.sprite;
    if (s.frame_profile_name === "pixel_vert") continue;
    if (p.integrate_in_background) continue; // drawbridges etc., not roof closers
    // night/fog banks may not exist; fall back to the Day sprite
    const fx =
      (await loadFxSprite(ambiance, s.frame_profile_name, s.profile_name)) ??
      (await loadFxSprite("Day", s.frame_profile_name, s.profile_name));
    if (!fx) continue;
    const [left, top] = fxTopLeft(fx, s.position_x, s.position_y, s.elevation);
    composites.push({ input: await loadKeyedFxPng(fx.framePath), left, top });
  }
  const img = await sharp(mapPath).composite(composites).png().toBuffer();
  patchedMapCache.set(key, img);
  return img;
}

export async function runExtraction(opts: ExtractOptions): Promise<ExtractSummary> {
  const levelsDir = path.join(datadirPath(), "Data", "Levels");

  const dayPath = await findMapPng(levelsDir, "Day", opts.map);
  if (!dayPath) throw new Error(`no Day/${opts.map}.map.png under ${levelsDir}`);

  const meta = await sharp(dayPath).metadata();
  const mapW = meta.width!;
  const mapH = meta.height!;

  const level: ProtoLevel = JSON.parse(
    await fs.readFile(path.join(levelsDir, `${opts.map}.rhp.json`), "utf8"),
  );
  const daySrc: string | Buffer = opts.applyPatches
    ? await patchedMapImage(levelsDir, opts.map, "Day", dayPath, level)
    : dayPath;

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

  const existing = (await loadLibraryIndex()).filter((e) => e.source_map === opts.map);

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

    const maskCropPng = await sharp(Buffer.from(cropMask.data), {
      raw: { width: cw, height: ch, channels: 1 },
    })
      .extract({ left: b.minX, top: b.minY, width: aw, height: ah })
      .png()
      .toBuffer();
    const maskAlpha = await sharp(maskCropPng).extractChannel(0).raw().toBuffer();

    // cut each available ambiance with the same mask
    const images: Record<string, Buffer> = { "mask.png": maskCropPng };
    const cutouts: Partial<Record<"day" | "fog" | "night", string>> = {};
    for (const amb of ["Day", "Fog", "Night"] as const) {
      const mapPath = amb === "Day" ? dayPath : await findMapPng(levelsDir, amb, opts.map);
      if (!mapPath) continue;
      const src: string | Buffer = opts.applyPatches
        ? await patchedMapImage(levelsDir, opts.map, amb, mapPath, level)
        : mapPath;
      const rgb = await sharp(src)
        .extract({ left: ax, top: ay, width: aw, height: ah })
        .removeAlpha()
        .raw()
        .toBuffer();
      const rgba = Buffer.alloc(aw * ah * 4);
      for (let i = 0; i < aw * ah; i++) {
        rgba[i * 4] = rgb[i * 3]!;
        rgba[i * 4 + 1] = rgb[i * 3 + 1]!;
        rgba[i * 4 + 2] = rgb[i * 3 + 2]!;
        rgba[i * 4 + 3] = maskAlpha[i]!;
      }
      const key = amb.toLowerCase() as "day" | "fog" | "night";
      const file = `${key}.png`;
      images[file] = await sharp(rgba, { raw: { width: aw, height: ah, channels: 4 } })
        .png()
        .toBuffer();
      cutouts[key] = file;
    }

    const clipped = clipLevel(level, assetBbox);

    const desc: AssetDescriptor = {
      id: assetId,
      name: assetName,
      tags: opts.tags,
      scale_class: opts.scaleClass,
      variant_group: opts.variantGroup,
      source: {
        map: opts.map,
        ambiance: "Day",
        bbox: assetBbox,
        extraction: {
          tool: opts.mode === "crop" ? "rect-crop" : "fal-ai/sam-3/image-rle",
          prompt: opts.prompt,
          boxes: opts.boxes,
          points: opts.points?.map((p) => [p.x, p.y] as [number, number]),
          score: mask.score ?? undefined,
        },
      },
      origin: [ax, ay],
      // default anchor: bottom-center of the mask (ground contact line)
      anchor: [Math.round(aw / 2), ah - 1],
      images: { day: cutouts.day!, mask: "mask.png", fog: cutouts.fog, night: cutouts.night },
      volumes: clipped.volumes,
      motion: clipped.motion,
      jump_zones: clipped.jump_zones,
      jump_line_pairs: clipped.jump_line_pairs,
      lifts: clipped.lifts,
      material_sectors: clipped.material_sectors,
      occlusion_masks: clipped.occlusion_masks,
    };

    const dir = await writeAsset(desc, images);
    console.log(`wrote ${dir}`);
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
