import type { Vec3 } from "./scene.ts";

/** Clip a source polyline without connecting separate surviving fragments. */
export function clipSplinePolyline(
  points: Vec3[],
  axis: number,
  min: number,
  max: number,
  stations: number[],
): Vec3[][] {
  if (points.length === 1)
    return points[0]![axis]! >= min && points[0]![axis]! <= max ? [[points[0]!]] : [];
  const result: Vec3[][] = [];
  let previousEnd = false;
  for (let i = 1; i < points.length; i++) {
    const a = points[i - 1]!,
      b = points[i]!,
      delta = b[axis]! - a[axis]!;
    const low =
      delta === 0 ? 0 : Math.max(0, Math.min((min - a[axis]!) / delta, (max - a[axis]!) / delta));
    const high =
      delta === 0 ? 1 : Math.min(1, Math.max((min - a[axis]!) / delta, (max - a[axis]!) / delta));
    if (high < low || (delta === 0 && (a[axis]! < min || a[axis]! > max))) {
      previousEnd = false;
      continue;
    }
    const cuts = [
      low,
      ...(delta === 0
        ? []
        : stations.map((s) => (s - a[axis]!) / delta).filter((t) => t > low && t < high)),
      high,
    ].sort((x, y) => x - y);
    const clipped = [...new Set(cuts)].map((t): Vec3 => [
      a[0] + (b[0] - a[0]) * t,
      a[1] + (b[1] - a[1]) * t,
      a[2] + (b[2] - a[2]) * t,
    ]);
    if (previousEnd && low === 0) result.at(-1)!.push(...clipped.slice(1));
    else result.push(clipped);
    previousEnd = high === 1;
  }
  return result;
}
