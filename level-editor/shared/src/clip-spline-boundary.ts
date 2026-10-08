import clipping from "polygon-clipping";
import type { Vec3 } from "./scene.ts";
import { clipSplinePolyline } from "./clip-spline-polyline.ts";

/** Crop a closed application contour, retaining independent closed components. */
export function clipSplineBoundary(
  points: Vec3[],
  axis: number,
  min: number,
  max: number,
  stations: number[],
): Vec3[][] {
  if (points.length < 3 || max <= min) return [];
  if (points.every((point) => point[axis]! >= min && point[axis]! <= max))
    return clipSplinePolyline([...points, points[0]!], axis, min, max, stations).map((fragment) =>
      fragment.slice(0, -1),
    );
  // Choose a projection containing the crop axis, including vertical contours.
  const candidates = [0, 1, 2].filter((other) => other !== axis);
  const area = (other: number) =>
    Math.abs(
      points.reduce((sum, a, i) => {
        const b = points[(i + 1) % points.length]!;
        return sum + a[axis]! * b[other]! - b[axis]! * a[other]!;
      }, 0),
    );
  const other = candidates.sort((a, b) => area(b) - area(a))[0]!;
  if (area(other) < 1e-9) return [];
  const project = (p: Vec3): [number, number] => [p[axis]!, p[other]!];
  const ring = points.map(project);
  const lo = Math.min(...ring.map((p) => p[1])) - 1;
  const hi = Math.max(...ring.map((p) => p[1])) + 1;
  const polygons = clipping.intersection(
    [ring],
    [
      [
        [min, lo],
        [max, lo],
        [max, hi],
        [min, hi],
      ],
    ],
  );
  const restore = ([x, y]: number[]): Vec3 => {
    // A strip intersection only creates points on original contour edges.
    // Interpolate the third coordinate there, never flattening sloped masks.
    for (let i = 0; i < points.length; i++) {
      const a = points[i]!,
        b = points[(i + 1) % points.length]!;
      const dx = b[axis]! - a[axis]!,
        dy = b[other]! - a[other]!;
      const lengthSquared = dx * dx + dy * dy;
      if (lengthSquared === 0) continue;
      const t = ((x! - a[axis]!) * dx + (y! - a[other]!) * dy) / lengthSquared;
      if (t < -1e-8 || t > 1 + 1e-8) continue;
      if (Math.hypot(a[axis]! + t * dx - x!, a[other]! + t * dy - y!) > 1e-7) continue;
      const result = a.map(
        (value, k) => value + (b[k]! - value) * Math.max(0, Math.min(1, t)),
      ) as Vec3;
      result[axis] = x!;
      result[other] = y!;
      return result;
    }
    throw new Error("Cropped mask boundary could not retain its spatial contour");
  };
  return polygons.flatMap((polygon) => {
    if (polygon.length !== 1) throw new Error("Cropped mask boundary contains unsupported holes");
    const ring = polygon[0]!;
    const closed = ring.slice(0, -1).flatMap((a, index) => {
      const b = ring[index + 1]!;
      const dx = b[0] - a[0],
        dy = b[1] - a[1];
      const lengthSquared = dx * dx + dy * dy;
      // Boolean clipping removes vertices collinear in its 2D projection.
      // They can still carry a bend in the third coordinate; restore those
      // source vertices before interpolating the spline's deformation stations.
      const intermediate = points
        .flatMap((point) => {
          const x = point[axis]!,
            y = point[other]!;
          const t = ((x - a[0]) * dx + (y - a[1]) * dy) / lengthSquared;
          return t > 1e-8 && t < 1 - 1e-8 && Math.hypot(a[0] + t * dx - x, a[1] + t * dy - y) < 1e-7
            ? [{ t, point }]
            : [];
        })
        .sort((a, b) => a.t - b.t);
      return [restore(a), ...intermediate.map(({ point }) => point)];
    });
    closed.push(closed[0]!);
    return clipSplinePolyline(closed, axis, min, max, stations)
      .filter((fragment) => fragment.length >= 4)
      .map((fragment) => fragment.slice(0, -1));
  });
}
