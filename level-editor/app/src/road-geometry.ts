import * as THREE from "three";
import type { Level3D, LevelSpline, MapCamera } from "@rle/shared";
import { sampleSpline } from "../../shared/src/spline-sampling.ts";
import {
  terrainTrianglesInBounds,
  terrainHeightAt,
  type TerrainTriangle,
} from "../../shared/src/authored-terrain.ts";

export type RibbonVertex = {
  x: number;
  y: number;
  z: number;
  offset: number;
  u: number;
  v: number;
};
type Vertex = RibbonVertex;
type Bounds = { minX: number; minY: number; maxX: number; maxY: number };
const epsilon = 1e-8;
function bounds(points: { x: number; y: number }[]): Bounds {
  return {
    minX: Math.min(...points.map((p) => p.x)),
    minY: Math.min(...points.map((p) => p.y)),
    maxX: Math.max(...points.map((p) => p.x)),
    maxY: Math.max(...points.map((p) => p.y)),
  };
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
  // Extended triangle edges can split a ribbon even when the triangle misses it.
  // Keep that ribbon intact instead of carrying artificial fragments to later cuts.
  return useful(inside) ? { inside, outside } : { inside: [], outside: [polygon] };
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
  return drapeRibbonGeometry(pairs, camera, document);
}

/** Shared exact terrain clipping for surface ribbons. Optional water floor keeps banks visible at the shoreline. */
export function drapeRibbonGeometry(
  pairs: RibbonVertex[][],
  camera: MapCamera,
  document?: Level3D,
  waterFloor = false,
) {
  const sine = Math.sin((camera.elevation_deg * Math.PI) / 180),
    cosine = Math.cos((camera.elevation_deg * Math.PI) / 180);
  const positions: number[] = [],
    uvs: number[] = [];
  const emit = (polygon: Vertex[], triangle?: TerrainTriangle) => {
    for (let i = 1; i + 1 < polygon.length; i++) {
      const a = polygon[0]!,
        b = polygon[i]!,
        c = polygon[i + 1]!;
      if (Math.abs((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) < epsilon) continue;
      for (const p of [a, b, c]) {
        const z = triangle ? height(triangle, p) : p.z;
        positions.push(p.x, -p.y / sine, (waterFloor ? Math.max(p.z, z) : z) / cosine + 0.8);
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
      const box = bounds(ribbon);
      const candidates = document
        ? terrainTrianglesInBounds(document, [box.minX, box.minY, box.maxX, box.maxY])
        : [];
      let remaining = [{ polygon: ribbon, bounds: box }];
      for (const triangle of candidates) {
        const triangleBounds = bounds(triangle.points.map((p) => ({ x: p[0], y: p[1] })));
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

/** Bounded live preview; release/export still use exact terrain-edge clipping. */
export function previewRoadGeometry(path: LevelSpline, camera: MapCamera, document: Level3D) {
  const sine = Math.sin((camera.elevation_deg * Math.PI) / 180),
    cosine = Math.cos((camera.elevation_deg * Math.PI) / 180);
  const samples = sampleSpline(path, camera, { spacing: 24, maxSamples: 256 });
  const columns = Math.max(
    2,
    Math.min(8, Math.ceil(Math.max(...samples.map((s) => s.width)) / 16)),
  );
  const positions: number[] = [],
    uvs: number[] = [],
    indices: number[] = [];
  for (let row = 0; row < samples.length; row++) {
    const sample = samples[row]!;
    const normal = new THREE.Vector3(-sample.tangent.y, sample.tangent.x, 0)
      .normalize()
      .multiplyScalar((sample.width * sample.lateralScale) / 2);
    for (let column = 0; column <= columns; column++) {
      const u = column / columns,
        side = u * 2 - 1;
      const x = sample.position.x + normal.x * side,
        y = sample.position.y + normal.y * side;
      const height = terrainHeightAt(document, x, -y * sine);
      positions.push(
        x,
        y,
        height === undefined
          ? sample.position.z + 0.8
          : (height + sample.heightOffset) / cosine + 0.8,
      );
      uvs.push(u, sample.distance / path.repeatLength);
      if (row && column < columns) {
        const b = row * (columns + 1) + column,
          a = b - columns - 1;
        indices.push(a, a + 1, b, a + 1, b + 1, b);
      }
    }
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("uv", new THREE.Float32BufferAttribute(uvs, 2));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}
