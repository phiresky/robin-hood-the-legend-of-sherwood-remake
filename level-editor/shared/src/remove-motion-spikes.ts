import type { Point } from "./level.ts";

/** Remove backtracking slits below the precision of authored placement seams.
 * Keep all other vertices, including narrow corners and fractional boundaries.
 */
export function removeMotionSpikes(points: Point[]): Point[] {
  const tolerance = 0.0001;
  const result = points.map((point): Point => [...point]);
  let changed = true;
  while (changed && result.length > 3) {
    changed = false;
    for (let i = 0; i < result.length; i++) {
      const previous = (i + result.length - 1) % result.length;
      const next = (i + 1) % result.length;
      const a = result[previous]!;
      const b = result[i]!;
      const c = result[next]!;
      if (
        Math.hypot(a[0] - c[0], a[1] - c[1]) <= tolerance &&
        Math.hypot(a[0] - b[0], a[1] - b[1]) > tolerance
      ) {
        // Retain the preceding endpoint instead of moving the surrounding edges.
        for (const index of [i, next].sort((left, right) => right - left)) result.splice(index, 1);
        changed = true;
        break;
      }
    }
  }
  return result.length >= 3 ? result : points.map((point): Point => [...point]);
}
