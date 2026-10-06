import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

export interface NavigationAnchorArea {
  plane: HeightPlane;
  polygon: Point[];
  blockers: Point[][];
  /** Ordinary movement contours use projected coordinates. Physical floors use world XY. */
  coordinateSpace?: "projected" | "world";
}

/** Receiving contours pass through the fixed clipping grid before binding. */
export function onClippedReceivingBoundary(point: Point, polygon: Point[]) {
  const tolerance = 2 / 1048576;
  return polygon.some((a, index) => {
    const b = polygon[(index + 1) % polygon.length]!;
    const dx = b[0] - a[0],
      dy = b[1] - a[1];
    const lengthSquared = dx * dx + dy * dy;
    if (lengthSquared === 0) return false;
    const t = Math.max(
      0,
      Math.min(1, ((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / lengthSquared),
    );
    return Math.hypot(point[0] - a[0] - t * dx, point[1] - a[1] - t * dy) <= tolerance;
  });
}

export function pointInGameplayPolygon(point: Point, polygon: Point[], includeBoundary = false) {
  let hit = false;
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const a = polygon[i]!,
      b = polygon[j]!;
    if (includeBoundary) {
      const cross = (point[0] - a[0]) * (b[1] - a[1]) - (point[1] - a[1]) * (b[0] - a[0]);
      if (
        Math.abs(cross) <= 1e-6 &&
        point[0] >= Math.min(a[0], b[0]) - 1e-6 &&
        point[0] <= Math.max(a[0], b[0]) + 1e-6 &&
        point[1] >= Math.min(a[1], b[1]) - 1e-6 &&
        point[1] <= Math.max(a[1], b[1]) + 1e-6
      )
        return true;
    }
    if (
      a[1] > point[1] !== b[1] > point[1] &&
      point[0] < ((b[0] - a[0]) * (point[1] - a[1])) / (b[1] - a[1]) + a[0]
    )
      hit = !hit;
  }
  return hit;
}

export function navigationAnchorHeight(area: NavigationAnchorArea, point: Vec3) {
  return planeHeight(area.plane, [
    point[0],
    area.coordinateSpace === "world" ? point[1] : point[1] - point[2],
  ]);
}

/** Resolve against the authored coordinate frame; screen coincidence is not physical support. */
export function containsNavigationAnchor(
  area: NavigationAnchorArea,
  point: Vec3,
  options: { projected?: Point; allowBlocked?: boolean; requireHeight?: boolean } = {},
) {
  if (
    options.requireHeight !== false &&
    !(Math.abs(navigationAnchorHeight(area, point) - point[2]) < 1e-4)
  )
    return false;
  const physical = area.coordinateSpace === "world";
  const position: Point = physical
    ? [point[0], point[1]]
    : (options.projected ?? [point[0], point[1] - point[2]]);
  return (
    pointInGameplayPolygon(position, area.polygon, physical) &&
    (options.allowBlocked === true ||
      !area.blockers.some((blocker) => pointInGameplayPolygon(position, blocker, physical)))
  );
}
