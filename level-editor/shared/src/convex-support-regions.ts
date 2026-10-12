import earcut from "earcut";
import clipping from "polygon-clipping";
import type { Point } from "./level.ts";
import { footprintBoundaryHull } from "./supported-footprint-centers.ts";

function area(points: Point[]): number {
  if (points.length < 3) return 0;
  const [x, y] = points[0]!;
  return (
    Math.abs(
      points.reduce((sum, p, i) => {
        const q = points[(i + 1) % points.length]!;
        return sum + (p[0] - x) * (q[1] - y) - (p[1] - y) * (q[0] - x);
      }, 0),
    ) / 2
  );
}

/** Convex candidates inside a simple support ring, without filling its notches. */
export function convexSupportRegions(boundary: Point[]): Point[][] {
  const indices = earcut(boundary.flat());
  const triangles: Point[][] = [];
  for (let i = 0; i < indices.length; i += 3)
    triangles.push(indices.slice(i, i + 3).map((index) => boundary[index]!));
  if (!triangles.length) return [];
  const pieces: Point[][] = [];
  for (const triangle of triangles) {
    let region = triangle;
    for (;;) {
      let best = region;
      let bestArea = area(region);
      for (const other of triangles) {
        const hull = footprintBoundaryHull([...region, ...other]);
        const size = area(hull);
        if (size <= bestArea + 1e-7) continue;
        const outside = clipping.difference([hull], [boundary]);
        if (outside.reduce((sum, polygon) => sum + area(polygon[0]!), 0) > 1e-7) continue;
        best = hull;
        bestArea = size;
      }
      if (best === region) break;
      region = best;
    }
    if (!pieces.some((piece) => clipping.xor([piece], [region]).length === 0)) pieces.push(region);
  }
  return pieces.sort((a, b) => area(b) - area(a));
}
