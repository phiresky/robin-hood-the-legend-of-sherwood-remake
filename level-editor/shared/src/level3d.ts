// The 3D editor's level document: the game's own obstacle representation
// per part (footprint polygon with absolute z_bottom/z_top per point), an
// editor transform on top of the reconstruction it came from, and the
// parts grouped into buildings. The reconstruction GLB (volumes.ts, one
// node per obstacle) supplies geometry and textures; the document says
// where each part now is and which ones exist. Baking
// (pipeline/src/bake.ts) turns it back into the files the game reads.
//
// A building in the game data is a stack of obstacles: an opaque box to
// the eave, non-opaque boxes for jettied floors and roof slopes, chimneys,
// door posts, furniture inside. They are grouped here by footprint overlap
// and height contact, so the editor selects the whole building by default
// while every part keeps its own transform relative to the group.
import polygonClipping, { type Polygon } from "polygon-clipping";
import type { MapCamera } from "./scene.ts";
import type { ObstaclePoint, SightObstacle } from "./level.ts";
import { signedPolygonArea } from "./geometry.ts";

/** rigid transform in game coordinates: translate (map px, absolute z) and turn about the vertical axis */
export interface GameTransform {
  dx: number;
  dy: number;
  dz: number;
  /** rotation about the pivot (the footprint centroid), degrees, counter-clockwise in map coordinates */
  rot_deg: number;
}

export const IDENTITY_TRANSFORM: GameTransform = { dx: 0, dy: 0, dz: 0, rot_deg: 0 };

export function isIdentity(t: GameTransform): boolean {
  return t.dx === 0 && t.dy === 0 && t.dz === 0 && t.rot_deg === 0;
}

/** one obstacle of the level: a part of a building, or a terrace */
export interface Level3DObject {
  /** unique in the document; the reconstruction node name for original parts ("building-042") */
  id: string;
  kind: "building" | "terrace";
  /** the reconstruction node this part draws with (its own for originals, the original's for duplicates) */
  node: string;
  /** map + obstacle index the geometry and the game data came from */
  source: { map: string; obstacle: number };
  /** the obstacle as the game sees it, before any transform */
  obstacle: SightObstacle;
  /** transform relative to the group (or the world for ungrouped parts) */
  transform: GameTransform;
  /** the building this part belongs to */
  group?: string;
  hidden?: boolean;
  name?: string;
}

/** a building: several parts moved together */
export interface Level3DGroup {
  id: string;
  name?: string;
  transform: GameTransform;
  hidden?: boolean;
}

export interface Level3D {
  provenance?: { source_sha256?: string; glb_sha256?: string };
  version: 1;
  map: string;
  /** map size in pixels */
  size: [number, number];
  camera: MapCamera;
  /** reconstruction GLB (relative to the document) */
  glb: string;
  objects: Level3DObject[];
  groups: Level3DGroup[];
  notes?: string;
}

// ── grouping ─────────────────────────────────────────────────────────

/** parts overlap in plan by at least this share of the smaller footprint to belong together */
const GROUP_OVERLAP = 0.4;
/** and their height ranges touch within this (game units) */
const GROUP_Z_GAP = 8;

function footprintArea(points: ObstaclePoint[]): number {
  return Math.abs(signedPolygonArea(points.map((p) => [p.x, p.y])));
}

function toPolygon(points: ObstaclePoint[]): Polygon {
  const ring = points.map((p) => [p.x, p.y] as [number, number]);
  ring.push(ring[0]!);
  return [ring];
}

function polysArea(m: ReturnType<typeof polygonClipping.intersection>): number {
  let a = 0;
  for (const poly of m) {
    for (const [k, ring] of poly.entries()) {
      let r = 0;
      for (let i = 0; i < ring.length - 1; i++) r += ring[i]![0] * ring[i + 1]![1] - ring[i + 1]![0] * ring[i]![1];
      a += (k === 0 ? 1 : -1) * Math.abs(r) / 2;
    }
  }
  return a;
}

