// Terrain synthesis for map recreation/composition (see ../docs/terrain.md).
//
// Layers, in order: anti-tiled grass base → feathered dirt (Ground/Stone
// material sectors) → road strokes (authored polylines) → forest canopy
// (Leaves sectors, noise-displaced edge) → water (Water sectors, feathered,
// bank-shaded). Tree scatter along canopy edges is returned as stamp points
// for the caller to composite.
import path from "node:path";
import sharp from "sharp";
import type { Point, ProtoLevel } from "@rle/shared";
import { libraryDir } from "./env";

export interface Swatch {
  data: Buffer; // raw RGB
  width: number;
  height: number;
}

export async function loadSwatch(id: string): Promise<Swatch | null> {
  try {
    const img = sharp(path.join(libraryDir, id, "day.png")).removeAlpha();
    const { width, height } = await img.metadata();
    return { data: await img.raw().toBuffer(), width: width!, height: height! };
  } catch {
    return null;
  }
}

export interface Road {
  points: Point[];
  width: number;
}

export interface TerrainSpec {
  roads?: Road[];
}

// --- noise -----------------------------------------------------------------

function hash2(ix: number, iy: number): number {
  let h = (ix * 374761393 + iy * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}

const smooth = (t: number) => t * t * (3 - 2 * t);

/** value noise in [0,1], feature size ~= scale px */
function noise(x: number, y: number, scale: number): number {
  const fx = x / scale;
  const fy = y / scale;
  const ix = Math.floor(fx);
  const iy = Math.floor(fy);
  const tx = smooth(fx - ix);
  const ty = smooth(fy - iy);
  const a = hash2(ix, iy);
  const b = hash2(ix + 1, iy);
  const c = hash2(ix, iy + 1);
  const d = hash2(ix + 1, iy + 1);
  return a + (b - a) * tx + (c - a) * ty + (a - b - c + d) * tx * ty;
}

// --- masks -----------------------------------------------------------------

function fillPolyMask(mask: Float32Array, W: number, H: number, poly: Point[]) {
  if (poly.length < 3) return;
  let minY = Infinity,
    maxY = -Infinity;
  for (const [, y] of poly) {
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  for (let y = Math.max(0, Math.ceil(minY)); y <= Math.min(H - 1, Math.floor(maxY)); y++) {
    const xs: number[] = [];
    for (let i = 0; i < poly.length; i++) {
      const [x1, y1] = poly[i]!;
      const [x2, y2] = poly[(i + 1) % poly.length]!;
      if (y1 === y2) continue;
      if ((y >= y1 && y < y2) || (y >= y2 && y < y1)) {
        xs.push(x1 + ((y - y1) / (y2 - y1)) * (x2 - x1));
      }
    }
    xs.sort((a, b) => a - b);
    for (let k = 0; k + 1 < xs.length; k += 2) {
      const x0 = Math.max(0, Math.ceil(xs[k]!));
      const x1 = Math.min(W, Math.floor(xs[k + 1]!) + 1);
      mask.fill(1, y * W + x0, y * W + x1);
    }
  }
}

/** separable box blur, `passes` times (≈ gaussian) */
function boxBlur(mask: Float32Array, W: number, H: number, r: number, passes = 2) {
  if (r <= 0) return;
  const tmp = new Float32Array(mask.length);
  for (let p = 0; p < passes; p++) {
    // horizontal
    for (let y = 0; y < H; y++) {
      let sum = 0;
      const row = y * W;
      for (let x = -r; x <= r; x++) sum += mask[row + Math.min(W - 1, Math.max(0, x))]!;
      for (let x = 0; x < W; x++) {
        tmp[row + x] = sum / (2 * r + 1);
        const add = Math.min(W - 1, x + r + 1);
        const sub = Math.max(0, x - r);
        sum += mask[row + add]! - mask[row + sub]!;
      }
    }
    // vertical
    for (let x = 0; x < W; x++) {
      let sum = 0;
      for (let y = -r; y <= r; y++) sum += tmp[Math.min(H - 1, Math.max(0, y)) * W + x]!;
      for (let y = 0; y < H; y++) {
        mask[y * W + x] = sum / (2 * r + 1);
        const add = Math.min(H - 1, y + r + 1) * W + x;
        const sub = Math.max(0, y - r) * W + x;
        sum += tmp[add]! - tmp[sub]!;
      }
    }
  }
}

function segDist(px: number, py: number, x1: number, y1: number, x2: number, y2: number) {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len2 = dx * dx + dy * dy;
  const t = len2 === 0 ? 0 : Math.max(0, Math.min(1, ((px - x1) * dx + (py - y1) * dy) / len2));
  const ex = px - (x1 + t * dx);
  const ey = py - (y1 + t * dy);
  return Math.sqrt(ex * ex + ey * ey);
}

/** accumulate road stroke alpha (feathered, width jittered by noise) */
function strokeRoad(mask: Float32Array, W: number, H: number, road: Road) {
  const feather = 10;
  for (let i = 0; i + 1 < road.points.length; i++) {
    const [x1, y1] = road.points[i]!;
    const [x2, y2] = road.points[i + 1]!;
    const pad = road.width / 2 + feather + 14;
    const minX = Math.max(0, Math.floor(Math.min(x1, x2) - pad));
    const maxX = Math.min(W - 1, Math.ceil(Math.max(x1, x2) + pad));
    const minY = Math.max(0, Math.floor(Math.min(y1, y2) - pad));
    const maxY = Math.min(H - 1, Math.ceil(Math.max(y1, y2) + pad));
    for (let y = minY; y <= maxY; y++) {
      for (let x = minX; x <= maxX; x++) {
        const jitter = (noise(x, y, 90) - 0.5) * 18;
        const half = road.width / 2 + jitter;
        const d = segDist(x, y, x1, y1, x2, y2);
        if (d >= half + feather) continue;
        const a = d <= half ? 1 : 1 - (d - half) / feather;
        const idx = y * W + x;
        if (a > mask[idx]!) mask[idx] = a;
      }
    }
  }
}

// --- sampling --------------------------------------------------------------

/** anti-tiled swatch sample: two offset layers blended by large-scale noise */
function sampleSwatch(sw: Swatch, x: number, y: number): [number, number, number] {
  const m = (v: number, n: number) => ((v % n) + n) % n;
  const i1 = (m(y, sw.height) * sw.width + m(x, sw.width)) * 3;
  const i2 = (m(y + 311, sw.height) * sw.width + m(x + 173, sw.width)) * 3;
  const t = smooth(Math.min(1, Math.max(0, noise(x, y, 210) * 1.6 - 0.3)));
  return [
    sw.data[i1]! + (sw.data[i2]! - sw.data[i1]!) * t,
    sw.data[i1 + 1]! + (sw.data[i2 + 1]! - sw.data[i1 + 1]!) * t,
    sw.data[i1 + 2]! + (sw.data[i2 + 2]! - sw.data[i1 + 2]!) * t,
  ];
}

/** blend a swatch over the canvas wherever alpha > 0 */
function blendLayer(
  canvas: Buffer,
  W: number,
  H: number,
  alpha: Float32Array,
  sw: Swatch,
  shade?: (x: number, y: number, idx: number) => [number, number, number],
) {
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const idx = y * W + x;
      const a = alpha[idx]!;
      if (a <= 0.003) continue;
      const [r, g, b] = sampleSwatch(sw, x, y);
      const [sr, sg, sb] = shade ? shade(x, y, idx) : [1, 1, 1];
      const di = idx * 3;
      canvas[di] = canvas[di]! + (r * sr - canvas[di]!) * a;
      canvas[di + 1] = canvas[di + 1]! + (g * sg - canvas[di + 1]!) * a;
      canvas[di + 2] = canvas[di + 2]! + (b * sb - canvas[di + 2]!) * a;
    }
  }
}

