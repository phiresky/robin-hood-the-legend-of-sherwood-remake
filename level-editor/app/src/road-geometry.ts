import * as THREE from "three";
import type { Level3D, LevelSpline, MapCamera } from "@rle/shared";
import { sampleSpline } from "../../shared/src/spline-sampling.ts";
import { terrainTriangles, type TerrainTriangle } from "../../shared/src/authored-terrain.ts";

type Vertex = { x: number; y: number; z: number; offset: number; u: number; v: number };
type Bounds = { minX: number; minY: number; maxX: number; maxY: number };
type IndexedTriangle = { triangle: TerrainTriangle; bounds: Bounds };
const indices = new WeakMap<
  TerrainTriangle[],
  { spacing: number; buckets: Map<string, IndexedTriangle[]> }
>();
const epsilon = 1e-8;
function bounds(points: { x: number; y: number }[]): Bounds {
  return {
    minX: Math.min(...points.map((p) => p.x)),
    minY: Math.min(...points.map((p) => p.y)),
    maxX: Math.max(...points.map((p) => p.x)),
    maxY: Math.max(...points.map((p) => p.y)),
  };
}
function bucketKeys(b: Bounds, spacing: number) {
  const keys: string[] = [];
  for (let x = Math.floor(b.minX / spacing); x <= Math.floor(b.maxX / spacing); x++)
    for (let y = Math.floor(b.minY / spacing); y <= Math.floor(b.maxY / spacing); y++)
      keys.push(`${x}/${y}`);
  return keys;
}
function terrainIndex(document: Level3D) {
  const triangles = terrainTriangles(document);
  const cached = indices.get(triangles);
  if (cached) return cached;
  const spacing = Math.max(16, document.terrain?.spacing ?? 128);
  const buckets = new Map<string, IndexedTriangle[]>();
  for (const triangle of triangles) {
    const item = { triangle, bounds: bounds(triangle.points.map((p) => ({ x: p[0], y: p[1] }))) };
    for (const key of bucketKeys(item.bounds, spacing)) {
      const bucket = buckets.get(key);
      if (bucket) bucket.push(item);
      else buckets.set(key, [item]);
    }
  }
  const result = { spacing, buckets };
  indices.set(triangles, result);
  return result;
}
function interpolate(a: Vertex, b: Vertex, t: number): Vertex {
  return {
    x: a.x + (b.x - a.x) * t,
    y: a.y + (b.y - a.y) * t,
    z: a.z + (b.z - a.z) * t,
    offset: a.offset + (b.offset - a.offset) * t,
    u: a.u + (b.u - a.u) * t,
    v: a.v + (b.v - a.v) * t,
  };
}
function useful(polygon: Vertex[]) {
  if (polygon.length < 3) return false;
  const a = polygon[0]!;
  let area = 0;
  for (let i = 1; i + 1 < polygon.length; i++) {
    const b = polygon[i]!,
      c = polygon[i + 1]!;
    area += (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
  }
  return Math.abs(area) > epsilon;
}
function overlaps(a: Bounds, b: Bounds) {
  return a.minX <= b.maxX && a.maxX >= b.minX && a.minY <= b.maxY && a.maxY >= b.minY;
}

/** Partition a convex ribbon polygon; outside pieces preserve the road beyond the grid. */
function partition(polygon: Vertex[], triangle: TerrainTriangle) {
  const points = triangle.points;
  const [a, b, c] = points;
  const orientation = Math.sign((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]));
  let inside = polygon;
  const outside: Vertex[][] = [];
  for (let edge = 0; edge < 3 && inside.length; edge++) {
    const p = points[edge]!,
      q = points[(edge + 1) % 3]!;
    const distance = (v: Vertex) =>
      orientation * ((q[0] - p[0]) * (v.y - p[1]) - (q[1] - p[1]) * (v.x - p[0]));
    const keep: Vertex[] = [],
      reject: Vertex[] = [];
    for (let i = 0; i < inside.length; i++) {
      const v = inside[i]!,
        w = inside[(i + 1) % inside.length]!;
      const dv = distance(v),
        dw = distance(w);
      if (dv >= 0) keep.push(v);
      else reject.push(v);
      if (dv >= 0 !== dw >= 0) {
        const cut = interpolate(v, w, dv / (dv - dw));
        keep.push(cut);
        reject.push(cut);
      }
    }
    if (useful(reject)) outside.push(reject);
    inside = keep;
  }
  return { inside, outside };
}
function height(triangle: TerrainTriangle, p: Vertex) {
  const [a, b, c] = triangle.points;
  const d = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const u = ((p.x - a[0]) * (c[1] - a[1]) - (p.y - a[1]) * (c[0] - a[0])) / d;
  const v = ((b[0] - a[0]) * (p.y - a[1]) - (b[1] - a[1]) * (p.x - a[0])) / d;
  return a[2] + u * (b[2] - a[2]) + v * (c[2] - a[2]) + p.offset;
}

