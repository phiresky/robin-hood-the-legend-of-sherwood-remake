import { execFile } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import sharp from "sharp";
import { sceneToMap, type MapCamera } from "@rle/shared";
import { cross, type Box, type Geometry } from "./volume-geometry.ts";
import type { Owners } from "./volume-raster.ts";
import { workDir } from "./env.ts";
export type Fill = "proc" | "synth" | "smear" | "none";
const MAX_ATLAS = 16384;
const MAP_MARGIN = 512;
interface Tile {
  face: number;
  /** map-pixel bbox of the face's projection */
  x0: number;
  y0: number;
  w: number;
  h: number;
  /** owned (known) pixels in the tile */
  known: number;
  /**
   * faces seen edge-on (and synthesized hidden faces) get a tile in their
   * own local frame instead of map pixels: x = (u - u0) × scale, y = (v1 - v) × scale
   */
  local: {
    u0: number;
    v0: number;
    u1: number;
    v1: number;
    scale: number;
  } | null;
  /** pixels that are trustworthy sources: own, faithfully reflected, or copied from a donor's valid pixels */
  valid: Uint8Array | null;
  /** pixels inside the face's polygon */
  inside: Uint8Array | null;
  rgba: Buffer;
  /** placement in the atlas */
  ax: number;
  ay: number;
}

// ── tiles and procedural fill ────────────────────────────────────────

/**
 * Fill alpha-0 pixels of an RGBA image from its known pixels by iterative
 * 8-neighbour averaging. Returns how many pixels stayed unknown.
 */
function smearFill(rgba: Buffer, w: number, h: number, maxIter = 600): number {
  const known = new Uint8Array(w * h);
  let unknown = 0;
  for (let i = 0; i < w * h; i++) {
    known[i] = rgba[i * 4 + 3]! > 0 ? 1 : 0;
    if (!known[i]) unknown++;
  }
  if (unknown === 0) return 0;
  const acc = new Float32Array(w * h * 3);
  for (let i = 0; i < w * h; i++) {
    acc[i * 3] = rgba[i * 4]!;
    acc[i * 3 + 1] = rgba[i * 4 + 1]!;
    acc[i * 3 + 2] = rgba[i * 4 + 2]!;
  }
  for (let iter = 0; iter < maxIter && unknown > 0; iter++) {
    const next = new Uint8Array(known);
    let progressed = false;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = y * w + x;
        if (known[i]) continue;
        let r = 0,
          gg = 0,
          b = 0,
          n = 0;
        for (let dy = -1; dy <= 1; dy++) {
          const yy = y + dy;
          if (yy < 0 || yy >= h) continue;
          for (let dx = -1; dx <= 1; dx++) {
            const xx = x + dx;
            if (xx < 0 || xx >= w) continue;
            const j = yy * w + xx;
            if (!known[j]) continue;
            r += acc[j * 3]!;
            gg += acc[j * 3 + 1]!;
            b += acc[j * 3 + 2]!;
            n++;
          }
        }
        if (n === 0) continue;
        acc[i * 3] = r / n;
        acc[i * 3 + 1] = gg / n;
        acc[i * 3 + 2] = b / n;
        next[i] = 1;
        unknown--;
        progressed = true;
      }
    }
    known.set(next);
    if (!progressed) break;
  }
  for (let i = 0; i < w * h; i++) {
    if (rgba[i * 4 + 3]! > 0 || !known[i]) continue;
    rgba[i * 4] = Math.round(acc[i * 3]!);
    rgba[i * 4 + 1] = Math.round(acc[i * 3 + 1]!);
    rgba[i * 4 + 2] = Math.round(acc[i * 3 + 2]!);
    rgba[i * 4 + 3] = 255;
  }
  return unknown;
}

/** mirror-repeat a coordinate into [0, L) */
function mirrorMod(x: number, L: number): number {
  if (L <= 0) return 0;
  const m = ((x % (2 * L)) + 2 * L) % (2 * L);
  return m < L ? m : 2 * L - m;
}

/** mirror-repeat index into a run of length L: 0,1,…,L-1,L-1,…,1,0,0,1,… */
function pingpong(k: number, L: number): number {
  if (L <= 1) return 0;
  const m = k % (2 * L);
  return m < L ? m : 2 * L - 1 - m;
}

/**
 * Fill alpha-0 pixels by reflecting the image across the visibility
 * boundary: every unknown pixel takes the known pixel mirrored through its
 * nearest known pixel (a 2D reflection, so a diagonal boundary mirrors the
 * picture instead of smearing it into streaks). With repeat, pixels farther
 * from the boundary than the known run behind it mirror-repeat that run;
 * without, they stay unknown. Returns how many pixels stayed unknown.
 */
function reflectFill(
  rgba: Buffer,
  w: number,
  h: number,
  repeat: boolean,
  maxRun = 512,
  valid: Uint8Array | null = null,
): number {
  const n = w * h;
  const known = new Uint8Array(n);
  let unknown = 0;
  for (let i = 0; i < n; i++) {
    known[i] = rgba[i * 4 + 3]! > 0 ? 1 : 0;
    if (!known[i]) unknown++;
  }
  if (unknown === 0 || unknown === n) return unknown;
  // nearest known pixel per pixel: two-pass chamfer propagation of indices
  const nearest = new Int32Array(n).fill(-1);
  const d2 = new Float32Array(n).fill(Infinity);
  for (let i = 0; i < n; i++) {
    if (known[i]) {
      nearest[i] = i;
      d2[i] = 0;
    }
  }
  const consider = (i: number, x: number, y: number, j: number) => {
    const c = nearest[j]!;
    if (c < 0) return;
    const cx = c % w;
    const cy = (c - cx) / w;
    const dd = (x - cx) * (x - cx) + (y - cy) * (y - cy);
    if (dd < d2[i]!) {
      d2[i] = dd;
      nearest[i] = c;
    }
  };
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      if (known[i]) continue;
      if (x > 0) consider(i, x, y, i - 1);
      if (y > 0) {
        consider(i, x, y, i - w);
        if (x > 0) consider(i, x, y, i - w - 1);
        if (x < w - 1) consider(i, x, y, i - w + 1);
      }
    }
  }
  for (let y = h - 1; y >= 0; y--) {
    for (let x = w - 1; x >= 0; x--) {
      const i = y * w + x;
      if (known[i]) continue;
      if (x < w - 1) consider(i, x, y, i + 1);
      if (y < h - 1) {
        consider(i, x, y, i + w);
        if (x < w - 1) consider(i, x, y, i + w + 1);
        if (x > 0) consider(i, x, y, i + w - 1);
      }
    }
  }
  const out = Buffer.from(rgba);
  for (let i = 0; i < n; i++) {
    if (known[i]) continue;
    const c = nearest[i]!;
    if (c < 0) continue;
    const x = i % w;
    const y = (i - x) / w;
    const cx = c % w;
    const cy = (c - cx) / w;
    const d = Math.hypot(x - cx, y - cy);
    const ux = (x - cx) / d;
    const uy = (y - cy) / d;
    // known run behind the boundary point along the reflection direction
    let run = 1;
    while (run < maxRun) {
      const qx = Math.round(cx - ux * run);
      const qy = Math.round(cy - uy * run);
      if (qx < 0 || qy < 0 || qx >= w || qy >= h || !known[qy * w + qx]) break;
      run++;
    }
    if (!repeat && d > run) continue;
    const back = mirrorMod(d, run);
    let sx = Math.round(cx - ux * back);
    let sy = Math.round(cy - uy * back);
    if (sx < 0 || sy < 0 || sx >= w || sy >= h || !known[sy * w + sx]) {
      sx = cx;
      sy = cy;
    }
    const j = (sy * w + sx) * 4;
    out[i * 4] = rgba[j]!;
    out[i * 4 + 1] = rgba[j + 1]!;
    out[i * 4 + 2] = rgba[j + 2]!;
    out[i * 4 + 3] = 255;
    if (valid && !repeat) valid[i] = 1;
    unknown--;
  }
  out.copy(rgba);
  return unknown;
}

