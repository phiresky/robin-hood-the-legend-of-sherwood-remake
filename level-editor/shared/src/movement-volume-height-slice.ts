import type { Point } from "./level.ts";
import { clipHeight, planeHeight, type HeightPlane } from "./gameplay-plane.ts";

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
    floor.map((value, index) => value - bottom[index]!) as HeightPlane,
  );
}