// --- main ------------------------------------------------------------------

export interface TerrainResult {
  png: Buffer;
  /** canopy-edge points for tree/bush scatter stamping */
  scatterPoints: Point[];
}

export async function renderTerrain(
  level: ProtoLevel,
  W: number,
  H: number,
  spec: TerrainSpec,
  swatchIds: { grass: string; dirt: string; road: string; canopy: string; water: string },
): Promise<TerrainResult | null> {
  const grass = await loadSwatch(swatchIds.grass);
  const dirt = await loadSwatch(swatchIds.dirt);
  const road = await loadSwatch(swatchIds.road);
  const canopy = await loadSwatch(swatchIds.canopy);
  const water = await loadSwatch(swatchIds.water);
  if (!grass) return null;

  const canvas = Buffer.alloc(W * H * 3);
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const [r, g, b] = sampleSwatch(grass, x, y);
      // low-frequency luminance modulation breaks up residual tiling
      const lum =
        (0.9 + 0.2 * noise(x + 9000, y, 420)) * (0.95 + 0.1 * noise(x, y + 9000, 130));
      const di = (y * W + x) * 3;
      canvas[di] = Math.min(255, r * lum);
      canvas[di + 1] = Math.min(255, g * lum);
      canvas[di + 2] = Math.min(255, b * lum);
    }
  }

  const sectorsOf = (materials: number[]) =>
    level.material_sectors.filter((ms) => materials.includes(ms.material));

  // dirt: Ground(0) + Stone(2) sectors, feathered
  if (dirt) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([0, 2])) fillPolyMask(mask, W, H, ms.polygon.points);
    boxBlur(mask, W, H, 6);
    blendLayer(canvas, W, H, mask, dirt);
  }

  // roads
  if (road && spec.roads?.length) {
    const mask = new Float32Array(W * H);
    for (const r of spec.roads) strokeRoad(mask, W, H, r);
    blendLayer(canvas, W, H, mask, road);
  }

  // forest canopy: Leaves(4), noise-displaced edge for an organic boundary
  const scatterPoints: Point[] = [];
  if (canopy) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([4])) fillPolyMask(mask, W, H, ms.polygon.points);
    boxBlur(mask, W, H, 10);
    const displaced = new Float32Array(W * H);
    for (let y = 0; y < H; y++) {
      for (let x = 0; x < W; x++) {
        const dx = Math.round((noise(x + 5000, y, 70) - 0.5) * 46);
        const dy = Math.round((noise(x, y + 5000, 70) - 0.5) * 46);
        const sx = Math.min(W - 1, Math.max(0, x + dx));
        const sy = Math.min(H - 1, Math.max(0, y + dy));
        displaced[y * W + x] = mask[sy * W + sx]!;
      }
    }
    blendLayer(canvas, W, H, displaced, canopy, (x, y) => {
      const s = 0.86 + 0.28 * noise(x, y, 55);
      return [s, s, s];
    });
    // scatter along the boundary band
    for (let y = 0; y < H; y += 64) {
      for (let x = 0; x < W; x += 64) {
        const v = displaced[y * W + x]!;
        if (v > 0.25 && v < 0.85 && hash2(x, y) < 0.6) {
          scatterPoints.push([
            x + Math.round((hash2(x + 7, y) - 0.5) * 56),
            y + Math.round((hash2(x, y + 7) - 0.5) * 56),
          ]);
        }
      }
    }
  }

  // water: Water(5), feathered, banks shaded darker toward the shore
  if (water) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([5])) fillPolyMask(mask, W, H, ms.polygon.points);
    boxBlur(mask, W, H, 5);
    const interior = Float32Array.from(mask);
    boxBlur(interior, W, H, 22);
    blendLayer(canvas, W, H, mask, water, (x, y, idx) => {
      const depth = Math.min(1, interior[idx]!);
      const s = (0.66 + 0.3 * depth) * (0.96 + 0.08 * noise(x, y, 120));
      // cool the hue so water separates from grass
      return [s * 0.88, s * 0.97, s * 1.12];
    });
  }

  const png = await sharp(canvas, { raw: { width: W, height: H, channels: 3 } })
    .png()
    .toBuffer();
  return { png, scatterPoints };
}
