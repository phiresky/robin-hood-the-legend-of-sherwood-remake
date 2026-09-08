import earcut from "earcut";
import polygonClipping, {
  type MultiPolygon,
  type Ring,
} from "polygon-clipping";
import {
  gameToScene,
  signedPolygonArea,
  type MapCamera,
  type ProtoLevel,
} from "@rle/shared";
export const TERRACE_AREA = 100000;
const MIN_HEIGHT = 2;
export interface Face {
  /** triangle ids */
  tris: number[];
  /** vertex ids (each face owns its vertices) */
  verts: number[];
  obstacle: number;
  kind: "side" | "top";
  /** outward normal xy (sides) */
  nx: number;
  ny: number;
  /**
   * the face's 2D frame: origin and in-plane axes e1/e2 (scene units).
   * Walls: e1 along the wall, e2 up. Roof parts: e1 along the ridge if
   * there is one, e2 down-slope away from it, so the two slopes of a gable
   * share u along the ridge and v as the distance from it.
   */
  plane: {
    origin: [number, number, number];
    e1: [number, number, number];
    e2: [number, number, number];
  };
  /** tops: the other slope of the same roof sharing the ridge, if any */
  ridgeMate: number;
}

export interface Geometry {
  positions: Float32Array<ArrayBuffer>;
  /** flat triangle list, oriented outward */
  tris: Uint32Array<ArrayBuffer>;
  faces: Face[];
  faceOfTri: Int32Array;
  terraceIds: Set<number>;
  obstacles: number;
  clipping: { sameFacing: number; sharedInterior: number };
}

export function cross(a: number[], b: number[]): number[] {
  return [
    a[1]! * b[2]! - a[2]! * b[1]!,
    a[2]! * b[0]! - a[0]! * b[2]!,
    a[0]! * b[1]! - a[1]! * b[0]!,
  ];
}

export function footprintArea(pts: { x: number; y: number }[]): number {
  return Math.abs(signedPolygonArea(pts.map((p) => [p.x, p.y])));
}

// ── geometry ─────────────────────────────────────────────────────────

type V3 = [number, number, number];
const v3sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const v3add = (a: V3, b: V3): V3 => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const v3scale = (a: V3, s: number): V3 => [a[0] * s, a[1] * s, a[2] * s];
const v3dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const v3cross = (a: V3, b: V3): V3 => cross(a, b) as V3;
const v3norm = (a: V3): V3 => {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};

export type Box = [number, number, number, number];

/** planes within this angle (degrees) and offset (map px) are the same plane */
const PLANE_ANGLE = 0.5;
const PLANE_OFFSET = 0.25;
/** face polygons smaller than this (px²) or thinner than this (area / perimeter, px) after clipping are dropped */
const MIN_FACE_AREA = 1;
const MIN_FACE_WIDTH = 1;

/** the canonical 2D frame of a plane, identical for every face on that plane (either facing) */
interface PlaneFrame {
  key: string;
  origin: V3;
  e1: V3;
  e2: V3;
  /** canonical unit normal */
  n: V3;
  /** the face's outward normal is -n */
  flip: boolean;
}

function canonicalFrame(n: V3, p: V3): PlaneFrame {
  if (Math.abs(n[2]) < 1e-6) {
    // vertical plane: unsigned direction of the normal in [0, 180)
    const deg = (Math.atan2(n[1], n[0]) * 180) / Math.PI;
    let a = Math.round(deg / PLANE_ANGLE) * PLANE_ANGLE;
    let flip = false;
    if (a < 0) {
      a += 180;
      flip = true;
    }
    if (a >= 180) {
      a -= 180;
      flip = !flip;
    }
    const rad = (a * Math.PI) / 180;
    const nc: V3 = [Math.cos(rad), Math.sin(rad), 0];
    const dq = Math.round(v3dot(nc, p) / PLANE_OFFSET) * PLANE_OFFSET;
    return {
      key: `w${a}:${dq}`,
      origin: v3scale(nc, dq),
      e1: [-Math.sin(rad), Math.cos(rad), 0],
      e2: [0, 0, 1],
      n: nc,
      flip,
    };
  }
  // sloped or flat plane, normal points up
  const nc = v3norm([
    Math.round(n[0] * 1000) / 1000,
    Math.round(n[1] * 1000) / 1000,
    Math.round(n[2] * 1000) / 1000,
  ]);
  const dq = Math.round(v3dot(nc, p) / PLANE_OFFSET) * PLANE_OFFSET;
  const e1 =
    Math.abs(nc[2]) > 0.9999
      ? ([1, 0, 0] as V3)
      : v3norm(v3cross([0, 0, 1], nc));
  const e2 = v3cross(nc, e1);
  return {
    key: `t${nc.map((v) => v.toFixed(3)).join(",")}:${dq}`,
    origin: v3scale(nc, dq),
    e1,
    e2,
    n: nc,
    flip: false,
  };
}

