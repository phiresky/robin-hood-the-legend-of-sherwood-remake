import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { footprintBoundaryHull } from "./supported-footprint-centers.ts";

/** Test every portion of a segment, including crossings of concave boundaries. */
export function jumpSpanInsideReceiver(a: Point, b: Point, boundary: Point[]): boolean {
  if (!pointInGameplayPolygon(a, boundary, true) || !pointInGameplayPolygon(b, boundary, true))
    return false;
  const dx = b[0] - a[0],
    dy = b[1] - a[1];
  const lengthSquared = dx * dx + dy * dy;
  if (lengthSquared === 0) return true;
  const cuts = [0, 1];
  for (const [index, c] of boundary.entries()) {
    const d = boundary[(index + 1) % boundary.length]!;
    const ex = d[0] - c[0],
      ey = d[1] - c[1];
    const cx = c[0] - a[0],
      cy = c[1] - a[1];
    const determinant = dx * ey - dy * ex;
    if (Math.abs(determinant) > 1e-10) {
      const t = (cx * ey - cy * ex) / determinant;
      const u = (cx * dy - cy * dx) / determinant;
      if (t > 0 && t < 1 && u >= 0 && u <= 1) cuts.push(t);
    } else if (Math.abs(cx * dy - cy * dx) < 1e-8) {
      for (const point of [c, d]) {
        const t = ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / lengthSquared;
        if (t > 0 && t < 1) cuts.push(t);
      }
    }
  }
  cuts.sort((x, y) => x - y);
  return cuts.slice(1).every((end, index) => {
    const t = (cuts[index]! + end) / 2;
    return pointInGameplayPolygon([a[0] + dx * t, a[1] + dy * t], boundary, true);
  });
}

/** The swept walking box must remain supported across the entire jump span. */
export function jumpSpanHasFootprint(a: Point, b: Point, free: MultiPolygon, half: Point): boolean {
  if (half.some((value) => !Number.isFinite(value) || value <= 0))
    throw new Error("Jump footprint needs positive finite half extents");
  const sweep = footprintBoundaryHull(
    [a, b].flatMap(([x, y]) =>
      [-1, 1].flatMap((sx) => [-1, 1].map((sy): Point => [x + sx * half[0], y + sy * half[1]])),
    ),
  );
  return clipping.difference([sweep], free).every((polygon) =>
    polygon.every(
      (ring) =>
        Math.abs(
          ring.reduce((sum, p, index) => {
            const q = ring[(index + 1) % ring.length]!;
            return sum + p[0] * q[1] - p[1] * q[0];
          }, 0),
        ) < 1e-7,
    ),
  );
}

/** Check every potentially active solid while retaining inactive receiving planes. */
export function jumpFlightObstacles(
  geometry: Pick<CompiledAssetGeometry, "sight_obstacles" | "movement_transitions">,
) {
  const changing = new Set(
    (geometry.movement_transitions ?? []).flatMap((transition) => [
      ...(transition.initial_sight ?? []),
      ...(transition.applied_sight ?? []),
    ]),
  );
  return geometry.sight_obstacles.map((receiver, index) =>
    changing.has(index) ? { ...receiver, initial_active: true } : receiver,
  );
}
