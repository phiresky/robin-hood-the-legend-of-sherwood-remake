import type { Point } from "./level.ts";
import { clipHeight } from "./gameplay-plane.ts";

export function stairBoundaryPlanes(points: Point[]): [number, number, number][] {
  const area = points.reduce((sum, p, i) => {
    const q = points[(i + 1) % points.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  if (Math.abs(area) < 1e-8) return [];
  const sign = Math.sign(area);
  return points.flatMap((p, i) => {
    const q = points[(i + 1) % points.length]!;
    const length = Math.hypot(q[0] - p[0], q[1] - p[1]);
    if (length < 1e-8) return [];
    const x = (sign * (p[1] - q[1])) / length;
    const y = (sign * (q[0] - p[0])) / length;
    return [[x, y, -x * p[0] - y * p[1]]];
  });
}

/** Intersect inward edge half-planes; a nonconvex floor yields its inner kernel. */
export function stairCenterRegion(points: Point[], half: Readonly<Point>): Point[] {
  const planes = stairBoundaryPlanes(points);
  if (planes.length < 3) return [];
  const xs = points.map((p) => p[0]),
    ys = points.map((p) => p[1]);
  let result: Point[] = [
    [Math.min(...xs), Math.min(...ys)],
    [Math.max(...xs), Math.min(...ys)],
    [Math.max(...xs), Math.max(...ys)],
    [Math.min(...xs), Math.max(...ys)],
  ];
  for (const [x, y, c] of planes)
    result = clipHeight(result, [x, y, c - Math.abs(x) * half[0] - Math.abs(y) * half[1]]);
  result = result.filter(
    (p, i) =>
      Math.hypot(
        p[0] - result[(i + 1) % result.length]![0],
        p[1] - result[(i + 1) % result.length]![1],
      ) > 1e-8,
  );
  return result.filter((p, i) => {
    const a = result[(i + result.length - 1) % result.length]!,
      b = result[(i + 1) % result.length]!;
    return (
      Math.abs((p[0] - a[0]) * (b[1] - a[1]) - (p[1] - a[1]) * (b[0] - a[0])) >
      1e-7 * Math.hypot(b[0] - a[0], b[1] - a[1])
    );
  });
}