/** a planar face before triangulation: polygon(s) in the canonical frame of its plane */
interface Planar {
  obstacle: number;
  kind: "side" | "top";
  frame: PlaneFrame;
  polys: MultiPolygon;
  /** who keeps overlapping area: terrace > opaque > non-opaque, then larger area */
  priority: number;
  /** tops: the other slope of the same roof (index into the planar list) */
  ridgeMate: number;
}

function ringArea(r: Ring): number {
  return Math.abs(signedPolygonArea(r));
}

function polysArea(m: MultiPolygon): number {
  let a = 0;
  for (const poly of m)
    for (const [k, ring] of poly.entries())
      a += k === 0 ? ringArea(ring) : -ringArea(ring);
  return a;
}

function polysBox(m: MultiPolygon): Box {
  let u0 = Infinity,
    v0 = Infinity,
    u1 = -Infinity,
    v1 = -Infinity;
  for (const poly of m) {
    for (const [u, v] of poly[0]!) {
      u0 = Math.min(u0, u);
      u1 = Math.max(u1, u);
      v0 = Math.min(v0, v);
      v1 = Math.max(v1, v);
    }
  }
  return [u0, v0, u1, v1];
}

function ringLength(r: Ring): number {
  let l = 0;
  for (let i = 0; i < r.length - 1; i++)
    l += Math.hypot(r[i + 1]![0] - r[i]![0], r[i + 1]![1] - r[i]![1]);
  return l;
}

/** drop polygons below MIN_FACE_AREA and slivers thinner than MIN_FACE_WIDTH */
function cleanPolys(m: MultiPolygon): MultiPolygon {
  return m.filter((poly) => {
    const a = ringArea(poly[0]!);
    return (
      a >= MIN_FACE_AREA &&
      a / Math.max(1, ringLength(poly[0]!)) >= MIN_FACE_WIDTH / 2
    );
  });
}

/**
 * Build the faces: every obstacle's walls and the planar parts of its top,
 * then remove what coincides. Faces on the same plane facing the same way
 * (a jettied floor's front on the main box, a house wall on the terrace
 * cliff, coplanar roofs) keep only the highest-priority one over the
 * overlap; faces on the same plane facing each other (the shared wall of
 * two adjacent houses) lose the overlap on both sides, since it is inside
 * the joined block. So no two triangles share depth and every map pixel has
 * exactly one owner.
 */
