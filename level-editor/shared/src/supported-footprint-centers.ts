import clipping, { type MultiPolygon, type Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";

function convexHull(points: Point[]): Point[] {
  const sorted = [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const turn = (a: Point, b: Point, c: Point) =>
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const chain = (points: Point[]): Point[] => {
    const result: Point[] = [];
    for (const point of points) {
      while (result.length > 1 && turn(result.at(-2)!, result.at(-1)!, point) <= 0) result.pop();
      result.push(point);
    }
    return result.slice(0, -1);
  };
  return [...chain(sorted), ...chain([...sorted].reverse())];
}

/** Exact box support across joined footprints, retaining holes and disconnected pieces. */
export function supportedFootprintCenters(
  centers: MultiPolygon,
  supports: Polygon[],
  half: Readonly<Point>,
): MultiPolygon {
  if (half.some((value) => !Number.isFinite(value) || value <= 0))
    throw new Error("Footprint support requires finite positive half extents");
  if (!centers.length || !supports.length) return [];
  const joined = clipping.union(supports[0]!, ...supports.slice(1));
  let result = clipping.intersection(centers, joined);
  if (!result.length) return [];
  const points = result.flat(2);
  const left = Math.min(...points.map(([x]) => x));
  const right = Math.max(...points.map(([x]) => x));
  const top = Math.min(...points.map(([, y]) => y));
  const bottom = Math.max(...points.map(([, y]) => y));
  const offsets: Point[] = [
    [-half[0], -half[1]],
    [half[0], -half[1]],
    [half[0], half[1]],
    [-half[0], half[1]],
  ];
  // A center inside the union has complete box support exactly when its box
  // crosses none of the union's boundary edges, including hole boundaries.
  // Testing corners alone would miss holes or notches lying inside the box.
  for (const polygon of joined)
    for (const ring of polygon)
      for (let i = 1; i < ring.length; i++) {
        const a = ring[i - 1]!,
          b = ring[i]!;
        if (
          Math.max(a[0], b[0]) + half[0] < left ||
          Math.min(a[0], b[0]) - half[0] > right ||
          Math.max(a[1], b[1]) + half[1] < top ||
          Math.min(a[1], b[1]) - half[1] > bottom
        )
          continue;
        const sweep = convexHull(
          [a, b].flatMap(([x, y]) => offsets.map(([dx, dy]): Point => [x + dx, y + dy])),
        );
        result = clipping.difference(result, [sweep]);
        if (!result.length) return [];
      }
  return result;
}
