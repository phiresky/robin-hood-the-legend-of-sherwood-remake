import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { ObstaclePoint, Point } from "../../shared/src/level.ts";
import { closedMeshComponents } from "./closed-mesh-components.ts";
import clipping, { type MultiPolygon } from "polygon-clipping";
import earcut, { flatten } from "earcut";

const cross = (a: Point, b: Point, c: Point) =>
  (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
const area = (ring: Point[]) =>
  Math.abs(
    ring.reduce((sum, p, i) => {
      const q = ring[(i + 1) % ring.length]!;
      return sum + p[0] * q[1] - p[1] * q[0];
    }, 0),
  ) / 2;

function intersection(subject: Point[], clip: Point[]): Point[] {
  let result = subject;
  for (let i = 0; i < clip.length; i++) {
    const a = clip[i]!,
      b = clip[(i + 1) % clip.length]!;
    const input = result;
    result = [];
    for (let j = 0; j < input.length; j++) {
      const p = input[j]!,
        q = input[(j + 1) % input.length]!;
      const dp = cross(a, b, p),
        dq = cross(a, b, q);
      if (dp >= 0) result.push(p);
      if (dp < 0 !== dq < 0) {
        const t = dp / (dp - dq);
        result.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
      }
    }
  }
  return cleanRing(result);
}

function cleanRing(input: Point[]): Point[] {
  const result = [...input];
  // Native cap planes use the first three points. Remove clipping duplicates
  // and collinear corners so those points define a plane.
  let changed = true;
  while (changed && result.length >= 3) {
    changed = false;
    for (let i = 0; i < result.length; i++) {
      if (
        Math.abs(
          cross(
            result[(i + result.length - 1) % result.length]!,
            result[i]!,
            result[(i + 1) % result.length]!,
          ),
        ) <= 1e-10
      ) {
        result.splice(i, 1);
        changed = true;
        break;
      }
    }
  }
  return result;
}

/** Convert one closed shell to convex native capped volumes. Pair adjacent
 * entry/exit faces along vertical rays, retaining gaps between solid intervals.
 * Crossing or coincident cap faces fail rather than silently filling cavities.
 * Nothing here chooses material, movement or sight semantics for the asset. */
export function meshCappedVolumes(mesh: readonly MaskTriangle[]): ObstaclePoint[][] {
  const components = closedMeshComponents(mesh);
  if (components.length !== 1) throw new Error("Capped conversion requires one closed shell");
  const triangles = components[0]!;
  const origin = triangles[0]![0];
  const signedVolume = triangles.reduce((sum, triangle) => {
    const [a, b, c] = triangle.map((p) => p.map((v, i) => v - origin[i]!));
    return (
      sum +
      (a![0]! * (b![1]! * c![2]! - b![2]! * c![1]!) +
        a![1]! * (b![2]! * c![0]! - b![0]! * c![2]!) +
        a![2]! * (b![0]! * c![1]! - b![1]! * c![0]!)) /
        6
    );
  }, 0);
  if (Math.abs(signedVolume) < 1e-9) throw new Error("Physical shell has no enclosed volume");
  const faces = triangles.flatMap(([a, b, c]) => {
    const ring: Point[] = [
      [a[0], a[1]],
      [b[0], b[1]],
      [c[0], c[1]],
    ];
    const determinant = cross(ring[0]!, ring[1]!, ring[2]!);
    if (Math.abs(determinant) < 1e-12) return [];
    const dx = ((b[2] - a[2]) * (c[1] - a[1]) - (c[2] - a[2]) * (b[1] - a[1])) / determinant;
    const dy = ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])) / determinant;
    if (determinant < 0) ring.reverse();
    return [
      {
        ring,
        top: determinant * signedVolume > 0,
        height: (p: Point) => a[2] + dx * (p[0] - a[0]) + dy * (p[1] - a[1]),
        bounds: [
          Math.min(a[0], b[0], c[0]),
          Math.min(a[1], b[1], c[1]),
          Math.max(a[0], b[0], c[0]),
          Math.max(a[1], b[1], c[1]),
        ],
      },
    ];
  });
  const volumes: ObstaclePoint[][] = [];
  const overlaps = new Map<string, Point[]>();
  const centre = (ring: Point[]): Point => [
    ring.reduce((sum, p) => sum + p[0], 0) / ring.length,
    ring.reduce((sum, p) => sum + p[1], 0) / ring.length,
  ];
  const key = (i: number, j: number) => `${Math.min(i, j)},${Math.max(i, j)}`;
  let coveredArea = 0,
    volume = 0;
  for (let i = 0; i < faces.length; i++) {
    const a = faces[i]!;
    for (let j = i + 1; j < faces.length; j++) {
      const b = faces[j]!;
      if (
        a.bounds[0]! >= b.bounds[2]! ||
        b.bounds[0]! >= a.bounds[2]! ||
        a.bounds[1]! >= b.bounds[3]! ||
        b.bounds[1]! >= a.bounds[3]!
      )
        continue;
      const ring = intersection(a.ring, b.ring);
      const size = area(ring);
      if (size <= 1e-9) continue;
      const deltas = ring.map((p) => a.height(p) - b.height(p));
      if (Math.min(...deltas) < -1e-6 && Math.max(...deltas) > 1e-6)
        throw new Error(`Physical shell has intersecting cap faces ${i}/${j}`);
      if (deltas.every((delta) => Math.abs(delta) < 1e-8))
        throw new Error(`Physical shell has coincident cap faces ${i}/${j}`);
      overlaps.set(key(i, j), ring);
    }
  }
  for (let i = 0; i < faces.length; i++) {
    const a = faces[i]!;
    for (let j = i + 1; j < faces.length; j++) {
      const b = faces[j]!;
      const ring = overlaps.get(key(i, j));
      if (!ring || a.top === b.top) continue;
      const top = a.top ? a : b,
        bottom = a.top ? b : a;
      if (top.height(centre(ring)) <= bottom.height(centre(ring))) continue;
      let regions: MultiPolygon = [[ring]];
      for (let k = 0; k < faces.length && regions.length; k++) {
        if (k === i || k === j || !overlaps.has(key(i, k)) || !overlaps.has(key(j, k))) continue;
        const other = faces[k]!;
        const overlap = intersection(ring, other.ring);
        if (area(overlap) <= 1e-9) continue;
        const p = centre(overlap),
          z = other.height(p);
        if (z > bottom.height(p) && z < top.height(p))
          regions = clipping.difference(regions, [other.ring]);
      }
      for (const region of regions) {
        const outer = cleanRing(region[0]!);
        if (outer.length < 3) continue;
        if (cross(outer[0]!, outer[1]!, outer[2]!) < 0) outer.reverse();
        let polygons: Point[][];
        if (
          region.length === 1 &&
          outer.every(
            (p, index) =>
              cross(p, outer[(index + 1) % outer.length]!, outer[(index + 2) % outer.length]!) > 0,
          )
        ) {
          // Native obstacles already support convex polygons; triangulating
          // them adds collision seams without improving the represented shape.
          polygons = [outer];
        } else {
          const flattened = flatten(region);
          const indices = earcut(flattened.vertices, flattened.holes, flattened.dimensions);
          polygons = [];
          for (let offset = 0; offset < indices.length; offset += 3) {
            const polygon: Point[] = indices
              .slice(offset, offset + 3)
              .map((index) => [flattened.vertices[index * 2]!, flattened.vertices[index * 2 + 1]!]);
            if (cross(polygon[0]!, polygon[1]!, polygon[2]!) < 0) polygon.reverse();
            polygons.push(polygon);
          }
        }
        for (const polygon of polygons) {
          const size = area(polygon);
          if (size <= 1e-9) continue;
          const points = polygon.map((p) => {
            let z_bottom = bottom.height(p),
              z_top = top.height(p);
            if (z_bottom > z_top + 1e-6)
              throw new Error("Physical shell has crossing top and bottom caps");
            if (z_bottom > z_top) z_bottom = z_top = (z_bottom + z_top) / 2;
            return { x: p[0], y: p[1], z_bottom, z_top };
          });
          for (let k = 1; k < points.length - 1; k++) {
            const triangleArea = Math.abs(cross(polygon[0]!, polygon[k]!, polygon[k + 1]!)) / 2;
            volume +=
              (triangleArea *
                [points[0]!, points[k]!, points[k + 1]!].reduce(
                  (sum, p) => sum + p.z_top - p.z_bottom,
                  0,
                )) /
              3;
          }
          coveredArea += size;
          volumes.push(points);
        }
      }
    }
  }
  for (const top of [false, true]) {
    const expected = faces
      .filter((face) => face.top === top)
      .reduce((sum, face) => sum + area(face.ring), 0);
    if (Math.abs(expected - coveredArea) > Math.max(1e-6, expected * 1e-8))
      throw new Error("Capped volumes do not cover the shell footprint");
  }
  if (
    !volumes.length ||
    Math.abs(volume - Math.abs(signedVolume)) > Math.max(1e-6, Math.abs(signedVolume) * 1e-8)
  )
    throw new Error("Capped volumes do not preserve enclosed mesh volume");
  return volumes;
}
