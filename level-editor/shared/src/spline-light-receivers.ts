import type { AssetLightRegion, AssetWalkableSurface } from "./asset-gameplay.ts";
import { clipHeight, heightPlane, planeHeight } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

/** Find probes inside both the exported light contour and its deformed source receivers. */
export function splineLightReceivers(
  light: AssetLightRegion,
  surfaces: AssetWalkableSurface[],
  origin: readonly number[],
): [Vec3, Vec3][] {
  const contour: Point[] = light.polygon.map(([x, y, z]) => [
    Math.round(x - origin[0]!) + origin[0]!,
    Math.round(y - z - origin[1]!) + origin[1]!,
  ]);
  const result: [Vec3, Vec3][] = [];
  for (const surface of surfaces) {
    const world = surface.polygon.map(([x, y], i): Vec3 => {
      const z = typeof surface.height === "number" ? surface.height : surface.height[i]!;
      return [x, y - z, z];
    });
    const plane = heightPlane(world);
    const signedArea = world.reduce((sum, a, i) => {
      const b = world[(i + 1) % world.length]!;
      return sum + a[0] * b[1] - a[1] * b[0];
    }, 0);
    let overlap = contour;
    for (let i = 0; i < world.length && overlap.length; i++) {
      const a = world[i]!,
        b = world[(i + 1) % world.length]!,
        sign = Math.sign(signedArea);
      overlap = clipHeight(overlap, [
        (a[1] - b[1]) * sign,
        (b[0] - a[0]) * sign,
        (a[0] * b[1] - a[1] * b[0]) * sign,
      ]);
    }
    const area = overlap.reduce((sum, a, i) => {
      const b = overlap[(i + 1) % overlap.length]!;
      return sum + a[0] * b[1] - a[1] * b[0];
    }, 0);
    if (Math.abs(area) < 1e-8) continue;
    const x = overlap.reduce((sum, p) => sum + p[0], 0) / overlap.length;
    const y = overlap.reduce((sum, p) => sum + p[1], 0) / overlap.length;
    const z = planeHeight(plane, [x, y]);
    result.push([
      [x, y + z - 1 / 1024, z - 1 / 1024],
      [x, y + z + 1 / 1024, z + 1 / 1024],
    ]);
  }
  return result;
}
