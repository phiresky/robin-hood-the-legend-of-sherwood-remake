import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import { stairBoundaryPlanes, stairCenterRegion } from "./stair-center-region.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";

type Lift = NonNullable<CompiledAssetGeometry["lifts"]>[number];
type Door = Lift["doors"][number];

function closestApproaches(lift: Lift, centers: Point[]): Point[] | undefined {
  const limits = stairBoundaryPlanes(centers);
  if (limits.length < 3) return undefined;
  const proposals: Point[] = [];
  for (const door of lift.doors) {
    if (door.sector_in !== lift.motion_area_index) return undefined;
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
        if (x === door.point_mid[0] && y === door.point_mid[1]) continue;
        if (limits.every(([a, b, c]) => a * x + b * y + c >= 0)) {
          best = [x, y];
          bestDistance = distance;
        }
      }
    if (!best) return undefined;
    proposals.push(best);
  }
  return proposals;
}

/** Prepare mutually straight-reachable endpoints in a convex, unobstructed stair. */
export function prepareDirectStairApproaches(
  geometry: CompiledAssetGeometry,
  includeFullBox = true,
): Set<Door> {
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
    const fullCenters = stairCenterRegion(boundary, [half[0] + 0.01, half[1] + 0.01]);
    const useInset = fullCenters.length < 3;
    if (!useInset && !includeFullBox) continue;
    // Direct native walking sweeps the one-unit-inset box. Keep a small margin
    // so integer endpoints cannot touch a collision edge through roundoff.
    const centers = useInset
      ? stairCenterRegion(boundary, [half[0] - 0.99, half[1] - 0.99])
      : fullCenters;
    const limits = stairBoundaryPlanes(centers);
    if (limits.length < 3) continue;
    if (
      !useInset &&
      lift.doors.every((door) =>
        limits.every(([a, b, c]) => a * door.point_in[0] + b * door.point_in[1] + c >= 0),
      )
    )
      continue;
    const allowed = prepareCorridorStates([], boundary);
    const clear = (points: Point[] | undefined): points is Point[] =>
      !!points && points.every((a) => points.every((b) => allowed(a, b).length > 0));
    let proposals = closestApproaches(lift, centers);
    if (useInset && !clear(proposals)) {
      // Horizontal sweeps reserve one extra unit on their left side. Prepare
      // that asymmetric footprint as a shifted symmetric center region.
      const horizontal = stairCenterRegion(boundary, [half[0] - 0.49, half[1] - 0.99]).map(
        ([x, y]): Point => [x + 0.5, y],
      );
      proposals = closestApproaches(lift, horizontal);
    }
    if (!clear(proposals)) continue;
    // All endpoint pairs have been checked against the native sweep, including
    // its horizontal special case. Existing direct movement executes the route.
    lift.doors.forEach((door, i) => {
      door.point_in = proposals[i]!;
      if (useInset) prepared.add(door);
    });
  }
  return prepared;
}
