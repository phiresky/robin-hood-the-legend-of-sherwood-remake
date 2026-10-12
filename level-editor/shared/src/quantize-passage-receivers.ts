import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

/** Keep rounded ordinary passage endpoints on their authored receiving surface. */
export function quantizePassageReceivers(geometry: CompiledAssetGeometry): void {
  for (const door of geometry.doors) {
    if (door.door_type !== 0 || !door.world_endpoints) continue;
    for (const side of ["in", "out"] as const) {
      const key = side === "in" ? "inside" : "outside";
      const world = door.world_endpoints[key];
      const exact: Point = [world[0], world[1] - world[2]];
      const receivers = geometry.sight_obstacles
        .filter(
          (obstacle) =>
            Array.isArray(obstacle.projection_area) &&
            obstacle.projection_area[0] === door[`sector_${side}`] &&
            obstacle.projection_area[1] === door[`layer_${side}`],
        )
        .sort(
          (a, b) =>
            Math.max(...b.points.map((point) => point.z_top)) -
            Math.max(...a.points.map((point) => point.z_top)),
        )
        .map((obstacle) => ({
          obstacle,
          polygon: obstacle.points.map((point): Point => [
            Math.fround(point.x),
            Math.fround(Math.fround(point.y) - Math.fround(point.z_top)),
          ]),
        }));
      if (
        receivers.some(({ polygon }) =>
          pointInGameplayPolygon(door[`point_${side}`], polygon, true),
        )
      )
        continue;
      // Quantization must not relocate an unsupported anchor onto another floor.
      const supporting = receivers.filter(({ polygon }) =>
        pointInGameplayPolygon(exact, polygon, true),
      );
      const candidates: Point[] = [];
      for (let x = Math.ceil(exact[0] - 2); x <= Math.floor(exact[0] + 2); x++)
        for (let y = Math.ceil(exact[1] - 2); y <= Math.floor(exact[1] + 2); y++)
          if (
            x >= -32768 &&
            x <= 32767 &&
            y >= -32768 &&
            y <= 32767 &&
            Math.hypot(x - exact[0], y - exact[1]) <= 2
          )
            candidates.push([x, y]);
      candidates.sort(
        (a, b) =>
          Math.hypot(a[0] - exact[0], a[1] - exact[1]) -
          Math.hypot(b[0] - exact[0], b[1] - exact[1]),
      );
      for (const point of candidates) {
        const receiver = supporting.find(({ polygon }) =>
          pointInGameplayPolygon(point, polygon, true),
        );
        if (!receiver) continue;
        const anchors =
          receiver.obstacle.projection_plane ??
          receiver.obstacle.points.map((p): Vec3 => [p.x, p.y, p.z_top]);
        const plane = heightPlane(anchors.map(([x, y, z]): Vec3 => [x, y - z, z]));
        const z = planeHeight(plane, point);
        door[`point_${side}`] = point;
        door.world_endpoints[key] = [point[0], point[1] + z, z];
        break;
      }
    }
  }
}
