// Reading FX/patch sprites from the hackable datadir's .rhs.d directories
// (Data/Animations/<Ambiance>/<bank>.rhs.d/manifest.json + per-row frame PNGs).
//
// Draw convention for animation/patch sprites: the serialized position IS the
// sprite's top-left. The engine builds the anchor as (pos + center, +elev) and
// draws at floor(anchor_map - center) + frame offset, so the frame's top-left
// lands at (position_x + offset_x, position_y + offset_y); elevation shifts
// the 3D anchor but cancels out of the projected draw position.
import fs from "node:fs/promises";
import path from "node:path";
import sharp from "sharp";
import { datadirPath } from "./env";

/**
 * The hackable converter currently exports sprite-bank frames with the RGB565
 * transparency key baked in as opaque green (0,251,0) instead of alpha.
 * TODO: fix in convert_datadir (Rust) and drop this.
 */
export async function loadKeyedFxPng(framePath: string): Promise<Buffer> {
  const img = sharp(framePath).ensureAlpha();
  const { width, height } = await img.metadata();
  const raw = await img.raw().toBuffer();
  for (let i = 0; i < raw.length; i += 4) {
    if (raw[i] === 0 && raw[i + 1] === 251 && raw[i + 2] === 0) raw[i + 3] = 0;
  }
  return sharp(raw, { raw: { width: width!, height: height!, channels: 4 } })
    .png()
    .toBuffer();
}

export interface RhsFrame {
  delay: number;
  distance: number;
  file: string;
  offset_x: number;
  offset_y: number;
  sound_id: number;
}

export interface RhsRow {
  action: string;
  action_id: number;
  path: string;
  hotspot_x: number;
  hotspot_y: number;
  frames: RhsFrame[];
}

export interface RhsProfile {
  name: string;
  center_x: number;
  center_y: number;
  width: number;
  height: number;
  rows: RhsRow[];
}

interface Manifest {
  profiles: RhsProfile[];
}

const manifestCache = new Map<string, Manifest | null>();

async function loadManifest(ambiance: string, bank: string): Promise<Manifest | null> {
  const key = `${ambiance}/${bank}`;
  if (manifestCache.has(key)) return manifestCache.get(key)!;
  const dir = path.join(datadirPath(), "Data", "Animations", ambiance);
  let result: Manifest | null = null;
  try {
    const entries = await fs.readdir(dir);
    const hit = entries.find((e) => e.toLowerCase() === `${bank.toLowerCase()}.rhs.d`);
    if (hit) {
      result = JSON.parse(await fs.readFile(path.join(dir, hit, "manifest.json"), "utf8"));
      (result as Manifest & { _dir?: string })._dir = path.join(dir, hit);
    }
  } catch {
    result = null;
  }
  manifestCache.set(key, result);
  return result;
}

export interface FxSprite {
  /** absolute path of the first frame PNG */
  framePath: string;
  frameCount: number;
  centerX: number;
  centerY: number;
  hotspotX: number;
  hotspotY: number;
  offsetX: number;
  offsetY: number;
  width: number;
  height: number;
  action: string;
}

/** first row / first frame of a profile — the sprite's representative still */
export async function loadFxSprite(
  ambiance: string,
  bank: string,
  profileName: string,
): Promise<FxSprite | null> {
  const manifest = await loadManifest(ambiance, bank);
  if (!manifest) return null;
  const profile = manifest.profiles.find((p) => p.name === profileName);
  if (!profile) return null;
  const row = profile.rows[0];
  const frame = row?.frames[0];
  if (!row || !frame) return null;
  const dir = (manifest as Manifest & { _dir?: string })._dir!;
  // single-profile banks store frames directly under <bank>.rhs.d/<row.path>/
  let framePath = path.join(dir, profileName, row.path, frame.file);
  try {
    await fs.access(framePath);
  } catch {
    framePath = path.join(dir, row.path, frame.file);
  }
  return {
    framePath,
    frameCount: row.frames.length,
    centerX: profile.center_x,
    centerY: profile.center_y,
    hotspotX: row.hotspot_x,
    hotspotY: row.hotspot_y,
    offsetX: frame.offset_x,
    offsetY: frame.offset_y,
    width: profile.width,
    height: profile.height,
    action: row.action,
  };
}

/** top-left position for compositing an animation/patch sprite */
export function fxTopLeft(
  fx: FxSprite,
  positionX: number,
  positionY: number,
  _elevation: number,
): [number, number] {
  return [Math.round(positionX + fx.offsetX), Math.round(positionY + fx.offsetY)];
}