/** Cut roads at every terrain edge so even narrow ridges and channels remain flush. */
export function roadGeometry(
  path: LevelSpline,
  camera: MapCamera,
  document: Level3D,
): THREE.BufferGeometry {
  const sine = Math.sin((camera.elevation_deg * Math.PI) / 180),
    cosine = Math.cos((camera.elevation_deg * Math.PI) / 180);
  const { spacing, buckets } = terrainIndex(document);
  const positions: number[] = [],
    uvs: number[] = [];
  const pairs = sampleSpline(path, camera).map((sample) => {
    const normal = new THREE.Vector3(-sample.tangent.y, sample.tangent.x, 0)
      .normalize()
      .multiplyScalar((sample.width * sample.lateralScale) / 2);
    return [-1, 1].map((sign) => ({
      x: sample.position.x + sign * normal.x,
      y: -(sample.position.y + sign * normal.y) * sine,
      z: sample.position.z * cosine,
      offset: sample.heightOffset,
      u: (sign + 1) / 2,
      v: sample.distance / path.repeatLength,
    }));
  });
  const emit = (polygon: Vertex[], triangle?: TerrainTriangle) => {
    for (let i = 1; i + 1 < polygon.length; i++) {
      const a = polygon[0]!,
        b = polygon[i]!,
        c = polygon[i + 1]!;
      if (Math.abs((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) < epsilon) continue;
      for (const p of [a, b, c]) {
        positions.push(p.x, -p.y / sine, (triangle ? height(triangle, p) : p.z) / cosine + 0.8);
        uvs.push(p.u, p.v);
      }
    }
  };
  for (let i = 0; i + 1 < pairs.length; i++) {
    const a = pairs[i]!,
      b = pairs[i + 1]!;
    for (const ribbon of [
      [a[0]!, a[1]!, b[0]!],
      [a[1]!, b[1]!, b[0]!],
    ]) {
      const box = bounds(ribbon),
        candidates = new Set<IndexedTriangle>();
      for (const key of bucketKeys(box, spacing))
        for (const item of buckets.get(key) ?? []) {
          const other = item.bounds;
          if (
            other.minX <= box.maxX &&
            other.maxX >= box.minX &&
            other.minY <= box.maxY &&
            other.maxY >= box.minY
          )
            candidates.add(item);
        }
      let remaining = [{ polygon: ribbon, bounds: box }];
      for (const { triangle, bounds: triangleBounds } of candidates) {
        const next: typeof remaining = [];
        for (const piece of remaining) {
          if (!overlaps(piece.bounds, triangleBounds)) {
            next.push(piece);
            continue;
          }
          const { inside, outside } = partition(piece.polygon, triangle);
          emit(inside, triangle);
          for (const polygon of outside) next.push({ polygon, bounds: bounds(polygon) });
        }
        remaining = next;
        if (!remaining.length) break;
      }
      for (const { polygon } of remaining) emit(polygon);
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geometry.computeVertexNormals();
  return geometry;
}
