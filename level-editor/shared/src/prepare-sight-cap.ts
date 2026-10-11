import type { SightObstacle } from "./level.ts";
import { heightPlane } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

/** Keep the native first-three-vertex caps usable after placement and clipping. */
export function prepareSightCap(points: SightObstacle["points"]): SightObstacle["points"] {
  const usable = (offset: number) => {
    const [a, b, c] = [0, 1, 2].map((index) => points[(offset + index) % points.length]!);
    if (!a || !b || !c) return false;
    for (const coordinate of [(value: number) => value, Math.fround]) {
      const cross =
        (coordinate(b.x) - coordinate(a.x)) * (coordinate(c.y) - coordinate(a.y)) -
        (coordinate(c.x) - coordinate(a.x)) * (coordinate(b.y) - coordinate(a.y));
      if (Math.abs(cross) < 1e-8) return false;
    }
    return true;
  };
  if (usable(0)) return points;
  for (let offset = 1; offset < points.length; offset++) {
    if (!usable(offset)) continue;
    const rotated = [...points.slice(offset), ...points.slice(0, offset)];
    // Recover only well-defined planar caps. A warped authored solid needs an
    // explicit asset repair rather than an arbitrary new cap selected here.
    for (const height of ["z_top", "z_bottom"] as const)
      heightPlane(rotated.map((point): Vec3 => [point.x, point.y, point[height]]));
    return rotated;
  }
  throw new Error("Sight obstacle has no stable consecutive vertices for its cap planes");
}