/** candidate copy directions for patchFill: 16 angles */
const PATCH_DIRS: [number, number][] = Array.from({ length: 16 }, (_, a) => [
  Math.cos((a * Math.PI) / 8),
  Math.sin((a * Math.PI) / 8),
]);
/** shift magnitudes as multiples of the region's extent along the direction */
const PATCH_FACTORS = [0.35, 0.6, 1.05, 1.3, 1.6, 2, 2.5, 3.2];

/**
 * Fill alpha-0 pixels of a ground image by copying coherent patches of the
 * nearby known ground. Every connected unknown region is filled by the
 * translation that brings the most known ground onto it while matching the
 * colours along the region's border best; what that leaves uncovered gets
 * the next best translation, and so on. Only original pixels are ever
 * copied. Returns how many pixels stayed unknown.
 */
function patchFill(rgba: Buffer, w: number, h: number): number {
  const n = w * h;
  const known = new Uint8Array(n);
  /** which copy (1-based, per region and round) filled a pixel; 0 = original */
  const source = new Int32Array(n);
  let copies = 0;
  let unknown = 0;
  for (let i = 0; i < n; i++) {
    known[i] = rgba[i * 4 + 3]! > 0 ? 1 : 0;
    if (!known[i]) unknown++;
  }
  if (unknown === 0) return 0;
  const visited = new Uint8Array(n);
  const queue = new Int32Array(unknown);
  const ringMark = new Uint8Array(n);
  const ring: number[] = [];
  const colourErr = (a: number, b: number) => {
    const dr = rgba[a * 4]! - rgba[b * 4]!;
    const dg = rgba[a * 4 + 1]! - rgba[b * 4 + 1]!;
    const db = rgba[a * 4 + 2]! - rgba[b * 4 + 2]!;
    return dr * dr + dg * dg + db * db;
  };

  for (let seed = 0; seed < n; seed++) {
    if (known[seed] || visited[seed]) continue;
    // flood the region (4-connected), collecting its bbox and border ring
    let head = 0;
    let len = 0;
    queue[len++] = seed;
    visited[seed] = 1;
    let bx0 = w,
      by0 = h,
      bx1 = 0,
      by1 = 0;
    ring.length = 0;
    while (head < len) {
      const p = queue[head++]!;
      const x = p % w;
      const y = (p - x) / w;
      if (x < bx0) bx0 = x;
      if (x > bx1) bx1 = x;
      if (y < by0) by0 = y;
      if (y > by1) by1 = y;
      const nb = [
        x > 0 ? p - 1 : -1,
        x < w - 1 ? p + 1 : -1,
        y > 0 ? p - w : -1,
        y < h - 1 ? p + w : -1,
      ];
      for (const q of nb) {
        if (q < 0) continue;
        if (known[q]) {
          if (!ringMark[q]) {
            ringMark[q] = 1;
            ring.push(q);
          }
        } else if (!visited[q]) {
          visited[q] = 1;
          queue[len++] = q;
        }
      }
    }
    for (const q of ring) ringMark[q] = 0;
    const bw = bx1 - bx0 + 1;
    const bh = by1 - by0 + 1;
    const ringStep = Math.max(1, Math.ceil(ring.length / 3000));

    let remaining = Array.from(queue.subarray(0, len));
    for (let round = 0; round < 8 && remaining.length > 0; round++) {
      const stride = Math.max(1, Math.ceil(remaining.length / 20000));
      const samples = Math.ceil(remaining.length / stride);
      // coverage of every candidate shift
      const cands: { dx: number; dy: number; cover: number }[] = [];
      let bestCover = 0;
      for (const [cx, cy] of PATCH_DIRS) {
        const extent = Math.abs(cx) * bw + Math.abs(cy) * bh;
        for (const f of PATCH_FACTORS) {
          const dx = Math.round(cx * extent * f);
          const dy = Math.round(cy * extent * f);
          let cover = 0;
          for (let k = 0; k < remaining.length; k += stride) {
            const p = remaining[k]!;
            const x = p % w;
            const y = (p - x) / w;
            const sx = x + dx;
            const sy = y + dy;
            if (sx < 0 || sx >= w || sy < 0 || sy >= h) continue;
            if (known[sy * w + sx]) cover++;
          }
          const c = cover / samples;
          if (c > bestCover) bestCover = c;
          cands.push({ dx, dy, cover: c });
        }
      }
      if (bestCover < 0.05) break;
      // among the well-covering shifts, the one whose source matches the
      // colours around the region best
      let best: { dx: number; dy: number } | null = null;
      let bestErr = Infinity;
      for (const c of cands) {
        if (c.cover < 0.85 * bestCover) continue;
        let err = 0;
        let cnt = 0;
        let total = 0;
        for (let k = 0; k < ring.length; k += ringStep) {
          const r = ring[k]!;
          total++;
          const x = r % w;
          const y = (r - x) / w;
          const sx = x + c.dx;
          const sy = y + c.dy;
          if (sx < 0 || sx >= w || sy < 0 || sy >= h) continue;
          const s = sy * w + sx;
          if (!known[s]) continue;
          err += colourErr(r, s);
          cnt++;
        }
        const e = cnt >= 0.2 * total ? err / cnt : 1e9 - c.cover;
        if (e < bestErr) {
          bestErr = e;
          best = c;
        }
      }
      if (!best) break;
      const left: number[] = [];
      copies++;
      for (const p of remaining) {
        const x = p % w;
        const y = (p - x) / w;
        const sx = x + best.dx;
        const sy = y + best.dy;
        const s = sx < 0 || sx >= w || sy < 0 || sy >= h ? -1 : sy * w + sx;
        if (s < 0 || !known[s]) {
          left.push(p);
          continue;
        }
        rgba[p * 4] = rgba[s * 4]!;
        rgba[p * 4 + 1] = rgba[s * 4 + 1]!;
        rgba[p * 4 + 2] = rgba[s * 4 + 2]!;
        rgba[p * 4 + 3] = 255;
        source[p] = copies;
        unknown--;
      }
      remaining = left;
    }
  }
  featherSeams(rgba, w, h, source);
  return unknown;
}

/** how far (px) on each side of a patch seam the copied pixels are blended */
const SEAM_FEATHER = 3;

/**
 * Soften the seams between copied patches (and between a patch and the
 * original): copied pixels within SEAM_FEATHER of a pixel from a different
 * source take a box-blurred colour instead. Original pixels never change.
 */
function featherSeams(rgba: Buffer, w: number, h: number, source: Int32Array) {
  const n = w * h;
  const seam = new Uint8Array(n);
  let any = false;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      if (source[i] === 0 || rgba[i * 4 + 3] === 0) continue;
      const s = source[i];
      if (
        (x > 0 && source[i - 1] !== s) ||
        (x < w - 1 && source[i + 1] !== s) ||
        (y > 0 && source[i - w] !== s) ||
        (y < h - 1 && source[i + w] !== s)
      ) {
        seam[i] = 1;
        any = true;
      }
    }
  }
  if (!any) return;
  // dilate the seam set by SEAM_FEATHER (over copied pixels only)
  let band = seam;
  for (let k = 0; k < SEAM_FEATHER; k++) {
    const next = new Uint8Array(band);
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const i = y * w + x;
        if (band[i] || source[i] === 0 || rgba[i * 4 + 3] === 0) continue;
        if (
          (x > 0 && band[i - 1]) ||
          (x < w - 1 && band[i + 1]) ||
          (y > 0 && band[i - w]) ||
          (y < h - 1 && band[i + w])
        )
          next[i] = 1;
      }
    }
    band = next;
  }
  const r = SEAM_FEATHER;
  const out = Buffer.from(rgba);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = y * w + x;
      if (!band[i]) continue;
      let sr = 0,
        sg = 0,
        sb = 0,
        cnt = 0;
      for (let dy = -r; dy <= r; dy++) {
        const yy = y + dy;
        if (yy < 0 || yy >= h) continue;
        for (let dx = -r; dx <= r; dx++) {
          const xx = x + dx;
          if (xx < 0 || xx >= w) continue;
          const j = (yy * w + xx) * 4;
          if (rgba[j + 3] === 0) continue;
          sr += rgba[j]!;
          sg += rgba[j + 1]!;
          sb += rgba[j + 2]!;
          cnt++;
        }
      }
      if (cnt === 0) continue;
      out[i * 4] = Math.round(sr / cnt);
      out[i * 4 + 1] = Math.round(sg / cnt);
      out[i * 4 + 2] = Math.round(sb / cnt);
    }
  }
  out.copy(rgba);
}

