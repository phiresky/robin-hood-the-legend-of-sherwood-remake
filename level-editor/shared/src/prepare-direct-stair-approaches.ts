import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import { stairBoundaryPlanes, stairCenterRegion } from "./stair-center-region.ts";

type Door = NonNullable<CompiledAssetGeometry["lifts"]>[number]["doors"][number];

/** Prepare mutually straight-reachable endpoints in a convex, unobstructed stair. */
export function prepareDirectStairApproaches(geometry: CompiledAssetGeometry): Set<Door> {
  const prepared = new Set<Door>();
  const areas = new Map<number, CompiledAssetGeometry["motion_data"]["layers"][number][number]>();
  let sector = 0;
  for (const layer of geometry.motion_data.layers)
    for (const area of layer) {
      areas.set(sector, area);
      sector += 1 + area.obstacles.length;
    }
  for (const lift of geometry.lifts ?? []) {
    if (lift.lift_type !== 1 || lift.doors.length < 2) continue;
    const area = areas.get(lift.motion_area_index);
    if (!area || area.obstacles.length) continue;
    const boundary = area.polygon.points;
    const planes = stairBoundaryPlanes(boundary);
    if (
      planes.length < 3 ||
      planes.some(([x, y, c]) => boundary.some((p) => x * p[0] + y * p[1] + c < -1e-6))
    )
      continue;
    if (stairCenterRegion(boundary, half).length >= 3) continue;
    // Direct native walking sweeps the one-unit-inset box. Keep a small margin
    // so integer endpoints cannot touch a collision edge through roundoff.
    const centers = stairCenterRegion(boundary, [half[0] - 0.99, half[1] - 0.99]);
    const limits = stairBoundaryPlanes(centers);
    if (limits.length < 3) continue;
    const proposals: Point[] = [];
    for (const door of lift.doors) {
      if (door.sector_in !== lift.motion_area_index) break;
      const old = door.point_in;
      let best: Point | undefined;
      let bestDistance = Infinity;
      for (let x = old[0] - 64; x <= old[0] + 64; x++)
        for (let y = old[1] - 64; y <= old[1] + 64; y++) {
          const distance = (x - old[0]) ** 2 + (y - old[1]) ** 2;
          if (
            distance > 64 ** 2 ||
            distance >= bestDistance ||
            x < -32768 ||
            x > 32767 ||
            y < -32768 ||
            y > 32767
          )
            continue;
          if (
            (x - old[0]) * (old[0] - door.point_mid[0]) +
              (y - old[1]) * (old[1] - door.point_mid[1]) <
            0
          )
            continue;
          if (limits.every(([a, b, c]) => a * x + b * y + c >= 0)) {
            best = [x, y];
            bestDistance = distance;
          }
        }
      if (!best) break;
      proposals.push(best);
    }
    if (proposals.length !== lift.doors.length) continue;
    // Convex center coverage proves every endpoint pair has a clear sweep;
    // no graphless search or new runtime movement mode is needed.
    lift.doors.forEach((door, i) => {
      door.point_in = proposals[i]!;
      prepared.add(door);
    });
  }
  return prepared;
}
