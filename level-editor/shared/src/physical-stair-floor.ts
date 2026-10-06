import { heightPlane, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { onClippedReceivingBoundary, pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

export interface PhysicalFloorPatch {
  plane: HeightPlane;
  boundary: Point[];
}

/** Parameters along AB where CD touches or crosses it, including collinear ends. */
function intersections(a: Point, b: Point, c: Point, d: Point): number[] {
  const r: Point = [b[0] - a[0], b[1] - a[1]];
  const s: Point = [d[0] - c[0], d[1] - c[1]];
  const q: Point = [c[0] - a[0], c[1] - a[1]];
  const cross = (u: Point, v: Point) => u[0] * v[1] - u[1] * v[0];
  const length = Math.hypot(...r);
  if (length === 0) return [];
  const determinant = cross(r, s);
  if (Math.abs(determinant) > 1e-12 * length * Math.hypot(...s)) {
    const t = cross(q, s) / determinant,
      u = cross(q, r) / determinant;
    return t >= -1e-10 && t <= 1 + 1e-10 && u >= -1e-10 && u <= 1 + 1e-10
      ? [Math.max(0, Math.min(1, t))]
      : [];
  }
  if (Math.abs(cross(q, r)) > 1e-8 * length) return [];
  return [c, d]
    .map((p) => ((p[0] - a[0]) * r[0] + (p[1] - a[1]) * r[1]) / length ** 2)
    .filter((t) => t >= -1e-10 && t <= 1 + 1e-10)
    .map((t) => Math.max(0, Math.min(1, t)));
}

function contains(patch: PhysicalFloorPatch, point: Point) {
  return (
    pointInGameplayPolygon(point, patch.boundary, true) ||
    onClippedReceivingBoundary(point, patch.boundary)
  );
}

/** Retain every flight's height function; never fit one plane across a bend. */
export function physicalStairFloor(surfaces: { polygon: Vec3[] }[]) {
  const vertices = surfaces.flatMap((surface) => surface.polygon);
  if (vertices.length < 3 || vertices.some((p) => p.some((n) => !Number.isFinite(n))))
    throw new Error("Physical stair needs finite floor vertices");
  const patches: PhysicalFloorPatch[] = surfaces.map(({ polygon }) => ({
    plane: heightPlane(polygon),
    boundary: polygon.map(([x, y]) => [x, y]),
  }));
  const plane = heightPlane(vertices, false);
  const piecewise = vertices.some(([x, y, z]) => Math.abs(planeHeight(plane, [x, y]) - z) > 1e-4);
  if (piecewise)
    for (const [index, patch] of patches.entries())
      for (const other of patches.slice(0, index)) {
        const shared = [
          ...patch.boundary.filter((p) => contains(other, p)),
          ...other.boundary.filter((p) => contains(patch, p)),
        ];
        for (const [i, a] of patch.boundary.entries()) {
          const b = patch.boundary[(i + 1) % patch.boundary.length]!;
          for (const [j, c] of other.boundary.entries()) {
            const d = other.boundary[(j + 1) % other.boundary.length]!;
            shared.push(
              ...intersections(a, b, c, d).map((t): Point => [
                a[0] + t * (b[0] - a[0]),
                a[1] + t * (b[1] - a[1]),
              ]),
            );
          }
        }
        if (
          shared.some(
            (point) =>
              Math.abs(planeHeight(patch.plane, point) - planeHeight(other.plane, point)) > 1e-4,
          )
        )
          throw new Error(
            "Physical stair floor patches disagree at their shared boundary or overlap",
          );
      }
  const heightAt = (point: Point): number => {
    if (!piecewise) return planeHeight(plane, point);
    const patch = patches.find((patch) => contains(patch, point));
    if (!patch) throw new Error("Physical stair point has no floor support");
    return planeHeight(patch.plane, point);
  };
  const splitRing = (ring: Point[]): Point[] => {
    if (!piecewise) return ring;
    return ring.flatMap((a, i) => {
      const b = ring[(i + 1) % ring.length]!;
      const cuts = [0];
      for (const patch of patches)
        for (const [j, c] of patch.boundary.entries())
          cuts.push(...intersections(a, b, c, patch.boundary[(j + 1) % patch.boundary.length]!));
      cuts.sort((a, b) => a - b);
      return cuts
        .filter((t, index) => t < 1 - 1e-10 && (index === 0 || t - cuts[index - 1]! > 1e-10))
        .map((t): Point => (t === 0 ? a : [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]));
    });
  };
  return { plane, patches: piecewise ? patches : undefined, heightAt, splitRing };
}
