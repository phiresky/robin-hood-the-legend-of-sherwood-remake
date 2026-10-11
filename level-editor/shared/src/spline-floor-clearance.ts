import { clipHeight, heightPlane, planeHeight } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

/** Map an authored clearance through an existing floor triangle's affine frame. */
export function splineFloorClearance(
  clearance: Vec3[],
  source: Vec3[],
  placed: Vec3[],
): { source: Point; placed: Vec3 }[] {
  const plane = heightPlane(source);
  if (clearance.some(([x, y, z]) => Math.abs(planeHeight(plane, [x, y]) - z) > 1e-6)) return [];
  const a = source[0]!,
    b = source[1]!,
    c = source[2]!;
  const determinant = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
  const winding = Math.sign(determinant);
  let clipped: Point[] = clearance.map(([x, y]) => [x, y]);
  for (let i = 0; i < 3 && clipped.length; i++) {
    const from = source[i]!,
      to = source[(i + 1) % 3]!;
    const dx = to[0] - from[0],
      dy = to[1] - from[1];
    clipped = clipHeight(clipped, [
      -dy * winding,
      dx * winding,
      (dy * from[0] - dx * from[1]) * winding,
    ]);
  }
  return clipped.map(([x, y]) => {
    const u = ((x - a[0]) * (c[1] - a[1]) - (y - a[1]) * (c[0] - a[0])) / determinant;
    const v = ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])) / determinant;
    const interpolate = (axis: number) =>
      placed[0]![axis]! +
      u * (placed[1]![axis]! - placed[0]![axis]!) +
      v * (placed[2]![axis]! - placed[0]![axis]!);
    return { source: [x, y], placed: [interpolate(0), interpolate(1), interpolate(2)] };
  });
}
