import type { Point } from "./level.ts";
import { clipHeight, planeHeight, type HeightPlane } from "./gameplay-plane.ts";

/** Generated spline coordinates retain 1/1024-unit horizontal precision. */
export const MOVEMENT_CONTACT_TOLERANCE = 1 / 1024;

/** Convex height mask in world XY; callers intersect it with the actual footprint and holes. */
export function movementVolumeHeightSlice(
  footprint: Point[],
  floor: HeightPlane,
  bottom: HeightPlane,
  top: HeightPlane,
  includeTopContact = false,
): Point[] {
  const above = top.map((value, index) => value - floor[index]!) as HeightPlane;
  // A solid supporting the floor from below is not an obstacle on that floor.
  if (!includeTopContact && footprint.every((point) => planeHeight(above, point) <= 1e-7))
    return [];
  const xs = footprint.map(([x]) => x),
    ys = footprint.map(([, y]) => y);
  const minX = Math.min(...xs),
    maxX = Math.max(...xs);
  const minY = Math.min(...ys),
    maxY = Math.max(...ys);
  let below = floor.map((value, index) => value - bottom[index]!) as HeightPlane;
  // A nominally touching sloped underside must not become an underpass through
  // independent coordinate rounding. Do not extend solids across larger gaps.
  if (footprint.every((point) => Math.abs(planeHeight(below, point)) <= MOVEMENT_CONTACT_TOLERANCE))
    below = [0, 0, 0];
  return clipHeight(
    clipHeight(
      [
        [minX, minY],
        [maxX, minY],
        [maxX, maxY],
        [minX, maxY],
      ],
      above,
    ),
    below,
  );
}
