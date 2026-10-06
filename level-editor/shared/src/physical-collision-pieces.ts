import earcut, { deviation } from "earcut";
import type { Point } from "./level.ts";

function simpleRing(points: Point[]): boolean {
  const cross = (a: Point, b: Point, c: Point) =>
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const on = (a: Point, b: Point, c: Point) =>
    cross(a, b, c) === 0 &&
    c[0] >= Math.min(a[0], b[0]) &&
    c[0] <= Math.max(a[0], b[0]) &&
    c[1] >= Math.min(a[1], b[1]) &&
    c[1] <= Math.max(a[1], b[1]);
  const origin = points[0];
  if (!origin || points.length < 3) return false;
  let area = 0;
  for (const [i, a] of points.entries()) {
    const b = points[(i + 1) % points.length]!;
    if (a.some((value) => !Number.isFinite(value)) || (a[0] === b[0] && a[1] === b[1]))
      return false;
    area += cross(origin, a, b);
    for (let j = i + 2; j < points.length; j++) {
      if (i === 0 && j === points.length - 1) continue;
      const c = points[j]!,
        d = points[(j + 1) % points.length]!;
      if (
        (cross(a, b, c) * cross(a, b, d) < 0 && cross(c, d, a) * cross(c, d, b) < 0) ||
        on(a, b, c) ||
        on(a, b, d) ||
        on(c, d, a) ||
        on(c, d, b)
      )
        return false;
    }
  }
  return Number.isFinite(area) && Math.abs(area) > 1e-9;
}

/** Preserve collision when a narrow contour folds over itself at runtime precision. */
export function physicalCollisionPieces(polygon: Point[]): Point[][] {
  const runtime = (ring: Point[]): Point[] =>
    ring.map(([x, y]) => [Math.fround(x), Math.fround(y)]);
  if (simpleRing(runtime(polygon))) return [polygon];
  if (!simpleRing(polygon)) throw new Error("Physical collision contour is not simple");
  const vertices = polygon.flat();
  const indices = earcut(vertices);
  if (!indices.length || deviation(vertices, [], 2, indices) > 1e-8)
    throw new Error("Physical collision partition does not preserve coverage");
  const pieces: Point[][] = [];
  for (let i = 0; i < indices.length; i += 3) {
    const triangle = indices.slice(i, i + 3).map((index) => polygon[index]!);
    if (!simpleRing(runtime(triangle)))
      throw new Error("Physical collision piece cannot retain area at runtime precision");
    pieces.push(triangle);
  }
  return pieces;
}