/**
 * Group obstacle indices into buildings: union-find over pairs whose
 * footprints overlap by GROUP_OVERLAP of the smaller one and whose height
 * ranges touch. Terraces (and anything in `exclude`) stay alone. Returns
 * group id (smallest member index) per obstacle index; singletons are
 * their own group.
 */
export function groupObstacles(obstacles: SightObstacle[], exclude: Set<number>): Map<number, number> {
  const parent = new Map<number, number>();
  const find = (i: number): number => {
    let r = i;
    while (parent.get(r) !== r) r = parent.get(r)!;
    let x = i;
    while (parent.get(x) !== r) {
      const next = parent.get(x)!;
      parent.set(x, r);
      x = next;
    }
    return r;
  };
  const union = (a: number, b: number) => {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) parent.set(Math.max(ra, rb), Math.min(ra, rb));
  };
  interface Item {
    i: number;
    box: [number, number, number, number];
    area: number;
    z0: number;
    z1: number;
    poly: Polygon;
  }
  const items: Item[] = [];
  obstacles.forEach((o, i) => {
    if (exclude.has(i) || o.points.length < 3) return;
    parent.set(i, i);
    const xs = o.points.map((p) => p.x);
    const ys = o.points.map((p) => p.y);
    items.push({
      i,
      box: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)],
      area: footprintArea(o.points),
      z0: Math.min(...o.points.map((p) => p.z_bottom)),
      z1: Math.max(...o.points.map((p) => p.z_top)),
      poly: toPolygon(o.points),
    });
  });
  for (let a = 0; a < items.length; a++) {
    const A = items[a]!;
    for (let b = a + 1; b < items.length; b++) {
      const B = items[b]!;
      if (A.box[2] < B.box[0] || B.box[2] < A.box[0] || A.box[3] < B.box[1] || B.box[3] < A.box[1]) continue;
      if (A.z0 > B.z1 + GROUP_Z_GAP || B.z0 > A.z1 + GROUP_Z_GAP) continue;
      const smaller = Math.min(A.area, B.area);
      if (smaller < 1) continue;
      let overlap = 0;
      try {
        overlap = polysArea(polygonClipping.intersection(A.poly, B.poly));
      } catch {
        continue;
      }
      if (overlap >= GROUP_OVERLAP * smaller) union(A.i, B.i);
    }
  }
  const out = new Map<number, number>();
  for (const i of parent.keys()) out.set(i, find(i));
  return out;
}

// ── floating parts ───────────────────────────────────────────────────

/** a raised part counts as supported where it is when a support under its footprint covers this share of it */
const SNAP_SUPPORTED = 0.6;
/** after the shift the support must cover this share of the part's footprint */
const SNAP_OVERLAP = 0.8;
/** raised parts closer than this to a support's top are left alone */
const SNAP_TOLERANCE = 3;
/** shifts below this are authored detail (awnings on posts, ledges), not displacement */
const SNAP_MIN = 10;
/** parts must not be shifted further than this (game units) */
const SNAP_MAX = 200;

/**
 * Snap floating parts back onto what they sit on. Some non-opaque pieces
 * (roof slopes, jetties) are stored displaced along the view ray: a point
 * moved by (y - Δ, z - Δ) projects to the same map pixel, so the map looks
 * right, no sight is affected, and in 3D the piece floats Δ above and Δ
 * south of its body. For every raised part without a support under its
 * footprint, look for an obstacle whose top it lands on when shifted by
 * Δ = z_bottom - top, and apply that shift to y and z (the projection does
 * not change). Only non-opaque parts are moved by default: they block no
 * sight, so the game cannot tell; opaque candidates (spires, chimneys, but
 * also legitimately raised things like bridges or tree crowns) are only
 * reported as suspects unless `includeOpaque` is set. Returns the
 * corrected obstacles, what was snapped and the suspects left alone.
 */
