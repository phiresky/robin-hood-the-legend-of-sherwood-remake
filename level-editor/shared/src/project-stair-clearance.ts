import type { Point } from "./level.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { clipHeight } from "./gameplay-plane.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import clipping from "polygon-clipping";
import { stairBoundaryPlanes, stairCenterRegion } from "./stair-center-region.ts";
import { simplifyMotionRing } from "./motion-quantization.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

/**
 * Recover a floor whose projection cannot fit a native actor. Native
 * movement erodes the emitted polygon by its screen-space footprint; compensate
 * for the floor projection so this leaves only physically supported centers.
 * A collapsed concave outline may use its conservative inner kernel.
 * Holes, obstacles and bent floors must be handled separately by the caller.
 */
export function projectStairClearance(
  boundary: Point[],
  plane: HeightPlane,
  preservePrecision = false,
  options: { movementHalf?: Readonly<Point>; requiredCenters?: Point[] } = {},
): Point[] | undefined {
  // Clipping can retain a nearly straight intermediate vertex. Remove noise
  // within two clipping-grid units before intersecting offset edge planes.
  boundary = simplifyMotionRing(boundary, 2 / 1048576);
  const [a, b, c] = plane;
  const movementHalf = options.movementHalf ?? [half[0] - 1, half[1] - 1];
  if (Math.abs(1 - b) < 1e-6 || boundary.length < 3) return undefined;
  let projected: Point[] = boundary.map(([x, y]) => [x, y - a * x - b * y - c]);
  const planes = stairBoundaryPlanes(projected);
  if (planes.some(([x, y, c]) => projected.some((p) => x * p[0] + y * p[1] + c < -1e-6))) {
    // Corner containment overestimates the space for a box in a concave floor.
    // If even that region is empty, no existing full-box approach can be lost.
    const shifted = [
      [-half[0], -half[1]],
      [-half[0], half[1]],
      [half[0], -half[1]],
      [half[0], half[1]],
    ].map(([dx, dy]) => [projected.map(([x, y]): Point => [x + dx!, y + dy!])]);
    if (clipping.intersection(shifted[0]!, ...shifted.slice(1)).length) return undefined;
    projected = stairCenterRegion(projected, [0, 0]);
    if (projected.length < 3) return undefined;
  }
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
    // Walking uses its one-unit inset; climbing requires the full native box.
    // Use the same footprint before and after projection.
    const physicalSupport =
      Math.abs(nx - a * ny) * movementHalf[0] + Math.abs((1 - b) * ny) * movementHalf[1];
    const nativeSupport = Math.abs(nx) * movementHalf[0] + Math.abs(ny) * movementHalf[1];
    const correction = nativeSupport - physicalSupport;
    changed ||= Math.abs(correction) > 1e-6;
    // Integer output reserves half a unit per coordinate for rounding. Precise
    // output needs only a margin for native binary32 collision coordinates.
    const margin = preservePrecision ? 0.001 : 0.5001;
    lines.push([nx, ny, limit + correction - margin * (Math.abs(nx) + Math.abs(ny))]);
  }
  if (!changed || lines.length < 3) return undefined;
  // Keep established passage endpoints and receiving-boundary crossings when
  // the native floor already has room for an actor. Re-preparing those routes
  // is separate from recovering projection-collapsed corridors.
  const existingArea = existingCenters.reduce((sum, p, i) => {
    const q = existingCenters[(i + 1) % existingCenters.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  const centerPlanes = stairBoundaryPlanes(existingCenters);
  if (
    Math.abs(existingArea) > 1e-6 &&
    (options.requiredCenters ?? []).every((p) =>
      centerPlanes.every(([x, y, c]) => x * p[0] + y * p[1] + c >= -1e-6),
    )
  )
    return undefined;
  const result: Point[] = [];
  let redundantEdge = false;
  for (const [i, first] of lines.entries()) {
    const second = lines[(i + 1) % lines.length]!;
    const determinant = first[0] * second[1] - first[1] * second[0];
    if (Math.abs(determinant) < 1e-8) {
      redundantEdge = true;
      break;
    }
    const point: Point = [
      (first[2] * second[1] - first[1] * second[2]) / determinant,
      (first[0] * second[2] - first[2] * second[0]) / determinant,
    ];
    if (lines.some(([x, y, limit]) => x * point[0] + y * point[1] > limit + 1e-6)) {
      redundantEdge = true;
      break;
    }
    result.push(point);
  }
  // Offset planes can form arbitrarily long miters at acute projected corners.
  // Supported centers lie inside the projected floor, so their native boxes
  // fit inside its bounds expanded by the footprint. Clip excess miters before
  // quantization: they otherwise become distant walkable/clickable stair sectors.
  const xs = projected.map(([x]) => x);
  const ys = projected.map(([, y]) => y);
  let bounded = result;
  if (redundantEdge) {
    // Offsetting a short chamfer can make its constraint redundant. Intersect
    // all constraints instead of requiring each adjacent pair to be a vertex.
    bounded = [
      [Math.min(...xs) - half[0], Math.min(...ys) - half[1]],
      [Math.max(...xs) + half[0], Math.min(...ys) - half[1]],
      [Math.max(...xs) + half[0], Math.max(...ys) + half[1]],
      [Math.min(...xs) - half[0], Math.max(...ys) + half[1]],
    ];
    for (const [x, y, limit] of lines) bounded = clipHeight(bounded, [-x, -y, limit]);
    if (bounded.length < 3) return undefined;
  }
  for (const limit of [
    [1, 0, -Math.min(...xs) + half[0]],
    [-1, 0, Math.max(...xs) + half[0]],
    [0, 1, -Math.min(...ys) + half[1]],
    [0, -1, Math.max(...ys) + half[1]],
  ] satisfies HeightPlane[])
    bounded = clipHeight(bounded, limit);
  const output = preservePrecision
    ? bounded
    : bounded.map(([x, y]): Point => [Math.round(x), Math.round(y)]);
  if (
    output.some((point) =>
      point.some((value) => !Number.isFinite(value) || value < -32768 || value > 32767),
    )
  )
    throw new Error("Stair clearance projection exceeds the game coordinate range");
  // Preparing footprint clearance must not strand an authored entry inside the
  // original floor. Climbing transitions use those entry points directly.
  if (
    options.requiredCenters?.some(
      (point) => pointInGameplayPolygon(point, projected) && !pointInGameplayPolygon(point, output),
    )
  )
    return undefined;
  return output;
}
