// Extract one asset from a map via SAM 3.
//
//   pnpm --filter pipeline extract -- --map Leicester --bbox 2900,400,600,500 \
//     --prompt "watermill building" --name "Leicester watermill" --tags building,water
//
// Flow: crop the Day map around --bbox (padded), send the crop to SAM 3 with
// the concept prompt (full-res masks since the crop is small), pick the best
// mask, tighten the bbox to the mask, cut Day/Fog/Night pixels with the mask
// as alpha, clip intersecting level metadata to asset-local coords, and write
// everything to library/<id>/. A review sheet lands in work/<id>/.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import type { AssetDescriptor, ProtoLevel } from "@rle/shared";
import { datadirPath, workDir } from "./env";
import { segment, type SamMask } from "./sam";
import { clipLevel, type Bbox } from "./clip";
import { writeAsset } from "./library";

interface Args {
  map: string;
  bbox: Bbox;
  prompt: string;
  name: string;
  id?: string;
  tags: string[];
  pad: number;
  maxMasks: number;
  pick: number | "best";
  scaleClass: "unique" | "variant" | "spline-segment";
}

function parseArgs(argv: string[]): Args {
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const req = (flag: string): string => {
    const v = get(flag);
    if (!v) throw new Error(`missing --${flag}`);
    return v;
  };
  const bbox = req("bbox").split(",").map(Number);
  if (bbox.length !== 4 || bbox.some(Number.isNaN)) throw new Error("--bbox wants x,y,w,h");
  return {
    map: req("map"),
    bbox: bbox as Bbox,
    prompt: req("prompt"),
    name: req("name"),
    id: get("id"),
    tags: get("tags")?.split(",") ?? [],
    pad: Number(get("pad") ?? 48),
    maxMasks: Number(get("max-masks") ?? 8),
    pick: get("pick") === undefined ? "best" : Number(get("pick")),
    scaleClass: (get("scale-class") as Args["scaleClass"]) ?? "unique",
  };
}

function slugify(s: string): string {
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
  if (maxX < 0) throw new Error("mask is empty");
  return { minX, minY, maxX, maxY };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const levelsDir = path.join(datadirPath(), "Data", "Levels");
  const id = args.id ?? slugify(args.name);

  const dayPath = await findMapPng(levelsDir, "Day", args.map);
  if (!dayPath) throw new Error(`no Day/${args.map}.map.png under ${levelsDir}`);

  const day = sharp(dayPath);
  const meta = await day.metadata();
  const mapW = meta.width!;
  const mapH = meta.height!;

  // padded crop around the requested bbox, clamped to the map
  const [bx, by, bw, bh] = args.bbox;
  const cx = Math.max(0, Math.floor(bx - args.pad));
  const cy = Math.max(0, Math.floor(by - args.pad));
  const cw = Math.min(mapW - cx, Math.ceil(bw + 2 * args.pad));
  const ch = Math.min(mapH - cy, Math.ceil(bh + 2 * args.pad));

  const cropPng = await sharp(dayPath)
    .extract({ left: cx, top: cy, width: cw, height: ch })
    .png()
    .toBuffer();

  console.log(`SAM 3: "${args.prompt}" on ${cw}x${ch} crop of ${args.map} @ ${cx},${cy}`);
  const masks = await segment({
    imagePng: cropPng,
    width: cw,
    height: ch,
    prompt: args.prompt,
    maxMasks: args.maxMasks,
  });
  console.log(
    `got ${masks.length} mask(s), scores: ${masks.map((m) => m.score?.toFixed(3) ?? "?").join(", ")}`,
  );

  const chosen: SamMask =
    args.pick === "best"
      ? masks.reduce((a, b) => ((b.score ?? 0) > (a.score ?? 0) ? b : a))
      : masks[args.pick] ??
        (() => {
          throw new Error(`--pick ${args.pick} out of range (${masks.length} masks)`);
        })();

  // resize mask to crop resolution
  // note: sharp's resize() promotes 1-channel raw input to 3 channels;
  // extractChannel(0) forces it back to a single channel
  const maskResized = await sharp(Buffer.from(chosen.data), {
    raw: { width: chosen.width, height: chosen.height, channels: 1 },
  })
    .resize(cw, ch, { kernel: "nearest" })
    .extractChannel(0)
    .raw()
    .toBuffer();
  const cropMask = { data: new Uint8Array(maskResized), width: cw, height: ch };

  // tighten to mask bounds (asset bbox in map coords)
  const b = maskBounds(cropMask);
  const ax = cx + b.minX;
  const ay = cy + b.minY;
  const aw = b.maxX - b.minX + 1;
  const ah = b.maxY - b.minY + 1;
  console.log(`asset bbox: ${ax},${ay} ${aw}x${ah}`);

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
    const mapPath = amb === "Day" ? dayPath : await findMapPng(levelsDir, amb, args.map);
    if (!mapPath) continue;
    const rgb = await sharp(mapPath)
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

  // clip level metadata to the asset region
  const level: ProtoLevel = JSON.parse(
    await fs.readFile(path.join(levelsDir, `${args.map}.rhp.json`), "utf8"),
  );
  const clipped = clipLevel(level, [ax, ay, aw, ah]);

  const desc: AssetDescriptor = {
    id,
    name: args.name,
    tags: args.tags,
    scale_class: args.scaleClass,
    source: {
      map: args.map,
      ambiance: "Day",
      bbox: [ax, ay, aw, ah],
      extraction: {
        tool: "fal-ai/sam-3/image-rle",
        prompt: args.prompt,
        score: chosen.score ?? undefined,
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

  // review sheet: crop + all candidate masks side by side
  const reviewDir = path.join(workDir, id);
  await fs.mkdir(reviewDir, { recursive: true });
  await fs.writeFile(path.join(reviewDir, "crop.png"), cropPng);
  for (let i = 0; i < masks.length; i++) {
    const m = masks[i]!;
    const resized = await sharp(Buffer.from(m.data), {
      raw: { width: m.width, height: m.height, channels: 1 },
    })
      .resize(cw, ch, { kernel: "nearest" })
      .extractChannel(0)
      .raw()
      .toBuffer();
    // red overlay where masked
    const overlay = Buffer.alloc(cw * ch * 4);
    for (let p = 0; p < cw * ch; p++) {
      if (resized[p]) {
        overlay[p * 4] = 255;
        overlay[p * 4 + 3] = 110;
      }
    }
    const sheet = await sharp(cropPng)
      .composite([
        { input: overlay, raw: { width: cw, height: ch, channels: 4 }, blend: "over" },
      ])
      .png()
      .toBuffer();
    await fs.writeFile(
      path.join(reviewDir, `mask-${i}${i === masks.indexOf(chosen) ? "-CHOSEN" : ""}.png`),
      sheet,
    );
  }
  console.log(`review sheets in ${reviewDir}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