export function snapFloatingParts(
  obstacles: SightObstacle[],
  exclude: Set<number>,
  opts: { includeOpaque?: boolean } = {},
): { obstacles: SightObstacle[]; snapped: { index: number; support: number; delta: number }[]; suspects: { index: number; support: number; delta: number }[] } {
  interface Item {
    i: number;
    box: [number, number, number, number];
    area: number;
    zb: number;
    zt: number;
    poly: Polygon;
  }
  const items: Item[] = [];
  obstacles.forEach((o, i) => {
    if (o.points.length < 3) return;
    const xs = o.points.map((p) => p.x);
    const ys = o.points.map((p) => p.y);
    items.push({
      i,
      box: [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)],
      area: footprintArea(o.points),
      zb: Math.min(...o.points.map((p) => p.z_bottom)),
      zt: Math.max(...o.points.map((p) => p.z_top)),
      poly: toPolygon(o.points),
    });
  });
  const overlap = (a: Polygon, b: Polygon): number => {
    try {
      return polysArea(polygonClipping.intersection(a, b));
    } catch {
      return 0;
    }
  };
  const shiftedPoly = (o: SightObstacle, d: number): Polygon => {
    const ring = o.points.map((p) => [p.x, p.y - d] as [number, number]);
    ring.push(ring[0]!);
    return [ring];
  };
  const out = obstacles.map((o) => o);
  const snapped: { index: number; support: number; delta: number }[] = [];
  const suspects: { index: number; support: number; delta: number }[] = [];
  for (const it of items) {
    if (exclude.has(it.i) || it.zb <= SNAP_TOLERANCE || it.area < 1) continue;
    const o = obstacles[it.i]!;
    // supported where it is?
    let supported = false;
    let best: { support: number; delta: number; cover: number } | null = null;
    for (const s of items) {
      if (s.i === it.i || exclude.has(s.i)) continue;
      const delta = it.zb - s.zt;
      if (delta < -SNAP_TOLERANCE || delta > SNAP_MAX) continue;
      // the support must also stand on the ground or be lower: never snap onto something above
      if (s.box[2] < it.box[0] - delta - 1 || s.box[0] > it.box[2] + 1 || s.box[3] < it.box[1] - delta - 1 || s.box[1] > it.box[3] + 1) continue;
      if (Math.abs(delta) <= SNAP_TOLERANCE) {
        if (overlap(it.poly, s.poly) >= SNAP_SUPPORTED * it.area) {
          supported = true;
          break;
        }
        continue;
      }
      // a roof lands on a body at least as big as itself, never on a smaller thing
      if (delta < SNAP_MIN || s.area < it.area) continue;
      const cover = overlap(shiftedPoly(o, delta), s.poly) / it.area;
      if (cover >= SNAP_OVERLAP && (!best || cover > best.cover || (cover === best.cover && delta < best.delta))) best = { support: s.i, delta, cover };
    }
    if (supported || !best) continue;
    if (o.opaque && !opts.includeOpaque) {
      suspects.push({ index: it.i, support: best.support, delta: best.delta });
      continue;
    }
    const d = best.delta;
    out[it.i] = { ...o, points: o.points.map((p) => ({ ...p, y: p.y - d, z_bottom: p.z_bottom - d, z_top: p.z_top - d })) };
    snapped.push({ index: it.i, support: best.support, delta: d });
  }
  return { obstacles: out, snapped, suspects };
}

// ── transforms ───────────────────────────────────────────────────────

/** footprint centroid in map coordinates */
export function obstacleCentroid(points: ObstaclePoint[]): [number, number] {
  let x = 0;
  let y = 0;
  for (const p of points) {
    x += p.x;
    y += p.y;
  }
  return [x / points.length, y / points.length];
}

