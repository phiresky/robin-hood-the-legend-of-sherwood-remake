// Terrain synthesis core — pure buffer math, shared by the pipeline (sharp
// I/O) and the editor (canvas I/O). See level-editor/docs/terrain.md.
//
// Layers, in order: anti-tiled grass base → feathered dirt (Ground/Stone
// material sectors + authored regions) → road strokes → forest canopy
// (noise-displaced edge) → water (feathered, bank-shaded). Tree scatter along
// canopy is returned as stamp points for the caller to composite.
import type { Point } from "./level";
import type { WallRun } from "./draft";

export interface SwatchData {
  /** raw RGB, row-major */
  data: Uint8Array;
  width: number;
  height: number;
}

export interface Road {
  points: Point[];
  width: number;
}

export interface TerrainRegion {
  material: "grass" | "dirt" | "canopy" | "water";
  polygon: Point[];
}

export type SwatchRole = "grass" | "dirt" | "road" | "canopy" | "water";

export interface TerrainSpec {
  roads?: Road[];
  /** authored regions merged with the proto level's material sectors */
  regions?: TerrainRegion[];
  /** authored wall runs (recreation copies them into the draft) */
  walls?: WallRun[];
  /** library asset ids used as texture swatches, per role */
  swatches?: Partial<Record<SwatchRole, string>>;
}

/** material polygons from the proto level (GameMaterial numbering) */
export interface TerrainMaterialSector {
  material: number;
  points: Point[];
}

// --- noise -----------------------------------------------------------------

export function hash2(ix: number, iy: number): number {
  let h = (ix * 374761393 + iy * 668265263) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967295;
}

const smooth = (t: number) => t * t * (3 - 2 * t);

/** value noise in [0,1], feature size ~= scale px */
export function noise(x: number, y: number, scale: number): number {
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

export function fillPolyMask(mask: Float32Array, W: number, H: number, poly: Point[]) {
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
      if (x1 > x0) mask.fill(1, y * W + x0, y * W + x1);
    }
  }
}

