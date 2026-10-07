import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

/** z = ax + by + c in whichever coordinate frame the caller supplies. */
export type HeightPlane = [number, number, number];
export const planeHeight = (plane: HeightPlane, point: Point) =>
  plane[0] * point[0] + plane[1] * point[1] + plane[2];

/** Store a projected receiving plane independently of clipped polygon order. */
export function projectionPlaneAnchors(points: Point[], plane: HeightPlane): [Vec3, Vec3, Vec3] {
  const xs = points.map(([x]) => x),
    ys = points.map(([, y]) => y),
    minX = Math.min(...xs),
    maxX = Math.max(...xs),
    minY = Math.min(...ys),
    maxY = Math.max(...ys);
  if (!(maxX > minX && maxY > minY))
    throw new Error("Receiving polygon needs nonzero projected bounds");
  const world = (x: number, y: number): Vec3 => {
    const z = planeHeight(plane, [x, y]);
    return [x, y + z, z];
  };
  // A clipping operation can introduce three nearly collinear leading
  // vertices. Bounding corners retain both spans even for very thin pieces;
  // the anchors define a plane, not additional receiving coverage.
  return [world(minX, minY), world(maxX, minY), world(minX, maxY)];
}

export function heightPlane(points: Vec3[], requirePlanar = true): HeightPlane {
  const a = points[0]!;
  for (let i = 1; i + 1 < points.length; i++) {
    const b = points[i]!,
      c = points[i + 1]!;
    const det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
    if (Math.abs(det) < 1e-8) continue;
    const dx = ((b[2] - a[2]) * (c[1] - a[1]) - (c[2] - a[2]) * (b[1] - a[1])) / det;
    const dy = ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])) / det;
    const plane: HeightPlane = [dx, dy, a[2] - dx * a[0] - dy * a[1]];
    if (
      requirePlanar &&
      points.some((p) => Math.abs(planeHeight(plane, [p[0], p[1]]) - p[2]) > 1e-4)
    )
      throw new Error("Walkable surface must be planar; split it into planar asset surfaces");
    return plane;
  }
  throw new Error("Gameplay surface has no nondegenerate height plane");
}

/** Clip a convex polygon against a linear height inequality. */
export function clipHeight(points: Point[], plane: HeightPlane): Point[] {
  const result: Point[] = [];
  for (let i = 0; i < points.length; i++) {
    const a = points[i]!,
      b = points[(i + 1) % points.length]!;
    const da = planeHeight(plane, a),
      db = planeHeight(plane, b);
    if (da >= 0) result.push(a);
    if (da >= 0 !== db >= 0) {
      const t = da / (da - db);
      result.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
    }
  }
  return result;
}