/** fill a tile's unknown pixels according to the mode; returns how many stayed unknown */
function fillTile(
  rgba: Buffer,
  w: number,
  h: number,
  fill: Fill,
  groundLike: boolean,
): number {
  if (fill === "none") {
    let unknown = 0;
    for (let i = 0; i < w * h; i++) if (rgba[i * 4 + 3] === 0) unknown++;
    return unknown;
  }
  if (fill === "smear") return smearFill(rgba, w, h);
  let left = groundLike ? patchFill(rgba, w, h) : reflectFill(rgba, w, h, true);
  // what no patch reaches (mostly tile corners outside the face) is reflected, then smeared
  if (left > 0 && groundLike) left = reflectFill(rgba, w, h, true, 64);
  return left > 0 ? smearFill(rgba, w, h) : 0;
}

/** the EmbarkStudios texture-synthesis CLI (cargo install --locked texture-synthesis-cli) */
export interface TextureOptions {
  synthBinary: string;
  synthJobs: number;
  debug: boolean;
  dump?: string;
  idAtlas: boolean;
  workDirectory: string;
}
export function synthesisOptions(
  options: Partial<TextureOptions> = {},
): TextureOptions {
  const result = {
    synthBinary: path.join(os.homedir(), ".cargo", "bin", "texture-synthesis"),
    synthJobs: Math.max(1, Math.min(12, os.cpus().length - 2)),
    debug: false,
    idAtlas: false,
    workDirectory: workDir,
    ...options,
  };
  if (!Number.isSafeInteger(result.synthJobs) || result.synthJobs < 1)
    throw new Error("synthJobs must be a positive integer");
  return result;
}
/** tiles thinner than this fail in the synthesizer and take the procedural fill instead */
const SYNTH_MIN_SIDE = 16;
/** parallel synthesizer processes */

const execFileP = promisify(execFile);

/**
 * Fill alpha-0 pixels of an RGBA tile with the texture-synthesis CLI
 * (Harrison/Wei-Levoy style example-based inpainting from the tile's own
 * pixels). Returns false if the tool failed (caller falls back).
 */
async function synthInpaint(
  rgba: Buffer,
  w: number,
  h: number,
  dir: string,
  name: string,
  binary: string,
): Promise<boolean> {
  const n = w * h;
  const rgb = Buffer.alloc(n * 3);
  const mask = Buffer.alloc(n);
  let unknown = 0;
  for (let i = 0; i < n; i++) {
    rgb[i * 3] = rgba[i * 4]!;
    rgb[i * 3 + 1] = rgba[i * 4 + 1]!;
    rgb[i * 3 + 2] = rgba[i * 4 + 2]!;
    if (rgba[i * 4 + 3]! > 0) mask[i] = 255;
    else unknown++;
  }
  if (unknown === 0) return true;
  const ex = path.join(dir, `${name}.png`);
  const mk = path.join(dir, `${name}-mask.png`);
  const out = path.join(dir, `${name}-out.png`);
  await sharp(rgb, { raw: { width: w, height: h, channels: 3 } })
    .png()
    .toFile(ex);
  await sharp(mask, { raw: { width: w, height: h, channels: 1 } })
    .png()
    .toFile(mk);
  try {
    await execFileP(
      binary,
      [
        "--out",
        out,
        "--out-size",
        `${w}x${h}`,
        "--inpaint",
        mk,
        "generate",
        ex,
      ],
      {
        maxBuffer: 1 << 24,
      },
    );
    const res = await sharp(out)
      .removeAlpha()
      .raw()
      .toBuffer({ resolveWithObject: true });
    if (res.info.width !== w || res.info.height !== h)
      throw new Error("synthesizer output dimensions do not match input");
    for (let i = 0; i < n; i++) {
      if (rgba[i * 4 + 3]! > 0) continue;
      rgba[i * 4] = res.data[i * 3]!;
      rgba[i * 4 + 1] = res.data[i * 3 + 1]!;
      rgba[i * 4 + 2] = res.data[i * 3 + 2]!;
      rgba[i * 4 + 3] = 255;
    }
    return true;
  } catch (e) {
    console.warn(
      `texture-synthesis failed on ${name} (${w}x${h}): ${(e as Error).message.split("\n")[0]}`,
    );
    return false;
  } finally {
    await Promise.all([ex, mk, out].map((f) => fs.rm(f, { force: true })));
  }
}

/** run jobs with limited concurrency */
async function pool<T>(
  items: T[],
  limit: number,
  job: (item: T) => Promise<void>,
): Promise<void> {
  let next = 0;
  const workers = Array.from(
    { length: Math.min(limit, items.length) },
    async () => {
      while (next < items.length) await job(items[next++]!);
    },
  );
  await Promise.all(workers);
}

/** 2×3 affine map (x, y) -> (a·x + b·y + c, d·x + e·y + f) */
type Affine = [number, number, number, number, number, number];

/** the affine map sending three points p to three points q (map-pixel coordinates) */
function affineFrom3(
  p: [number, number][],
  q: [number, number][],
): Affine | null {
  const [p0, p1, p2] = p as [
    [number, number],
    [number, number],
    [number, number],
  ];
  const [q0, q1, q2] = q as [
    [number, number],
    [number, number],
    [number, number],
  ];
  const ux = p1[0] - p0[0],
    uy = p1[1] - p0[1],
    vx = p2[0] - p0[0],
    vy = p2[1] - p0[1];
  const det = ux * vy - uy * vx;
  if (Math.abs(det) < 1e-6) return null;
  // solve [a b; d e] · [u v] = [q1-q0, q2-q0]
  const rx = q1[0] - q0[0],
    ry = q1[1] - q0[1],
    sx = q2[0] - q0[0],
    sy = q2[1] - q0[1];
  const a = (rx * vy - sx * uy) / det;
  const b = (sx * ux - rx * vx) / det;
  const d = (ry * vy - sy * uy) / det;
  const e = (sy * ux - ry * vx) / det;
  return [
    a,
    b,
    q0[0] - a * p0[0] - b * p0[1],
    d,
    e,
    q0[1] - d * p0[0] - e * p0[1],
  ];
}

/**
 * A face's tile frame <-> its local 2D frame in scene units. Walls: u along
 * the base from b0, v = height above the base. Roof parts: u, v along the
 * plane's e1/e2 (u along the ridge, v the distance down from it), so the two
 * slopes of a gable and any two walls line up at the same scale.
 */
interface Frame {
  toLocal: Affine;
  fromLocal: Affine;
  /** local bbox of the face */
  u0: number;
  v0: number;
  u1: number;
  v1: number;
  /** largest axis-aligned rectangle inside the face's polygon (local): the area a donor is repeated from */
  ru0: number;
  rv0: number;
  ru1: number;
  rv1: number;
}

/**
 * Copy the donor face onto the unknown pixels of dst at the donor's own
 * scale: dst pixel -> local (u, v) -> the same distances from the donor's
 * local origin (u reversed for a mirrored wall), mirror-repeating the donor
 * where the recipient is larger. Returns pixels copied.
 */
