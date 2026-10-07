import earcut from "earcut";
import type { Point, SightObstacle } from "./level.ts";

const cross = (a: Point, b: Point, c: Point) =>
  (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);

function hasArea(points: Point[]): boolean {
  const a = points[0];
  if (!a) return false;
  const b = points.find((p) => p[0] !== a[0] || p[1] !== a[1]);
  return !!b && points.some((p) => cross(a, b, p) !== 0);
}

function needsSplit(points: Point[]): boolean {
  for (let i = 0; i < points.length; i++) {
    const a = points[i]!,
      b = points[(i + 1) % points.length]!;
    const previous = points[(i + points.length - 1) % points.length]!;
    if (a[0] === b[0] && a[1] === b[1]) return true;
    if (
      cross(previous, a, b) === 0 &&
      (previous[0] - a[0]) * (b[0] - a[0]) + (previous[1] - a[1]) * (b[1] - a[1]) > 0
    )
      return true;
    for (let j = i + 2; j < points.length; j++) {
      if (i === 0 && j === points.length - 1) continue;
      const c = points[j]!,
        d = points[(j + 1) % points.length]!;
      if (
        Math.max(a[0], b[0]) < Math.min(c[0], d[0]) ||
        Math.max(c[0], d[0]) < Math.min(a[0], b[0]) ||
        Math.max(a[1], b[1]) < Math.min(c[1], d[1]) ||
        Math.max(c[1], d[1]) < Math.min(a[1], b[1])
      )
        continue;
      if (cross(a, b, c) * cross(a, b, d) <= 0 && cross(c, d, a) * cross(c, d, b) <= 0) return true;
    }
  }
  return false;
}

/** Preserve generated receiving geometry through the native binary32 conversion.
 * Authored physical volumes use their own validation and are never repaired here. */
export function nativeReceiverGeometry(
  points: SightObstacle["points"],
  label: string,
  warnings: string[],
): SightObstacle["points"][] {
  const native = points.map(({ x, y }): Point => [Math.fround(x), Math.fround(y)]);
  if (!native.every((point) => point.every(Number.isFinite)))
    throw new Error(`${label}: receiving geometry exceeds native coordinate range`);
  const collapsed = () =>
    warnings.push(
      `${label} collapsed to zero area at native coordinate precision and was omitted.`,
    );
  if (!hasArea(native)) {
    collapsed();
    return [];
  }
  const first = native[0]!,
    last = native.at(-1)!;
  const ring = first[0] === last[0] && first[1] === last[1] ? native.slice(0, -1) : native;
  if (!needsSplit(ring)) return [points];
  // Triangulate before rounding: each surviving triangle stays simple when its
  // vertices are converted, including narrow notches which pinch shut natively.
  const indices = earcut(points.flatMap(({ x, y }) => [x, y]));
  if (!indices.length) throw new Error(`${label}: cannot triangulate native receiving geometry`);
  warnings.push(
    `${label} was triangulated to preserve simple boundaries at native coordinate precision.`,
  );
  const result: SightObstacle["points"][] = [];
  for (let i = 0; i < indices.length; i += 3) {
    const triangle = indices.slice(i, i + 3);
    if (!hasArea(triangle.map((index) => native[index]!))) {
      collapsed();
      continue;
    }
    result.push(triangle.map((index) => points[index]!));
  }
  return result;
}
