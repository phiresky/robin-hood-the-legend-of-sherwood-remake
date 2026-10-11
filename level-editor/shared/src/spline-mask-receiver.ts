import earcut, { flatten } from "earcut";
import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Vec3 } from "./scene.ts";

/** Retain receiver ownership only when one authored floor contains the anchor. */
export function splineMaskReceiver(
  anchor: Vec3,
  surfaces: AssetWalkableSurface[],
  source: (node: string, point: Vec3) => Vec3,
): Vec3[][] {
  const candidates = surfaces.flatMap((surface) => {
    const local = surface.polygon.map(([x, y], i): Vec3 => [
      x,
      y,
      typeof surface.height === "number" ? surface.height : surface.height[i]!,
    ]);
    const localPlane = heightPlane(local);
    const rings = [
      local,
      ...(surface.holes ?? []).map((hole) =>
        hole.map(([x, y]): Vec3 => [x, y, planeHeight(localPlane, [x, y])]),
      ),
    ].map((ring) => ring.map((p) => source(surface.node, p)));
    const plane = heightPlane(rings[0]!);
    if (Math.abs(planeHeight(plane, [anchor[0], anchor[1]]) - anchor[2]) > 1e-6) return [];
    const contains = (ring: Vec3[]) =>
      pointInGameplayPolygon(
        [anchor[0], anchor[1]],
        ring.map(([x, y]) => [x, y]),
        true,
      );
    return contains(rings[0]!) && !rings.slice(1).some(contains) ? [rings] : [];
  });
  if (candidates.length !== 1) return [];
  const flat = flatten(candidates[0]!);
  const ids = earcut(flat.vertices, flat.holes, 3);
  const triangles: Vec3[][] = [];
  for (let i = 0; i < ids.length; i += 3)
    triangles.push(
      ids
        .slice(i, i + 3)
        .map((id): Vec3 => [
          flat.vertices[id * 3]!,
          flat.vertices[id * 3 + 1]!,
          flat.vertices[id * 3 + 2]!,
        ]),
    );
  return triangles;
}
