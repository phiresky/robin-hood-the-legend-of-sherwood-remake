// Cut Day/Fog/Night pixels with a mask, clip level metadata, and write a
// library asset. Shared by the SAM 2D extraction flow and the detection-driven
// 3D reconstruction flow (which brings its own masks).
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import {
  parseProtoLevel,
  type AssetDescriptor,
  type ProtoLevel,
} from "@rle/shared";
import { isMissing } from "./provider-cache";
import { datadirPath } from "./env";
import { clipLevel, type Bbox } from "./clip";
import { writeAsset } from "./library";
import { fxTopLeft, loadFxSprite, loadKeyedFxPng } from "./fx";

export function levelsDirPath(): string {
  return path.join(datadirPath(), "Data", "Levels");
}

export async function findMapPng(
  ambiance: string,
  map: string,
): Promise<string | null> {
  const dir = path.join(levelsDirPath(), ambiance);
  try {
    const files = await fs.readdir(dir);
    const hit = files.find(
      (f) => f.toLowerCase() === `${map.toLowerCase()}.map.png`,
    );
    return hit ? path.join(dir, hit) : null;
  } catch (error) {
    if (isMissing(error)) return null;
    throw error;
  }
}

export async function loadProtoLevel(map: string): Promise<ProtoLevel> {
  const dir = levelsDirPath();
  const files = await fs.readdir(dir);
  const hit = files.find(
    (f) => f.toLowerCase() === `${map.toLowerCase()}.rhp.json`,
  );
  if (!hit) throw new Error(`no ${map}.rhp.json under ${dir}`);
  return parseProtoLevel(
    JSON.parse(await fs.readFile(path.join(dir, hit), "utf8")),
  );
}

// full-map images with the non-integrated patch sprites (roof closers)
// composited on, cached per map+ambiance for the run
const patchedMapCache = new Map<string, Buffer>();

export async function patchedMapImage(
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

/** the map image for an ambiance, optionally with roof-closer patches applied */
export async function mapImageSource(
  map: string,
  ambiance: string,
  applyPatches: boolean,
  level: ProtoLevel,
): Promise<string | Buffer | null> {
  const mapPath = await findMapPng(ambiance, map);
  if (!mapPath) return null;
  return applyPatches
    ? patchedMapImage(map, ambiance, mapPath, level)
    : mapPath;
}

export interface MaskedAssetSpec {
  map: string;
  level: ProtoLevel;
  applyPatches: boolean;
  /** 8-bit mask covering exactly `bbox` (row-major, bbox w x h) */
  mask: Uint8Array;
  /** tight map-pixel bbox of the mask [x, y, w, h] */
  bbox: Bbox;
  id: string;
  name: string;
  tags: string[];
  scaleClass: AssetDescriptor["scale_class"];
  variantGroup?: string;
  extraction: AssetDescriptor["source"]["extraction"];
}

/** cut all ambiances with the mask, clip level metadata, write library/<id>/ */
export async function writeMaskedAsset(
  spec: MaskedAssetSpec,
): Promise<AssetDescriptor> {
  const [ax, ay, aw, ah] = spec.bbox;
  if (spec.mask.length !== aw * ah) {
    throw new Error(
      `${spec.id}: mask size ${spec.mask.length} != bbox area ${aw * ah}`,
    );
  }
  const maskPng = await sharp(Buffer.from(spec.mask), {
    raw: { width: aw, height: ah, channels: 1 },
  })
    .png()
    .toBuffer();

  const images: Record<string, Buffer> = { "mask.png": maskPng };
  const cutouts: Partial<Record<"day" | "fog" | "night", string>> = {};
  for (const amb of ["Day", "Fog", "Night"] as const) {
    const src = await mapImageSource(
      spec.map,
      amb,
      spec.applyPatches,
      spec.level,
    );
    if (!src) continue;
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
      rgba[i * 4 + 3] = spec.mask[i]!;
    }
    const key = amb.toLowerCase() as "day" | "fog" | "night";
    const file = `${key}.png`;
    images[file] = await sharp(rgba, {
      raw: { width: aw, height: ah, channels: 4 },
    })
      .png()
      .toBuffer();
    cutouts[key] = file;
  }
  if (!cutouts.day) throw new Error(`${spec.id}: no Day map for ${spec.map}`);

  const clipped = clipLevel(spec.level, spec.bbox);
  const desc: AssetDescriptor = {
    id: spec.id,
    name: spec.name,
    tags: spec.tags,
    scale_class: spec.scaleClass,
    variant_group: spec.variantGroup,
    source: {
      map: spec.map,
      ambiance: "Day",
      bbox: spec.bbox,
      extraction: spec.extraction,
    },
    origin: [ax, ay],
    // default anchor: bottom-center of the mask (ground contact line)
    anchor: [Math.round(aw / 2), ah - 1],
    images: {
      day: cutouts.day,
      mask: "mask.png",
      fog: cutouts.fog,
      night: cutouts.night,
    },
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
  return desc;
}