/** separable box blur, `passes` times (≈ gaussian) */
export function boxBlur(mask: Float32Array, W: number, H: number, r: number, passes = 2) {
  if (r <= 0) return;
  const tmp = new Float32Array(mask.length);
  for (let p = 0; p < passes; p++) {
    for (let y = 0; y < H; y++) {
      let sum = 0;
      const row = y * W;
      for (let x = -r; x <= r; x++) sum += mask[row + Math.min(W - 1, Math.max(0, x))]!;
      for (let x = 0; x < W; x++) {
        tmp[row + x] = sum / (2 * r + 1);
        sum += mask[row + Math.min(W - 1, x + r + 1)]! - mask[row + Math.max(0, x - r)]!;
      }
    }
    for (let x = 0; x < W; x++) {
      let sum = 0;
      for (let y = -r; y <= r; y++) sum += tmp[Math.min(H - 1, Math.max(0, y)) * W + x]!;
      for (let y = 0; y < H; y++) {
        mask[y * W + x] = sum / (2 * r + 1);
        sum += tmp[Math.min(H - 1, y + r + 1) * W + x]! - tmp[Math.max(0, y - r) * W + x]!;
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
export function strokeRoad(mask: Float32Array, W: number, H: number, road: Road) {
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
export function sampleSwatch(sw: SwatchData, x: number, y: number): [number, number, number] {
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

function blendLayer(
  canvas: Uint8Array,
  W: number,
  H: number,
  alpha: Float32Array,
  sw: SwatchData,
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

export interface TerrainCoreResult {
  /** raw RGB, row-major, W×H */
  rgb: Uint8Array;
  /** canopy points for tree/bush scatter stamping */
  scatterPoints: Point[];
}

export interface TerrainCoreOptions {
  W: number;
  H: number;
  /** proto-level material sectors (empty for from-scratch maps) */
  materialSectors: TerrainMaterialSector[];
  spec: TerrainSpec;
  swatches: Partial<Record<SwatchRole, SwatchData>>;
  /**
   * render scale: world coords in sectors/spec are divided by this (use >1
   * for a downscaled preview; noise/feather sizes scale along)
   */
  scale?: number;
}

export function renderTerrainCore(opts: TerrainCoreOptions): TerrainCoreResult | null {
  const { W, H, spec } = opts;
  const s = opts.scale ?? 1;
  const grass = opts.swatches.grass;
  if (!grass) return null;
  const sc = (poly: Point[]): Point[] =>
    s === 1 ? poly : poly.map(([x, y]) => [x / s, y / s]);
  const r = (v: number) => Math.max(1, Math.round(v / s));

  const canvas = new Uint8Array(W * H * 3);
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const [rr, gg, bb] = sampleSwatch(grass, x, y);
      const lum =
        (0.9 + 0.2 * noise(x + 9000, y, r(420))) * (0.95 + 0.1 * noise(x, y + 9000, r(130)));
      const di = (y * W + x) * 3;
      canvas[di] = Math.min(255, rr * lum);
      canvas[di + 1] = Math.min(255, gg * lum);
      canvas[di + 2] = Math.min(255, bb * lum);
    }
  }

  const sectorsOf = (materials: number[]) =>
    opts.materialSectors.filter((ms) => materials.includes(ms.material));
  const regionsOf = (material: TerrainRegion["material"]) =>
    (spec.regions ?? []).filter((rg) => rg.material === material);

  const dirt = opts.swatches.dirt;
  if (dirt) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([0, 2])) fillPolyMask(mask, W, H, sc(ms.points));
    for (const rg of regionsOf("dirt")) fillPolyMask(mask, W, H, sc(rg.polygon));
    boxBlur(mask, W, H, r(6));
    blendLayer(canvas, W, H, mask, dirt);
  }

  const road = opts.swatches.road;
  if (road && spec.roads?.length) {
    const mask = new Float32Array(W * H);
    for (const rd of spec.roads) {
      strokeRoad(mask, W, H, { points: sc(rd.points), width: rd.width / s });
    }
    blendLayer(canvas, W, H, mask, road);
  }

  const scatterPoints: Point[] = [];
  const canopy = opts.swatches.canopy;
  if (canopy) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([4])) fillPolyMask(mask, W, H, sc(ms.points));
    for (const rg of regionsOf("canopy")) fillPolyMask(mask, W, H, sc(rg.polygon));
    boxBlur(mask, W, H, r(10));
    const displaced = new Float32Array(W * H);
    for (let y = 0; y < H; y++) {
      for (let x = 0; x < W; x++) {
        const dx = Math.round((noise(x + 5000, y, r(70)) - 0.5) * r(46));
        const dy = Math.round((noise(x, y + 5000, r(70)) - 0.5) * r(46));
        const sx = Math.min(W - 1, Math.max(0, x + dx));
        const sy = Math.min(H - 1, Math.max(0, y + dy));
        displaced[y * W + x] = mask[sy * W + sx]!;
      }
    }
    blendLayer(canvas, W, H, displaced, canopy, (x, y) => {
      const sh = 0.86 + 0.28 * noise(x, y, r(55));
      return [sh, sh, sh];
    });
    const step = r(88);
    for (let y = 0; y < H; y += step) {
      for (let x = 0; x < W; x += step) {
        const v = displaced[y * W + x]!;
        const inside = v >= 0.85;
        const edge = v > 0.25 && v < 0.85;
        if ((inside && hash2(x, y) < 0.8) || (edge && hash2(x, y) < 0.55)) {
          scatterPoints.push([
            (x + Math.round((hash2(x + 7, y) - 0.5) * r(80))) * s,
            (y + Math.round((hash2(x, y + 7) - 0.5) * r(80))) * s,
          ]);
        }
      }
    }
  }

  const water = opts.swatches.water;
  if (water) {
    const mask = new Float32Array(W * H);
    for (const ms of sectorsOf([5])) fillPolyMask(mask, W, H, sc(ms.points));
    for (const rg of regionsOf("water")) fillPolyMask(mask, W, H, sc(rg.polygon));
    boxBlur(mask, W, H, r(5));
    const interior = Float32Array.from(mask);
    boxBlur(interior, W, H, r(22));
    blendLayer(canvas, W, H, mask, water, (x, y, idx) => {
      const depth = Math.min(1, interior[idx]!);
      const sh = (0.66 + 0.3 * depth) * (0.96 + 0.08 * noise(x, y, r(120)));
      return [sh * 0.88, sh * 0.97, sh * 1.12];
    });
  }

  return { rgb: canvas, scatterPoints };
}
