import type { Point } from "./level.ts";
import { removeMotionSpikes } from "./remove-motion-spikes.ts";
import { serializePreciseMotionRing } from "./serialize-precise-motion-ring.ts";

/** Clean numeric floor seams only when the prepared integer boundary is preserved. */
export function preparePreciseFloor(exact: Point[], grid: Point[]): Point[] | undefined {
  return (
    serializePreciseMotionRing(removeMotionSpikes(exact), grid) ??
    serializePreciseMotionRing(exact, grid)
  );
}