function sampleDonorFrame(
  dst: Tile,
  src: Tile,
  rf: Frame,
  df: Frame,
  mirrored: boolean,
): number {
  const dw = df.ru1 - df.ru0;
  const dh = df.rv1 - df.rv0;
  if (dw < 1 || dh < 1) return 0;
  let copied = 0;
  for (let y = 0; y < dst.h; y++) {
    for (let x = 0; x < dst.w; x++) {
      const k = y * dst.w + x;
      const i = k * 4;
      if (dst.rgba[i + 3]! > 0) continue;
      const tx = x + dst.x0 + 0.5;
      const ty = y + dst.y0 + 0.5;
      let u = rf.toLocal[0] * tx + rf.toLocal[1] * ty + rf.toLocal[2] - rf.ru0;
      const v =
        rf.toLocal[3] * tx + rf.toLocal[4] * ty + rf.toLocal[5] - rf.rv0;
      if (mirrored) u = dw - u;
      const su = df.ru0 + mirrorMod(u, dw);
      const sv = df.rv0 + mirrorMod(v, dh);
      const sx = Math.floor(
        df.fromLocal[0] * su + df.fromLocal[1] * sv + df.fromLocal[2] - src.x0,
      );
      const sy = Math.floor(
        df.fromLocal[3] * su + df.fromLocal[4] * sv + df.fromLocal[5] - src.y0,
      );
      if (sx < 0 || sy < 0 || sx >= src.w || sy >= src.h) continue;
      const sk = sy * src.w + sx;
      if (src.valid && !src.valid[sk]) continue;
      const j = sk * 4;
      if (src.rgba[j + 3] === 0) continue;
      dst.rgba[i] = src.rgba[j]!;
      dst.rgba[i + 1] = src.rgba[j + 1]!;
      dst.rgba[i + 2] = src.rgba[j + 2]!;
      dst.rgba[i + 3] = 255;
      if (dst.valid) dst.valid[k] = 1;
      copied++;
    }
  }
  return copied;
}

/**
 * Largest axis-aligned rectangle inside a polygon, on a unit grid over its
 * bbox (triangles given in the same 2D frame). Returns [u0, v0, u1, v1].
 */
function inscribedRect(tris: [number, number][][], box: Box): Box {
  const [u0, v0, u1, v1] = box;
  const W = Math.min(1024, Math.max(1, Math.ceil(u1 - u0)));
  const H = Math.min(1024, Math.max(1, Math.ceil(v1 - v0)));
  const su = (u1 - u0) / W;
  const sv = (v1 - v0) / H;
  const inside = new Uint8Array(W * H);
  for (const [a, b, c] of tris as [
    [number, number],
    [number, number],
    [number, number],
  ][]) {
    const area = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if (Math.abs(area) < 1e-9) continue;
    const inv = 1 / area;
    const cx0 = Math.max(0, Math.floor((Math.min(a[0], b[0], c[0]) - u0) / su));
    const cx1 = Math.min(
      W - 1,
      Math.ceil((Math.max(a[0], b[0], c[0]) - u0) / su),
    );
    const cy0 = Math.max(0, Math.floor((Math.min(a[1], b[1], c[1]) - v0) / sv));
    const cy1 = Math.min(
      H - 1,
      Math.ceil((Math.max(a[1], b[1], c[1]) - v0) / sv),
    );
    for (let cy = cy0; cy <= cy1; cy++) {
      const py = v0 + (cy + 0.5) * sv;
      for (let cx = cx0; cx <= cx1; cx++) {
        const px = u0 + (cx + 0.5) * su;
        const l0 =
          ((b[0] - px) * (c[1] - py) - (c[0] - px) * (b[1] - py)) * inv;
        const l1 =
          ((c[0] - px) * (a[1] - py) - (a[0] - px) * (c[1] - py)) * inv;
        if (l0 >= -1e-6 && l1 >= -1e-6 && 1 - l0 - l1 >= -1e-6)
          inside[cy * W + cx] = 1;
      }
    }
  }
  // maximal rectangle in a binary matrix (histogram per row + stack)
  const heights = new Int32Array(W);
  let best = 0;
  let rect: Box = [u0, v0, u0, v0];
  const stack: number[] = [];
  for (let cy = 0; cy < H; cy++) {
    for (let cx = 0; cx < W; cx++)
      heights[cx] = inside[cy * W + cx] ? heights[cx]! + 1 : 0;
    stack.length = 0;
    for (let cx = 0; cx <= W; cx++) {
      const hgt = cx < W ? heights[cx]! : 0;
      while (stack.length > 0 && heights[stack[stack.length - 1]!]! >= hgt) {
        const top = stack.pop()!;
        const hh = heights[top]!;
        const left = stack.length > 0 ? stack[stack.length - 1]! + 1 : 0;
        const area = hh * (cx - left);
        if (area > best) {
          best = area;
          rect = [
            u0 + left * su,
            v0 + (cy + 1 - hh) * sv,
            u0 + cx * su,
            v0 + (cy + 1) * sv,
          ];
        }
      }
      stack.push(cx);
    }
  }
  return rect;
}

export interface Textured {
  /** atlas UV per vertex */
  uvs: Float32Array<ArrayBuffer>;
  atlas: { width: number; height: number; rgba: Buffer };
  /** the atlas with every tile painted in its fill category colour (FILL_COLOURS) */
  debugAtlas: { width: number; height: number; rgba: Buffer };
  ground: { width: number; height: number; rgba: Buffer };
  stats: {
    faces: number;
    own: number;
    /** partly visible faces completed from a donor face before mirroring */
    donorFilled: number;
    borrowedWalls: number;
    borrowedRoofs: number;
    flat: number;
    /** tile pixels without own data (before filling) */
    unknownPx: number;
    /** tile pixels still unknown after filling */
    leftPx: number;
    atlasSide: number;
  };
}

function boxOverlap(a: Box, b: Box): number {
  const w = Math.min(a[2], b[2]) - Math.max(a[0], b[0]);
  const h = Math.min(a[3], b[3]) - Math.max(a[1], b[1]);
  return w > 0 && h > 0 ? w * h : 0;
}

/** fill categories for the --debug-fill render: own pixels reflected, reflected + donor, reflected +
 * no donor (repeats), (unused), hidden (donor repeated), ground-like patch fill, flat, texture-synthesis */
const FILL_COLOURS: [number, number, number][] = [
  [40, 200, 40],
  [230, 200, 30],
  [230, 60, 60],
  [60, 90, 230],
  [220, 60, 220],
  [40, 200, 200],
  [255, 255, 255],
  [255, 140, 0],
];
/** donors must have at least this share of own pixels */
const MIN_DONOR_FRAC = 0.3;
/** and an inscribed rectangle at least this big (scene units) to repeat from */
const MIN_DONOR_RECT = 4;
/** how far (map px) a hidden face may look for a donor on another obstacle */
const DONOR_RADIUS = 600;
/** texel density of synthesized (hidden-face) tiles relative to the map */
const HIDDEN_SCALE = 0.5;
/** faces whose projection covers less than this share of their true area are edge-on: local tile */
const EDGE_ON = 0.35;
/**
 * below this share the map holds so few pixels of the face that sampling
 * them through the projection only stretches a sliver into lines: the face
 * is treated as hidden and textured from a donor instead
 */
const EDGE_ON_HIDDEN = 0.2;

