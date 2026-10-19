import clipping from "polygon-clipping";
import type { Point } from "./level.ts";
import { normalizeRoundedMotionRing, simplifyMotionRing } from "./motion-quantization.ts";

/** The serialized precision contour is cyclic, but repairing a crossed grid
 * corner can depend on its starting vertex. Select an equivalent ordering whose
 * rounded contour reproduces the already prepared navigation polygon. */
export function serializePreciseMotionRing(exact: Point[], grid: Point[]): Point[] | undefined {
  for (let start = 0; start < exact.length; start++) {
    const candidate = [...exact.slice(start), ...exact.slice(0, start)];
    const rounded = candidate.map(([x, y]): Point => [Math.floor(x + 0.5), Math.floor(y + 0.5)]);
    // A repeated closure is removed by the loader's cyclic collinearity pass,
    // which removes the first vertex, not the last. Do that before the shared
    // normalization so both representations choose the same repaired corner.
    while (
      rounded.length > 2 &&
      rounded[0]![0] === rounded.at(-1)![0] &&
      rounded[0]![1] === rounded.at(-1)![1]
    )
      rounded.shift();
    const normalized = normalizeRoundedMotionRing(simplifyMotionRing(rounded));
    if (normalized.length >= 3 && clipping.xor([normalized], [grid]).length === 0) return candidate;
  }
  return undefined;
}