/** centroid of all parts of a group (before transforms) */
export function groupCentroid(parts: Level3DObject[]): [number, number] {
  const all = parts.flatMap((p) => p.obstacle.points);
  return all.length ? obstacleCentroid(all) : [0, 0];
}

function applyGame(points: ObstaclePoint[], t: GameTransform, pivot: [number, number]): ObstaclePoint[] {
  if (isIdentity(t)) return points;
  const a = (t.rot_deg * Math.PI) / 180;
  const c = Math.cos(a);
  const s = Math.sin(a);
  return points.map((p) => {
    const rx = p.x - pivot[0];
    const ry = p.y - pivot[1];
    return {
      ...p,
      x: pivot[0] + rx * c - ry * s + t.dx,
      y: pivot[1] + rx * s + ry * c + t.dy,
      z_bottom: p.z_bottom + t.dz,
      z_top: p.z_top + t.dz,
    };
  });
}

/** the parts of a group */
export function groupParts(doc: Level3D, groupId: string): Level3DObject[] {
  return doc.objects.filter((o) => o.group === groupId);
}

/** the obstacle as the bake writes it: the part's own transform, then its group's */
export function transformedObstacle(doc: Level3D, o: Level3DObject): SightObstacle {
  let points = applyGame(o.obstacle.points, o.transform, obstacleCentroid(o.obstacle.points));
  if (o.group) {
    const g = doc.groups.find((x) => x.id === o.group);
    if (g) points = applyGame(points, g.transform, groupCentroid(groupParts(doc, o.group)));
  }
  return points === o.obstacle.points ? o.obstacle : { ...o.obstacle, points };
}

/**
 * A game transform about a pivot as a 4x4 matrix (column-major) in the
 * Z-up scene frame of `scene.ts`. Rotation happens in game coordinates,
 * where footprints are true rectangles; in the scene frame Y is stretched
 * by 1/sin(elevation), so the result is an affine map, not a pure rotation.
 */
export function gameTransformMatrix(cam: MapCamera, t: GameTransform, pivot: [number, number]): number[] {
  const sinT = Math.sin((cam.elevation_deg * Math.PI) / 180);
  const cosT = Math.cos((cam.elevation_deg * Math.PI) / 180);
  const a = (t.rot_deg * Math.PI) / 180;
  const c = Math.cos(a);
  const s = Math.sin(a);
  // game: p' = R (p - c) + c + d  ->  scene X = x, Y = k y, Z = z / cosT with k = -1 / sinT
  const k = -1 / sinT;
  const m00 = c;
  const m01 = -s / k;
  const m10 = s * k;
  const m11 = c;
  const Cx = pivot[0];
  const Cy = pivot[1] * k;
  const tx = Cx - (m00 * Cx + m01 * Cy) + t.dx;
  const ty = Cy - (m10 * Cx + m11 * Cy) + t.dy * k;
  const tz = t.dz / cosT;
  return [m00, m10, 0, 0, m01, m11, 0, 0, 0, 0, 1, 0, tx, ty, tz, 1];
}

/** column-major 4x4 product a·b */
export function mulMatrix(a: number[], b: number[]): number[] {
  const out = new Array<number>(16).fill(0);
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      let v = 0;
      for (let k = 0; k < 4; k++) v += a[k * 4 + row]! * b[col * 4 + k]!;
      out[col * 4 + row] = v;
    }
  }
  return out;
}

/** the part's matrix in the scene frame (own transform, then the group's) */
export function partMatrix(cam: MapCamera, doc: Level3D, o: Level3DObject): number[] {
  const own = gameTransformMatrix(cam, o.transform, obstacleCentroid(o.obstacle.points));
  if (!o.group) return own;
  const g = doc.groups.find((x) => x.id === o.group);
  if (!g) return own;
  return mulMatrix(gameTransformMatrix(cam, g.transform, groupCentroid(groupParts(doc, o.group))), own);
}
