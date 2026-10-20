import clipping from "polygon-clipping";
import type { Point } from "./level.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { stairBoundaryPlanes, stairCenterRegion } from "./stair-center-region.ts";
import { footprintBoundaryHull } from "./supported-footprint-centers.ts";

/** Reserve the native body around climb entries without adding unsupported root positions.
 * Call only for convex, unobstructed floors; receiving geometry remains unchanged. */
export function climbEntryClearance(
  boundary: Point[],
  floor: Point[],
  entries: Point[],
): Point[] | undefined {
  if (!entries.length) return undefined;
  const planes = stairBoundaryPlanes(floor);
  if (
    planes.length < 3 ||
    planes.some(([x, y, c]) => floor.some((p) => x * p[0] + y * p[1] + c < -1e-6)) ||
    entries.some((point) => !pointInGameplayPolygon(point, floor))
  )
    return undefined;
  const corners = entries.flatMap(([x, y]) =>
    [-1, 1].flatMap((sx) =>
      [-1, 1].map((sy): Point => [x + sx * (half[0] + 0.001), y + sy * (half[1] + 0.001)]),
    ),
  );
  if (corners.every((point) => pointInGameplayPolygon(point, boundary, true))) return undefined;
  const expanded = footprintBoundaryHull([...boundary, ...corners]);
  const centers = stairCenterRegion(expanded, half);
  if (centers.length < 3 || entries.some((point) => !pointInGameplayPolygon(point, centers)))
    return undefined;
  // The enlarged collision envelope represents body extent, not extra floor.
  if (clipping.difference([centers], [floor]).length) return undefined;
  return expanded;
}