export function buildGeometry(
  level: Pick<ProtoLevel, "sight_obstacles">,
  cam: MapCamera,
  opaqueOnly: boolean,
  noTerraces: boolean,
): Geometry {
  const terraceIds = new Set<number>();
  level.sight_obstacles.forEach((o, i) => {
    if (o.points.length >= 3 && footprintArea(o.points) > TERRACE_AREA)
      terraceIds.add(i);
  });
  const planar: Planar[] = [];
  const toUV = (fr: PlaneFrame, p: V3): [number, number] => {
    const d = v3sub(p, fr.origin);
    return [v3dot(d, fr.e1), v3dot(d, fr.e2)];
  };

  let obstacles = 0;
  for (const [oi, o] of level.sight_obstacles.entries()) {
    if (opaqueOnly && !o.opaque) continue;
    const n = o.points.length;
    if (n < 3) continue;
    if (Math.max(...o.points.map((p) => p.z_top - p.z_bottom)) < MIN_HEIGHT)
      continue;
    if (noTerraces && terraceIds.has(oi)) continue;
    obstacles++;
    const priority = terraceIds.has(oi) ? 3 : o.opaque ? 2 : 1;
    const bottom = o.points.map(
      (p) => gameToScene(cam, p.x, p.y, p.z_bottom) as V3,
    );
    const top = o.points.map((p) => gameToScene(cam, p.x, p.y, p.z_top) as V3);
    // winding of the footprint in the scene frame (Y is flipped vs. game y)
    const ccw = signedPolygonArea(bottom.map((p) => [p[0], p[1]])) > 0;

    // sides: one quad per footprint edge
    for (let i = 0; i < n; i++) {
      const j = (i + 1) % n;
      if (top[i]![2] - bottom[i]![2] < 0.5 && top[j]![2] - bottom[j]![2] < 0.5)
        continue;
      const ex = bottom[j]![0] - bottom[i]![0];
      const ey = bottom[j]![1] - bottom[i]![1];
      const outward = v3norm(ccw ? [ey, -ex, 0] : [-ey, ex, 0]);
      const frame = canonicalFrame(outward, bottom[i]!);
      const ring: Ring = [bottom[i]!, bottom[j]!, top[j]!, top[i]!].map((p) =>
        toUV(frame, p),
      );
      ring.push(ring[0]!);
      planar.push({
        obstacle: oi,
        kind: "side",
        frame,
        polys: [[ring]],
        priority,
        ridgeMate: -1,
      });
    }

    // top: triangulate the footprint (scene XY), each vertex at its z_top,
    // then split the triangles into planar parts (a gable's two slopes are
    // separate faces: the back slope is seen edge-on or not at all)
    const flat: number[] = [];
    for (const p of top) flat.push(p[0], p[1]);
    const ears = earcut(flat);
    if (ears.length === 0) continue;
    const triCount = ears.length / 3;
    const normals: V3[] = [];
    for (let k = 0; k < triCount; k++) {
      const a = top[ears[k * 3]!]!;
      const b = top[ears[k * 3 + 1]!]!;
      const c = top[ears[k * 3 + 2]!]!;
      const nrm = v3cross(v3sub(b, a), v3sub(c, a));
      normals.push(v3norm(nrm[2] < 0 ? v3scale(nrm, -1) : nrm));
    }
    const part = Array.from({ length: triCount }, (_, k) => k);
    const find = (k: number): number =>
      part[k] === k ? k : (part[k] = find(part[k]!));
    const edgeOwner = new Map<string, number>();
    for (let k = 0; k < triCount; k++) {
      for (let e = 0; e < 3; e++) {
        const i = ears[k * 3 + e]!;
        const j = ears[k * 3 + ((e + 1) % 3)]!;
        const key = i < j ? `${i}-${j}` : `${j}-${i}`;
        const other = edgeOwner.get(key);
        if (other === undefined) {
          edgeOwner.set(key, k);
          continue;
        }
        if (v3dot(normals[k]!, normals[other]!) > 0.995)
          part[find(k)] = find(other);
      }
    }
    const parts = new Map<number, number[]>();
    for (let k = 0; k < triCount; k++) {
      const r = find(k);
      const list = parts.get(r) ?? [];
      list.push(k);
      parts.set(r, list);
    }
    const partIds: { index: number; points: Set<number> }[] = [];
    for (const triIds of parts.values()) {
      const nsum: V3 = [0, 0, 0];
      const pointSet = new Set<number>();
      for (const k of triIds) {
        const a = top[ears[k * 3]!]!;
        const b = top[ears[k * 3 + 1]!]!;
        const c = top[ears[k * 3 + 2]!]!;
        const nrm = v3cross(v3sub(b, a), v3sub(c, a));
        const s = nrm[2] < 0 ? -1 : 1;
        nsum[0] += s * nrm[0];
        nsum[1] += s * nrm[1];
        nsum[2] += s * nrm[2];
        for (let e = 0; e < 3; e++) pointSet.add(ears[k * 3 + e]!);
      }
      const normal = v3norm(nsum);
      const frame = canonicalFrame(normal, top[ears[triIds[0]! * 3]!]!);
      const tris: MultiPolygon = triIds.map((k) => {
        const ring: Ring = [0, 1, 2].map((e) =>
          toUV(frame, top[ears[k * 3 + e]!]!),
        );
        ring.push(ring[0]!);
        return [ring];
      });
      let polys: MultiPolygon;
      try {
        polys = polygonClipping.union(tris[0]!, ...tris.slice(1));
      } catch {
        polys = tris;
      }
      partIds.push({ index: planar.length, points: pointSet });
      planar.push({
        obstacle: oi,
        kind: "top",
        frame,
        polys,
        priority,
        ridgeMate: -1,
      });
    }
    // ridges: parts of the same obstacle sharing two footprint points
    for (const a of partIds) {
      for (const b of partIds) {
        if (a === b || planar[a.index]!.ridgeMate >= 0) continue;
        if ([...a.points].filter((p) => b.points.has(p)).length >= 2)
          planar[a.index]!.ridgeMate = b.index;
      }
    }
  }

  // ── coincident faces ──
  const groups = new Map<string, number[]>();
  planar.forEach((p, i) => {
    const list = groups.get(p.frame.key) ?? [];
    list.push(i);
    groups.set(p.frame.key, list);
  });
  let clippedSame = 0;
  let clippedOpposite = 0;
  const boxesOverlap = (a: Box, b: Box) =>
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
  for (const ids of groups.values()) {
    if (ids.length < 2) continue;
    const order = [...ids].sort(
      (a, b) =>
        planar[b]!.priority - planar[a]!.priority ||
        polysArea(planar[b]!.polys) - polysArea(planar[a]!.polys) ||
        a - b,
    );
    const boxes = new Map(order.map((i) => [i, polysBox(planar[i]!.polys)]));
    // same facing: the higher-priority face keeps the overlap
    for (let x = 0; x < order.length; x++) {
      const a = planar[order[x]!]!;
      if (a.polys.length === 0) continue;
      for (let y = x + 1; y < order.length; y++) {
        const b = planar[order[y]!]!;
        if (
          b.polys.length === 0 ||
          a.frame.flip !== b.frame.flip ||
          !boxesOverlap(boxes.get(order[x]!)!, boxes.get(order[y]!)!)
        )
          continue;
        const before = polysArea(b.polys);
        b.polys = cleanPolys(polygonClipping.difference(b.polys, a.polys));
        if (polysArea(b.polys) < before - 0.5) clippedSame++;
      }
    }
    // opposite facing: both lose the overlap (it is inside the joined solid)
    for (let x = 0; x < order.length; x++) {
      const a = planar[order[x]!]!;
      for (let y = x + 1; y < order.length; y++) {
        const b = planar[order[y]!]!;
        if (
          a.polys.length === 0 ||
          b.polys.length === 0 ||
          a.frame.flip === b.frame.flip
        )
          continue;
        if (!boxesOverlap(boxes.get(order[x]!)!, boxes.get(order[y]!)!))
          continue;
        const ov = cleanPolys(polygonClipping.intersection(a.polys, b.polys));
        if (ov.length === 0) continue;
        a.polys = cleanPolys(polygonClipping.difference(a.polys, ov));
        b.polys = cleanPolys(polygonClipping.difference(b.polys, ov));
        clippedOpposite++;
      }
    }
  }
  // ── triangulate ──
  const positions: number[] = [];
  const tris: number[] = [];
  const faces: Face[] = [];
  const faceOfTri: number[] = [];
  const faceOfPlanar = new Int32Array(planar.length).fill(-1);
  const addVertex = (p: V3): number => {
    positions.push(p[0], p[1], p[2]);
    return positions.length / 3 - 1;
  };
  const addTri = (
    face: number,
    a: number,
    b: number,
    c: number,
    outward: V3,
  ) => {
    const pa = positions.slice(a * 3, a * 3 + 3) as V3;
    const pb = positions.slice(b * 3, b * 3 + 3) as V3;
    const pc = positions.slice(c * 3, c * 3 + 3) as V3;
    const nrm = v3cross(v3sub(pb, pa), v3sub(pc, pa));
    if (v3dot(nrm, outward) >= 0) tris.push(a, b, c);
    else tris.push(a, c, b);
    faceOfTri.push(face);
    faces[face]!.tris.push(tris.length / 3 - 1);
  };
  for (const [pi, p] of planar.entries()) {
    if (p.polys.length === 0) continue;
    const { frame } = p;
    const outward: V3 = frame.flip ? v3scale(frame.n, -1) : frame.n;
    const face = faces.length;
    faces.push({
      tris: [],
      verts: [],
      obstacle: p.obstacle,
      kind: p.kind,
      nx: p.kind === "side" ? outward[0] : 0,
      ny: p.kind === "side" ? outward[1] : 0,
      plane: { origin: frame.origin, e1: frame.e1, e2: frame.e2 },
      ridgeMate: -1,
    });
    faceOfPlanar[pi] = face;
    for (const poly of p.polys) {
      const flat: number[] = [];
      const holes: number[] = [];
      const ids: number[] = [];
      for (const [k, ring] of poly.entries()) {
        const closed =
          ring.length > 1 &&
          ring[0]![0] === ring[ring.length - 1]![0] &&
          ring[0]![1] === ring[ring.length - 1]![1];
        const pts = closed ? ring.slice(0, -1) : ring;
        if (k > 0) holes.push(flat.length / 2);
        for (const [u, v] of pts) {
          flat.push(u, v);
          const id = addVertex(
            v3add(
              frame.origin,
              v3add(v3scale(frame.e1, u), v3scale(frame.e2, v)),
            ),
          );
          ids.push(id);
          faces[face]!.verts.push(id);
        }
      }
      const ears = earcut(flat, holes.length ? holes : undefined);
      for (let k = 0; k < ears.length; k += 3)
        addTri(
          face,
          ids[ears[k]!]!,
          ids[ears[k + 1]!]!,
          ids[ears[k + 2]!]!,
          outward,
        );
    }
  }
  // ridge mates and ridge-aligned roof frames (u along the ridge from its
  // lower-x end, v away from the ridge), so both slopes of a gable line up
  const P = (v: number): V3 => [
    positions[v * 3]!,
    positions[v * 3 + 1]!,
    positions[v * 3 + 2]!,
  ];
  for (const [pi, p] of planar.entries()) {
    const f = faceOfPlanar[pi]!;
    if (f < 0 || p.ridgeMate < 0) continue;
    const m = faceOfPlanar[p.ridgeMate]!;
    if (m < 0) continue;
    faces[f]!.ridgeMate = m;
    const shared: V3[] = [];
    for (const a of faces[f]!.verts) {
      const pa = P(a);
      if (faces[m]!.verts.some((b) => Math.hypot(...v3sub(P(b), pa)) < 0.5))
        shared.push(pa);
    }
    if (shared.length < 2) continue;
    let r0 = shared[0]!,
      r1 = shared[1]!,
      best = -1;
    for (const a of shared) {
      for (const b of shared) {
        const d = Math.hypot(...v3sub(a, b));
        if (d > best) {
          best = d;
          [r0, r1] = [a, b];
        }
      }
    }
    if (r0[0] > r1[0] || (r0[0] === r1[0] && r0[1] > r1[1]))
      [r0, r1] = [r1, r0];
    const e1 = v3norm(v3sub(r1, r0));
    const outward = faces[f]!.kind === "top" ? p.frame.n : p.frame.n;
    let e2 = v3cross(outward, e1);
    const centroid: V3 = [0, 0, 0];
    for (const v of faces[f]!.verts) {
      const q = P(v);
      centroid[0] += q[0] / faces[f]!.verts.length;
      centroid[1] += q[1] / faces[f]!.verts.length;
      centroid[2] += q[2] / faces[f]!.verts.length;
    }
    if (v3dot(e2, v3sub(centroid, r0)) < 0) e2 = v3scale(e2, -1);
    faces[f]!.plane = { origin: r0, e1, e2 };
  }
  return {
    positions: Float32Array.from(positions),
    tris: Uint32Array.from(tris),
    faces,
    faceOfTri: Int32Array.from(faceOfTri),
    terraceIds,
    obstacles,
    clipping: { sameFacing: clippedSame, sharedInterior: clippedOpposite },
  };
}
