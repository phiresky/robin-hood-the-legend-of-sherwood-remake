import type { Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";

/** Remove duplicate closure, straight-edge vertices and zero-width backtracking spikes. */
export function simplifyMotionRing(points: Point[], distanceTolerance = 0): Point[] {
  const result = points.map((p): Point => [...p]);
  if (
    result.length > 1 &&
    result[0]![0] === result.at(-1)![0] &&
    result[0]![1] === result.at(-1)![1]
  )
    result.pop();
  let changed = true;
  while (changed && result.length >= 3) {
    changed = false;
    for (let i = 0; i < result.length; i++) {
      const a = result[(i + result.length - 1) % result.length]!,
        b = result[i]!,
        c = result[(i + 1) % result.length]!;
      // Backtracking spikes have almost coincident endpoints. Measure their
      // width against the longest edge, rather than the tiny endpoint gap.
      if (
        Math.abs((b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])) <
        1e-8 +
          distanceTolerance *
            Math.max(
              Math.hypot(c[0] - a[0], c[1] - a[1]),
              Math.hypot(b[0] - a[0], b[1] - a[1]),
              Math.hypot(c[0] - b[0], c[1] - b[1]),
            )
      ) {
        result.splice(i, 1);
        changed = true;
        break;
      }
    }
  }
  return result;
}

function collinear(points: Point[]): boolean {
  const a = points[0];
  if (!a) return true;
  const b = points.find((p) => p[0] !== a[0] || p[1] !== a[1]);
  return !b || points.every((p) => (b[0] - a[0]) * (p[1] - a[1]) === (b[1] - a[1]) * (p[0] - a[0]));
}

/** Remove a crossed corner only when its width fits within grid-rounding error.
 * The precise contour remains separate; larger crossings remain invalid. */
export function normalizeRoundedMotionRing(points: Point[]): Point[] {
  let result = simplifyMotionRing(points);
  const cross = (a: Point, b: Point, c: Point) =>
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const removable = (p: Point, a: Point, b: Point) => {
    const dx = b[0] - a[0],
      dy = b[1] - a[1];
    const length = dx * dx + dy * dy;
    return length > 0 && cross(a, b, p) ** 2 <= length / 2;
  };
  for (;;) {
    let removed = false;
    for (let i = 0; i < result.length && result.length >= 4; i++) {
      const a = result[i]!,
        b = result[(i + 1) % result.length]!;
      const c = result[(i + 2) % result.length]!,
        d = result[(i + 3) % result.length]!;
      if (cross(a, b, c) * cross(a, b, d) >= 0 || cross(c, d, a) * cross(c, d, b) >= 0) continue;
      const index = removable(b, a, c)
        ? (i + 1) % result.length
        : removable(c, b, d)
          ? (i + 2) % result.length
          : undefined;
      if (index === undefined) continue;
      result.splice(index, 1);
      result = simplifyMotionRing(result);
      removed = true;
      break;
    }
    if (!removed) return result;
  }
}

/** Only boolean-operation output may lose zero-area rings on the integer grid. */
export function quantizeGeneratedMotionPolygon(
  polygon: Polygon,
  quantize: (value: number) => number,
  label: string,
  warnings: string[],
): Polygon | null {
  if (!polygon.length) throw new Error(`${label}: missing polygon boundary`);
  const result: Polygon = [];
  for (const [index, original] of polygon.entries()) {
    // Clipping may insert a fractional vertex on a straight edge. Rounding
    // that redundant vertex first creates a kink and can open a false seam.
    // Allow two units of the clipping grid for intersection noise, before
    // snapping to whole movement coordinates. Authored rings retain strict cleanup.
    const rounded = simplifyMotionRing(original, 2 / 1048576).map(([x, y]): Point => [
      quantize(x),
      quantize(y),
    ]);
    const normalized = normalizeRoundedMotionRing(rounded);
    // Retain existing vertex/closure conventions when no crossing was repaired.
    const repaired = normalized.length < simplifyMotionRing(rounded).length;
    if (repaired)
      warnings.push(`${label}: repaired a crossed subpixel corner on the integer movement grid.`);
    const points = repaired ? normalized : rounded;
    if (
      points.length &&
      original.length > 1 &&
      original[0]![0] === original.at(-1)![0] &&
      original[0]![1] === original.at(-1)![1]
    )
      points.push([...points[0]!]);
    if (collinear(simplifyMotionRing(points))) {
      warnings.push(
        `${label}: generated ${index === 0 ? "region" : "hole"} collapsed to zero area on the integer movement grid and was omitted.`,
      );
      if (index === 0) return null;
    } else result.push(points);
  }
  return result;
}
