import type { Point } from "./level.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { clipHeight } from "./gameplay-plane.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";

/**
 * Recover a convex floor whose projection cannot fit a native actor. Native
 * movement erodes the emitted polygon by its screen-space footprint; compensate
 * for the floor projection so this leaves only physically supported centers.
 * Holes, obstacles and bent floors must be handled separately by the caller.
 */
export function projectStairClearance(boundary: Point[], plane: HeightPlane): Point[] | undefined {
  const [a, b, c] = plane;
  if (Math.abs(1 - b) < 1e-6 || boundary.length < 3) return undefined;
  const projected: Point[] = boundary.map(([x, y]) => [x, y - a * x - b * y - c]);
  const area = projected.reduce((sum, p, i) => {
    const q = projected[(i + 1) % projected.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  if (Math.abs(area) < 1e-6) return undefined;
  const sign = Math.sign(area);
  const lines: [number, number, number][] = [];
  let existingCenters = projected;
  let changed = false;
  for (const [i, p] of projected.entries()) {
    const q = projected[(i + 1) % projected.length]!;
    const length = Math.hypot(q[0] - p[0], q[1] - p[1]);
    if (length < 1e-8) continue;
    const nx = (sign * (q[1] - p[1])) / length;
    const ny = (sign * (p[0] - q[0])) / length;
    const limit = nx * p[0] + ny * p[1];
    if (projected.some(([x, y]) => nx * x + ny * y > limit + 1e-6)) return undefined;
    existingCenters = clipHeight(existingCenters, [
      -nx,
      -ny,
      limit - Math.abs(nx) * half[0] - Math.abs(ny) * half[1],
    ]);
    // Apply the same one-unit movement inset before and after projection.
    // Endpoint boxes are larger, and therefore impose additional clearance.
    const physicalSupport =
      Math.abs(nx - a * ny) * (half[0] - 1) + Math.abs((1 - b) * ny) * (half[1] - 1);
    const nativeSupport = Math.abs(nx) * (half[0] - 1) + Math.abs(ny) * (half[1] - 1);
    const correction = nativeSupport - physicalSupport;
    changed ||= Math.abs(correction) > 1e-6;
    // Rounding each coordinate by at most half a unit must not expand the
    // physically supported center region. Keep a further floating-point margin.
    lines.push([nx, ny, limit + correction - 0.5001 * (Math.abs(nx) + Math.abs(ny))]);
  }
  if (!changed || lines.length < 3) return undefined;
  // Keep established passage endpoints and receiving-boundary crossings when
  // the native floor already has room for an actor. Re-preparing those routes
  // is separate from recovering projection-collapsed corridors.
  const existingArea = existingCenters.reduce((sum, p, i) => {
    const q = existingCenters[(i + 1) % existingCenters.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  if (Math.abs(existingArea) > 1e-6) return undefined;
  const result: Point[] = [];
  for (const [i, first] of lines.entries()) {
    const second = lines[(i + 1) % lines.length]!;
    const determinant = first[0] * second[1] - first[1] * second[0];
    if (Math.abs(determinant) < 1e-8) return undefined;
    const point: Point = [
      (first[2] * second[1] - first[1] * second[2]) / determinant,
      (first[0] * second[2] - first[2] * second[0]) / determinant,
    ];
    if (lines.some(([x, y, limit]) => x * point[0] + y * point[1] > limit + 1e-6)) return undefined;
    result.push([Math.round(point[0]), Math.round(point[1])]);
  }
  if (
    result.some((point) =>
      point.some((value) => !Number.isFinite(value) || value < -32768 || value > 32767),
    )
  )
    throw new Error("Stair clearance projection exceeds the game coordinate range");
  return result;
}
