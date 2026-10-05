import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

/** Place authored geometry without requiring a nondegenerate screen projection. */
export function placeGameplaySurface(
  surface: AssetWalkableSurface,
  transform: (node: string, point: Vec3) => Vec3,
) {
  const local = surface.polygon.map(([x, y], i): Vec3 => [
    x,
    y,
    typeof surface.height === "number" ? surface.height : surface.height[i]!,
  ]);
  const localPlane = heightPlane(local);
  const localHoles = (surface.holes ?? []).map((hole) =>
    hole.map(([x, y]): Vec3 => [x, y, planeHeight(localPlane, [x, y])]),
  );
  const place = (points: Vec3[]) => points.map((point) => transform(surface.node, point));
  const points = place(local);
  const holes = localHoles.map(place);
  const navigationHeight = surface.navigationHeight;
  const navigationPoints =
    navigationHeight === undefined
      ? points
      : place(local.map(([x, y]): Vec3 => [x, y, navigationHeight]));
  const navigationHoles =
    navigationHeight === undefined
      ? holes
      : localHoles.map((hole) => place(hole.map(([x, y]): Vec3 => [x, y, navigationHeight])));
  return {
    localPlane,
    points,
    holes,
    navigationPoints,
    navigationHoles,
    worldPlane: heightPlane(navigationPoints),
  };
}