export async function buildTextures(
  g: Geometry,
  own: Owners,
  cam: MapCamera,
  mapRgb: Buffer,
  mapW: number,
  mapH: number,
  fill: Fill,
  synthDir: string,
  options: TextureOptions = synthesisOptions(),
): Promise<Textured> {
  const { faces } = g;
  const tiles: (Tile | null)[] = faces.map(() => null);
  /** projected bbox of every face (also for faces without a tile) */
  const faceBox: (Box | null)[] = faces.map(() => null);
  /** own pixels / projected area */
  const frac = new Float32Array(faces.length);
  const pad = 1;
  let unknownPx = 0;
  let leftPx = 0;
  const isGroundLike = (f: number) =>
    faces[f]!.kind === "top" && g.terraceIds.has(faces[f]!.obstacle);
  const P = (v: number): [number, number, number] => [
    g.positions[v * 3]!,
    g.positions[v * 3 + 1]!,
    g.positions[v * 3 + 2]!,
  ];
  const centre = (f: number): [number, number] => {
    const b = faceBox[f]!;
    return [(b[0] + b[2]) / 2, (b[1] + b[3]) / 2];
  };
  const extent = (f: number) => {
    const b = faceBox[f]!;
    return Math.max(b[2] - b[0], b[3] - b[1]) / 2;
  };
  const dist = (a: number, b: number) => {
    const ca = centre(a);
    const cb = centre(b);
    return Math.hypot(ca[0] - cb[0], ca[1] - cb[1]);
  };

  // ── local frames ──
  /** local (u, v) of vertex v of face f, scene units */
  const localCoords = (f: number, v: number): [number, number] => {
    const { origin, e1, e2 } = faces[f]!.plane;
    const p = P(v);
    const d = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]];
    return [
      d[0]! * e1[0] + d[1]! * e1[1] + d[2]! * e1[2],
      d[0]! * e2[0] + d[1]! * e2[1] + d[2]! * e2[2],
    ];
  };
  /** scene point of local (u, v) on face f */
  const localToScene = (
    f: number,
    u: number,
    v: number,
  ): [number, number, number] => {
    const { origin, e1, e2 } = faces[f]!.plane;
    return [
      origin[0] + u * e1[0] + v * e2[0],
      origin[1] + u * e1[1] + v * e2[1],
      origin[2] + u * e1[2] + v * e2[2],
    ];
  };
  const localBox = (f: number): Box => {
    let u0 = Infinity,
      v0 = Infinity,
      u1 = -Infinity,
      v1 = -Infinity;
    for (const v of faces[f]!.verts) {
      const [u, vv] = localCoords(f, v);
      u0 = Math.min(u0, u);
      u1 = Math.max(u1, u);
      v0 = Math.min(v0, vv);
      v1 = Math.max(v1, vv);
    }
    return [u0, v0, u1, v1];
  };
  /** coordinates of vertex v of face f in f's tile frame (map px, or the local frame scaled) */
  const tileCoords = (f: number, v: number): [number, number] => {
    const t = tiles[f];
    if (!t?.local) return [own.px[v]!, own.py[v]!];
    const [u, vv] = localCoords(f, v);
    return [
      (u - t.local.u0) * t.local.scale,
      (t.local.v1 - vv) * t.local.scale,
    ];
  };
  /** tile frame <-> local frame of a face with a tile, through three spread vertices (memoized) */
  const frameCache = new Map<number, Frame | null>();
  const frame = (f: number): Frame | null => {
    const cached = frameCache.get(f);
    if (cached !== undefined) return cached;
    const fr = computeFrame(f);
    frameCache.set(f, fr);
    return fr;
  };
  const computeFrame = (f: number): Frame | null => {
    const verts = faces[f]!.verts;
    if (verts.length < 3) return null;
    const loc = verts.map((v) => localCoords(f, v));
    const a = 0;
    let b = 1;
    let bestD = -1;
    for (let i = 1; i < verts.length; i++) {
      const d = Math.hypot(loc[i]![0] - loc[a]![0], loc[i]![1] - loc[a]![1]);
      if (d > bestD) {
        bestD = d;
        b = i;
      }
    }
    let c = -1;
    let bestA = 1e-6;
    for (let i = 1; i < verts.length; i++) {
      const ar = Math.abs(
        (loc[b]![0] - loc[a]![0]) * (loc[i]![1] - loc[a]![1]) -
          (loc[i]![0] - loc[a]![0]) * (loc[b]![1] - loc[a]![1]),
      );
      if (ar > bestA) {
        bestA = ar;
        c = i;
      }
    }
    if (c < 0) return null;
    const ids = [verts[a]!, verts[b]!, verts[c]!];
    const tile = ids.map((v) => tileCoords(f, v));
    const local = [loc[a]!, loc[b]!, loc[c]!];
    const toLocal = affineFrom3(tile, local);
    const fromLocal = affineFrom3(local, tile);
    if (!toLocal || !fromLocal) return null;
    const box = localBox(f);
    const tris = faces[f]!.tris.map((t) =>
      [g.tris[t * 3]!, g.tris[t * 3 + 1]!, g.tris[t * 3 + 2]!].map((v) =>
        localCoords(f, v),
      ),
    );
    const [ru0, rv0, ru1, rv1] = inscribedRect(tris, box);
    return {
      toLocal,
      fromLocal,
      u0: box[0],
      v0: box[1],
      u1: box[2],
      v1: box[3],
      ru0,
      rv0,
      ru1,
      rv1,
    };
  };
  /** mask of the tile's pixels inside the face's polygon */
  const insideMask = (f: number, t: Tile): Uint8Array => {
    const m = new Uint8Array(t.w * t.h);
    for (const tri of faces[f]!.tris) {
      const [a, b, c] = [
        g.tris[tri * 3]!,
        g.tris[tri * 3 + 1]!,
        g.tris[tri * 3 + 2]!,
      ].map((v) => {
        const [x, y] = tileCoords(f, v);
        return [x - t.x0, y - t.y0] as [number, number];
      }) as [[number, number], [number, number], [number, number]];
      const area =
        (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
      if (Math.abs(area) < 1e-9) continue;
      const inv = 1 / area;
      const x0 = Math.max(0, Math.floor(Math.min(a[0], b[0], c[0])));
      const x1 = Math.min(t.w - 1, Math.ceil(Math.max(a[0], b[0], c[0])));
      const y0 = Math.max(0, Math.floor(Math.min(a[1], b[1], c[1])));
      const y1 = Math.min(t.h - 1, Math.ceil(Math.max(a[1], b[1], c[1])));
      for (let y = y0; y <= y1; y++) {
        const py = y + 0.5;
        for (let x = x0; x <= x1; x++) {
          const px = x + 0.5;
          const l0 =
            ((b[0] - px) * (c[1] - py) - (c[0] - px) * (b[1] - py)) * inv;
          const l1 =
            ((c[0] - px) * (a[1] - py) - (a[0] - px) * (c[1] - py)) * inv;
          // a little slack so edge pixels count as inside
          if (l0 >= -0.05 && l1 >= -0.05 && 1 - l0 - l1 >= -0.05)
            m[y * t.w + x] = 1;
        }
      }
    }
    return m;
  };
  /** own pixels become the tile's valid sources; frac = own / inside */
  const finishOwnTile = (f: number, t: Tile) => {
    tiles[f] = t; // tileCoords reads tiles[f].local
    const valid = new Uint8Array(t.w * t.h);
    for (let k = 0; k < t.w * t.h; k++)
      if (t.rgba[k * 4 + 3]! > 0) valid[k] = 1;
    t.valid = valid;
    t.inside = insideMask(f, t);
    let insideCount = 0;
    for (let k = 0; k < t.w * t.h; k++) if (t.inside[k]) insideCount++;
    frac[f] = Math.min(1, t.known / Math.max(1, insideCount));
    unknownPx += insideCount - Math.min(insideCount, t.known);
  };
  /** a tile in the face's local frame at the given texel density, with its own pixels if sample */
  const makeLocalTile = (
    f: number,
    scale: number,
    sample: boolean,
  ): Tile | null => {
    frameCache.delete(f);
    const [u0, v0, u1, v1] = localBox(f);
    if (u1 - u0 < 1 || v1 - v0 < 1) return null;
    const w = Math.ceil((u1 - u0) * scale) + 1;
    const h = Math.ceil((v1 - v0) * scale) + 1;
    const rgba = Buffer.alloc(w * h * 4);
    let known = 0;
    if (sample) {
      for (let y = 0; y < h; y++) {
        const v = v1 - (y + 0.5) / scale;
        for (let x = 0; x < w; x++) {
          const u = u0 + (x + 0.5) / scale;
          const [mx, my] = sceneToMap(cam, localToScene(f, u, v));
          const ix = Math.floor(mx);
          const iy = Math.floor(my);
          if (ix < 0 || iy < 0 || ix >= mapW || iy >= mapH) continue;
          const mi = iy * mapW + ix;
          const ti = (y * w + x) * 4;
          rgba[ti] = mapRgb[mi * 3]!;
          rgba[ti + 1] = mapRgb[mi * 3 + 1]!;
          rgba[ti + 2] = mapRgb[mi * 3 + 2]!;
          if (own.owner[mi] === f) {
            rgba[ti + 3] = 255;
            known++;
          }
        }
      }
    }
    return {
      face: f,
      x0: 0,
      y0: 0,
      w,
      h,
      known,
      local: { u0, v0, u1, v1, scale },
      valid: null,
      inside: null,
      rgba,
      ax: 0,
      ay: 0,
    };
  };

  // per-obstacle mean colour of owned pixels, for faces with nothing to borrow
  const meanAcc = new Map<number, [number, number, number, number]>();
  for (let i = 0; i < mapW * mapH; i++) {
    const f = own.owner[i]!;
    if (f < 0) continue;
    const o = faces[f]!.obstacle;
    const m = meanAcc.get(o) ?? [0, 0, 0, 0];
    m[0] += mapRgb[i * 3]!;
    m[1] += mapRgb[i * 3 + 1]!;
    m[2] += mapRgb[i * 3 + 2]!;
    m[3]++;
    meanAcc.set(o, m);
  }

  // faces with own pixels: tile = projected bbox, known = owned pixels;
  // faces seen edge-on (projection much smaller than their true area) get
  // a tile in their own frame instead so they keep resolution
  for (let f = 0; f < faces.length; f++) {
    const face = faces[f]!;
    let x0 = Infinity,
      y0 = Infinity,
      x1 = -Infinity,
      y1 = -Infinity;
    for (const v of face.verts) {
      x0 = Math.min(x0, own.px[v]!);
      x1 = Math.max(x1, own.px[v]!);
      y0 = Math.min(y0, own.py[v]!);
      y1 = Math.max(y1, own.py[v]!);
    }
    faceBox[f] = [x0, y0, x1, y1];
    let projArea = 0;
    let trueArea = 0;
    for (const t of face.tris) {
      const ia = g.tris[t * 3]!,
        ib = g.tris[t * 3 + 1]!,
        ic = g.tris[t * 3 + 2]!;
      projArea +=
        Math.abs(
          (own.px[ib]! - own.px[ia]!) * (own.py[ic]! - own.py[ia]!) -
            (own.px[ic]! - own.px[ia]!) * (own.py[ib]! - own.py[ia]!),
        ) / 2;
      const a = P(ia),
        b = P(ib),
        c = P(ic);
      const n = cross(
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
      );
      trueArea += Math.hypot(n[0]!, n[1]!, n[2]!) / 2;
    }
    if (trueArea >= 1 && projArea / trueArea < EDGE_ON) {
      if (projArea / trueArea < EDGE_ON_HIDDEN) continue; // hidden: synthesized below
      const t = makeLocalTile(f, 1, true);
      if (!t || t.known === 0) continue;
      finishOwnTile(f, t);
      continue;
    }
    // the tile may reach past the map (faces at the border); such pixels are unknown
    const bx0 = Math.max(-MAP_MARGIN, Math.floor(x0) - pad);
    const by0 = Math.max(-MAP_MARGIN, Math.floor(y0) - pad);
    const bx1 = Math.min(mapW - 1 + MAP_MARGIN, Math.ceil(x1) + pad);
    const by1 = Math.min(mapH - 1 + MAP_MARGIN, Math.ceil(y1) + pad);
    const w = bx1 - bx0 + 1;
    const h = by1 - by0 + 1;
    if (w <= 0 || h <= 0) continue;
    const rgba = Buffer.alloc(w * h * 4);
    let known = 0;
    for (let y = 0; y < h; y++) {
      const my = by0 + y;
      if (my < 0 || my >= mapH) continue;
      for (let x = 0; x < w; x++) {
        const mx = bx0 + x;
        if (mx < 0 || mx >= mapW) continue;
        const mi = my * mapW + mx;
        const ti = (y * w + x) * 4;
        rgba[ti] = mapRgb[mi * 3]!;
        rgba[ti + 1] = mapRgb[mi * 3 + 1]!;
        rgba[ti + 2] = mapRgb[mi * 3 + 2]!;
        if (own.owner[mi] === f) {
          rgba[ti + 3] = 255;
          known++;
        }
      }
    }
    if (known === 0) continue; // hidden: synthesized below
    finishOwnTile(f, {
      face: f,
      x0: bx0,
      y0: by0,
      w,
      h,
      known,
      local: null,
      valid: null,
      inside: null,
      rgba,
      ax: 0,
      ay: 0,
    });
  }
  const ownCount = tiles.filter((t) => t).length;
  /** how each face was textured, for the debug render: see FILL_COLOURS */
  const category = new Uint8Array(faces.length);
  /** tiles whose fill is complete (only those may serve as donors) */
  const filled = new Uint8Array(faces.length);
  const donorOf = new Int32Array(faces.length).fill(-1);
  const byObstacle = new Map<number, number[]>();
  faces.forEach((face, f) => {
    const list = byObstacle.get(face.obstacle) ?? [];
    list.push(f);
    byObstacle.set(face.obstacle, list);
  });
  const roofTiles: number[] = [];
  const wallTiles: number[] = [];
  const isDonor = (d: number) => {
    if (
      !tiles[d] ||
      tiles[d]!.local ||
      frac[d]! < MIN_DONOR_FRAC ||
      filled[d] !== 1
    )
      return false;
    const fr = frame(d);
    return (
      !!fr &&
      fr.ru1 - fr.ru0 >= MIN_DONOR_RECT &&
      fr.rv1 - fr.rv0 >= MIN_DONOR_RECT
    );
  };
  for (let f = 0; f < faces.length; f++) {
    if (!tiles[f] || tiles[f]!.local || frac[f]! < MIN_DONOR_FRAC) continue;
    if (faces[f]!.kind === "side") wallTiles.push(f);
    else if (!isGroundLike(f)) roofTiles.push(f);
  }

  // donor search. Walls: the opposite wall of the same obstacle (mirrored),
  // else its best visible wall, else the nearest visible wall of another
  // obstacle with a similar (or opposite, then mirrored) orientation. Roofs:
  // the other slope of the same roof (across the ridge), else another visible
  // part of the same roof, else the visible roof of an overlapping
  // neighbour, else the nearest visible roof.
  const findDonor = (f: number): { d: number; mirrored: boolean } | null => {
    const face = faces[f]!;
    if (face.kind === "side") {
      let best = -1;
      let bestScore = -Infinity;
      let mirrored = false;
      for (const d of byObstacle.get(face.obstacle) ?? []) {
        const df = faces[d]!;
        if (d === f || df.kind !== "side" || !isDonor(d)) continue;
        const dot = -(face.nx * df.nx + face.ny * df.ny);
        const score = (dot > 0.3 ? 1e9 : 0) + tiles[d]!.known;
        if (score > bestScore) {
          bestScore = score;
          best = d;
          mirrored = dot > 0.3;
        }
      }
      if (best >= 0) return { d: best, mirrored };
      if (!faceBox[f]) return null;
      let bestDist = DONOR_RADIUS + extent(f);
      for (const d of wallTiles) {
        const df = faces[d]!;
        if (df.obstacle === face.obstacle || !isDonor(d)) continue;
        const dot = face.nx * df.nx + face.ny * df.ny;
        if (Math.abs(dot) < 0.5) continue;
        const dd = dist(f, d);
        if (dd < bestDist) {
          bestDist = dd;
          best = d;
          mirrored = dot < 0;
        }
      }
      return best >= 0 ? { d: best, mirrored } : null;
    }
    if (isGroundLike(f) || !faceBox[f]) return null;
    if (face.ridgeMate >= 0 && isDonor(face.ridgeMate)) {
      return { d: face.ridgeMate, mirrored: false };
    }
    let best = -1;
    let bestKnown = 0;
    for (const d of byObstacle.get(face.obstacle) ?? []) {
      if (d === f || faces[d]!.kind !== "top" || !isDonor(d)) continue;
      if (tiles[d]!.known > bestKnown) {
        bestKnown = tiles[d]!.known;
        best = d;
      }
    }
    if (best >= 0) return { d: best, mirrored: false };
    const [x0, y0, x1, y1] = faceBox[f]!;
    const grow = 0.25 * Math.max(x1 - x0, y1 - y0, 4);
    const box: Box = [x0 - grow, y0 - grow, x1 + grow, y1 + grow];
    let bestOverlap = 0;
    for (const d of roofTiles) {
      if (faces[d]!.obstacle === face.obstacle || !isDonor(d)) continue;
      const ov = boxOverlap(box, faceBox[d]!);
      if (ov > bestOverlap) {
        bestOverlap = ov;
        best = d;
      }
    }
    if (best < 0) {
      let bestDist = DONOR_RADIUS + extent(f);
      for (const d of roofTiles) {
        if (faces[d]!.obstacle === face.obstacle || !isDonor(d)) continue;
        const dd = dist(f, d);
        if (dd < bestDist) {
          bestDist = dd;
          best = d;
        }
      }
    }
    return best >= 0 ? { d: best, mirrored: false } : null;
  };

  // fill the tiles, best-covered first: a face with less than half of its
  // projection visible is completed from a donor before its own pixels are
  // mirrored into whatever is still unknown
  let donorFilled = 0;
  const fillOrder = tiles
    .flatMap((t, f) => (t ? [f] : []))
    .sort((a, b) => frac[b]! - frac[a]!);
  const synthDone = new Uint8Array(faces.length);
  if (fill === "synth") {
    // every face with own pixels is inpainted from them by the synthesizer
    const jobs = fillOrder.filter(
      (f) =>
        tiles[f]!.known >= 100 &&
        Math.min(tiles[f]!.w, tiles[f]!.h) >= SYNTH_MIN_SIDE,
    );
    const t0 = Date.now();
    await pool(jobs, options.synthJobs, async (f) => {
      const t = tiles[f]!;
      if (
        await synthInpaint(
          t.rgba,
          t.w,
          t.h,
          synthDir,
          `face-${f}`,
          options.synthBinary,
        )
      ) {
        t.valid!.fill(1);
        synthDone[f] = 1;
      }
    });
    console.log(
      `texture-synthesis: ${jobs.length} tiles in ${((Date.now() - t0) / 1000).toFixed(0)} s`,
    );
  }
  for (const f of fillOrder) {
    const t = tiles[f]!;
    if (synthDone[f]) category[f] = 7;
    else if ((fill === "proc" || fill === "synth") && !isGroundLike(f)) {
      // a band along the visibility boundary is a faithful reflection of
      // what is seen; deeper unknown pixels come from a donor at 1:1 scale
      reflectFill(t.rgba, t.w, t.h, false, 512, t.valid);
      let left = 0;
      for (let k = 0; k < t.w * t.h; k++)
        if (t.inside![k] && t.rgba[k * 4 + 3] === 0) left++;
      let copied = 0;
      if (left > 0) {
        const donor = findDonor(f);
        if (donor) {
          donorOf[f] = donor.d;
          const rf = frame(f);
          const df = frame(donor.d);
          if (rf && df)
            copied = sampleDonorFrame(
              t,
              tiles[donor.d]!,
              rf,
              df,
              donor.mirrored,
            );
        }
      }
      if (copied > 0) donorFilled++;
      category[f] = left === 0 ? 0 : copied > 0 ? 1 : 2;
    } else category[f] = isGroundLike(f) ? 5 : 0;
    filled[f] = 1;
    leftPx += fillTile(t.rgba, t.w, t.h, fill, isGroundLike(f));
  }

  // hidden faces (no own pixels) get a local tile with a donor repeated at
  // its own scale; without a donor: a flat tile in the obstacle's mean
  // colour (or, for an obstacle nobody sees, the mean of the map around
  // it). With --fill none every hidden face stays transparent.
  const borrowFrom = new Int32Array(faces.length).fill(-1);
  let borrowedWalls = 0;
  let borrowedRoofs = 0;
  let flat = 0;
  const flatTiles = new Map<number, number>(); // obstacle -> face id holding the flat tile
  for (let f = 0; f < faces.length; f++) {
    if (tiles[f]) continue;
    const face = faces[f]!;
    if (fill !== "none") {
      const donor = findDonor(f);
      const df = donor ? frame(donor.d) : null;
      const t = donor && df ? makeLocalTile(f, HIDDEN_SCALE, false) : null;
      if (donor && df && t) {
        donorOf[f] = donor.d;
        tiles[f] = t;
        t.valid = new Uint8Array(t.w * t.h);
        const rf = frame(f);
        if (
          rf &&
          sampleDonorFrame(t, tiles[donor.d]!, rf, df, donor.mirrored) > 0
        ) {
          leftPx += fillTile(t.rgba, t.w, t.h, fill, false);
          if (face.kind === "side") borrowedWalls++;
          else borrowedRoofs++;
          category[f] = 4;
          continue;
        }
        tiles[f] = null;
      }
    }
    let flatFace = flatTiles.get(face.obstacle);
    if (flatFace === undefined) {
      let m = meanAcc.get(face.obstacle);
      if (fill !== "none" && (!m || m[3] === 0)) {
        // nobody sees this obstacle: average the map over its projection
        const acc: [number, number, number, number] = [0, 0, 0, 0];
        let bx0 = Infinity,
          by0 = Infinity,
          bx1 = -Infinity,
          by1 = -Infinity;
        for (const o of byObstacle.get(face.obstacle) ?? []) {
          const b = faceBox[o];
          if (!b) continue;
          bx0 = Math.min(bx0, b[0]);
          by0 = Math.min(by0, b[1]);
          bx1 = Math.max(bx1, b[2]);
          by1 = Math.max(by1, b[3]);
        }
        for (
          let y = Math.max(0, Math.floor(by0));
          y <= Math.min(mapH - 1, Math.ceil(by1));
          y++
        ) {
          for (
            let x = Math.max(0, Math.floor(bx0));
            x <= Math.min(mapW - 1, Math.ceil(bx1));
            x++
          ) {
            const i = y * mapW + x;
            acc[0] += mapRgb[i * 3]!;
            acc[1] += mapRgb[i * 3 + 1]!;
            acc[2] += mapRgb[i * 3 + 2]!;
            acc[3]++;
          }
        }
        m = acc;
      }
      const rgba = Buffer.alloc(2 * 2 * 4);
      if (fill !== "none" && m && m[3] > 0) {
        for (let i = 0; i < 4; i++) {
          rgba[i * 4] = Math.round(m[0] / m[3]);
          rgba[i * 4 + 1] = Math.round(m[1] / m[3]);
          rgba[i * 4 + 2] = Math.round(m[2] / m[3]);
          rgba[i * 4 + 3] = 255;
        }
      }
      tiles[f] = {
        face: f,
        x0: 0,
        y0: 0,
        w: 2,
        h: 2,
        known: 0,
        local: null,
        valid: null,
        inside: null,
        rgba,
        ax: 0,
        ay: 0,
      };
      flatTiles.set(face.obstacle, f);
      flatFace = f;
    }
    borrowFrom[f] = flatFace;
    category[f] = 6;
    flat++;
    if (options.debug) {
      const b = localBox(f);
      const d = findDonor(f);
      const df = d ? frame(d.d) : null;
      console.log(
        `flat face ${f} ${face.kind} obstacle ${face.obstacle} local ${(b[2] - b[0]).toFixed(1)}x${(b[3] - b[1]).toFixed(1)} verts ${face.verts.length} donor ${d ? `${d.d} (${faces[d.d]!.kind}, rect ${df ? `${(df.ru1 - df.ru0).toFixed(1)}x${(df.rv1 - df.rv0).toFixed(1)}` : "no frame"})` : "none"} frame ${frame(f) ? "ok" : "null"}`,
      );
    }
  }

  // pack the tiles (shelf packing, tallest first) into a square atlas
  const packed = tiles.filter((t): t is Tile => t !== null);
  let area = 0;
  for (const t of packed) area += (t.w + 1) * (t.h + 1);
  let side = 1024;
  while (side * side < area * 1.25 && side < MAX_ATLAS) side *= 2;
  const order = [...packed].sort((a, b) => b.h - a.h);
  let placed = false;
  for (; side <= MAX_ATLAS; side *= 2) {
    let x = 0,
      y = 0,
      shelf = 0;
    placed = true;
    for (const t of order) {
      if (x + t.w > side) {
        x = 0;
        y += shelf + 1;
        shelf = 0;
      }
      if (y + t.h > side || t.w > side) {
        placed = false;
        break;
      }
      t.ax = x;
      t.ay = y;
      x += t.w + 1;
      shelf = Math.max(shelf, t.h);
    }
    if (placed) break;
  }
  if (!placed)
    throw new Error(
      `texture atlas exceeds ${MAX_ATLAS}²; tiles cover ${area} px`,
    );
  console.log(
    `atlas ${side}²: ${packed.length} tiles covering ${(area / 1e6).toFixed(1)} Mpx`,
  );
  const atlas = Buffer.alloc(side * side * 4);
  for (const t of packed) {
    for (let y = 0; y < t.h; y++) {
      t.rgba.copy(
        atlas,
        ((t.ay + y) * side + t.ax) * 4,
        y * t.w * 4,
        (y + 1) * t.w * 4,
      );
    }
  }

  // UVs: own tile -> the vertex's position in the tile; flat tile -> its centre
  const uvs = new Float32Array((g.positions.length / 3) * 2);
  const uvAt = (t: Tile, x: number, y: number, out: number) => {
    // clamped into the tile: a vertex past the map margin stretches the edge
    const lx = Math.min(t.w - 0.5, Math.max(0.5, x - t.x0));
    const ly = Math.min(t.h - 0.5, Math.max(0.5, y - t.y0));
    uvs[out * 2] = (t.ax + lx) / side;
    uvs[out * 2 + 1] = (t.ay + ly) / side;
  };
  for (let f = 0; f < faces.length; f++) {
    const face = faces[f]!;
    const t = tiles[f];
    if (t && borrowFrom[f]! < 0) {
      for (const v of face.verts) uvAt(t, ...tileCoords(f, v), v);
      continue;
    }
    const st = borrowFrom[f]! >= 0 ? tiles[borrowFrom[f]!] : null;
    if (!st) continue;
    for (const v of face.verts) {
      uvs[v * 2] = (st.ax + 1) / side;
      uvs[v * 2 + 1] = (st.ay + 1) / side;
    }
  }

  // ground: the whole map, known = pixels nobody owns
  const ground = Buffer.alloc(mapW * mapH * 4);
  for (let i = 0; i < mapW * mapH; i++) {
    ground[i * 4] = mapRgb[i * 3]!;
    ground[i * 4 + 1] = mapRgb[i * 3 + 1]!;
    ground[i * 4 + 2] = mapRgb[i * 3 + 2]!;
    ground[i * 4 + 3] = own.owner[i]! < 0 ? 255 : 0;
  }
  if (fill === "synth") {
    const t0 = Date.now();
    await synthInpaint(
      ground,
      mapW,
      mapH,
      synthDir,
      "ground",
      options.synthBinary,
    );
    console.log(
      `texture-synthesis: ground in ${((Date.now() - t0) / 1000).toFixed(0)} s`,
    );
  }
  fillTile(ground, mapW, mapH, fill, true);

  if (options.dump) {
    // VOLUMES_DUMP=dir:f,f,f dumps those faces' tiles and masks
    const [dir, list] = options.dump.split(":");
    const wanted = new Set((list ?? "").split(",").map(Number));
    const outDir = dir || path.join(options.workDirectory, "dump");
    void fs.mkdir(outDir, { recursive: true });
    for (let f = 0; f < faces.length; f++) {
      const b = faceBox[f];
      const t = tiles[f];
      if (!b || !t || !wanted.has(f)) continue;
      const face = faces[f]!;
      const fr = frame(f);
      console.log(
        `face ${f} ${face.kind} obstacle ${face.obstacle} bbox ${b.map((v) => v.toFixed(0)).join(",")} tile ${t.w}x${t.h} ${t.local ? `local scale ${t.local.scale}` : "map"} known ${t.known} frac ${frac[f]!.toFixed(2)} category ${category[f]} donor ${donorOf[f]} ridgeMate ${face.ridgeMate} rect ${fr ? [fr.ru0, fr.rv0, fr.ru1, fr.rv1].map((v) => v.toFixed(0)).join(",") : "-"} box ${fr ? [fr.u0, fr.v0, fr.u1, fr.v1].map((v) => v.toFixed(0)).join(",") : "-"}`,
      );
      const masks = Buffer.alloc(t.w * t.h * 4);
      for (let k = 0; k < t.w * t.h; k++) {
        masks[k * 4] = t.valid?.[k] ? 255 : 0;
        masks[k * 4 + 1] = t.inside?.[k] ? 255 : 0;
        masks[k * 4 + 2] = t.rgba[k * 4 + 3]!;
        masks[k * 4 + 3] = 255;
      }
      void sharp(t.rgba, { raw: { width: t.w, height: t.h, channels: 4 } })
        .png()
        .toFile(path.join(outDir, `face-${f}.png`));
      void sharp(masks, { raw: { width: t.w, height: t.h, channels: 4 } })
        .png()
        .toFile(path.join(outDir, `face-${f}-masks.png`));
    }
  }

  const debugAtlas = Buffer.alloc(side * side * 4);
  for (let f = 0; f < faces.length; f++) {
    const t = tiles[f];
    if (!t) continue;
    const c: [number, number, number] = options.idAtlas
      ? [(f >> 16) & 255, (f >> 8) & 255, f & 255]
      : FILL_COLOURS[category[f]!]!;
    for (let y = 0; y < t.h; y++) {
      for (let x = 0; x < t.w; x++) {
        const i = ((t.ay + y) * side + t.ax + x) * 4;
        debugAtlas[i] = c[0];
        debugAtlas[i + 1] = c[1];
        debugAtlas[i + 2] = c[2];
        debugAtlas[i + 3] = 255;
      }
    }
  }

  return {
    uvs,
    atlas: { width: side, height: side, rgba: atlas },
    debugAtlas: { width: side, height: side, rgba: debugAtlas },
    ground: { width: mapW, height: mapH, rgba: ground },
    stats: {
      faces: faces.length,
      own: ownCount,
      donorFilled,
      borrowedWalls,
      borrowedRoofs,
      flat,
      unknownPx,
      leftPx,
      atlasSide: side,
    },
  };
}
