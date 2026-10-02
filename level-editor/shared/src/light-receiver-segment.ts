import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

/** Intersect a finite authored world-space segment with a projected receiving plane. */
export function lightReceiverIntersection(
  segment: [Vec3, Vec3],
  plane: HeightPlane,
  label = "Light",
): Vec3 | undefined {
  const [a, b] = segment;
  const distance = (point: Vec3) => point[2] - planeHeight(plane, [point[0], point[1] - point[2]]);
  const da = distance(a),
    db = distance(b);
  if (Math.abs(da - db) < 1e-8) {
    if (Math.abs(da) < 1e-4)
      throw new Error(`${label} receiving segment lies in a receiving plane`);
    return undefined;
  }
  const t = da / (da - db);
  if (t < 0 || t > 1) return undefined;
  return [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2])];
}
