import clipping, { type Polygon, type MultiPolygon } from "polygon-clipping";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { heightPlane, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { pointInGameplayPolygon, onClippedReceivingBoundary } from "./navigation-anchor.ts";
import { supportedFootprintCenters, footprintBoundaryHull } from "./supported-footprint-centers.ts";
import { stairCenterRegion } from "./stair-center-region.ts";
import { compileNavigationGraph } from "./compile-navigation-graph.ts";
import { convexSupportRegions } from "./convex-support-regions.ts";

const area = (regions: MultiPolygon) =>
  regions.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce(
        (total, ring, index) =>
          total +
          ((index === 0 ? 1 : -1) *
            Math.abs(
              ring.reduce((value, p, i) => {
                const q = ring[(i + 1) % ring.length]!;
                return value + p[0] * q[1] - p[1] * q[0];
              }, 0),
            )) /
            2,
        0,
      ),
    0,
  );

/** Require a shared segment at equal elevation, rather than projected overlap. */
function connectedAtHeight(
  floor: Polygon,
  plane: HeightPlane,
  landing: Polygon,
  height: HeightPlane,
) {
  const difference = plane.map((value, i) => value - height[i]!) as HeightPlane;
  const points: Point[] = [];
  const overlap = clipping.intersection(floor, landing);
  for (const polygon of overlap)
    for (const ring of polygon)
      for (let i = 1; i < ring.length; i++) {
        const a = ring[i - 1]!,
          b = ring[i]!;
        const da = planeHeight(difference, a),
          db = planeHeight(difference, b);
        if (Math.abs(da) < 1e-4) points.push(a);
        if (da * db < 0) {
          const t = da / (da - db);
          points.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
      }
  // Adjacent polygons can share an edge with no positive-area intersection.
  for (const [a, b] of [
    [floor, landing],
    [landing, floor],
  ])
    for (const point of a![0]!)
      if (
        onClippedReceivingBoundary(point, b![0]!) &&
        Math.abs(planeHeight(difference, point)) < 1e-4
      )
        points.push(point);
  return points.some((a) => points.some((b) => Math.hypot(a[0] - b[0], a[1] - b[1]) > 1e-3));
}

/** Recover short stair clearance using static support on its connected landings. */
export function prepareJoinedStairClearance(geometry: CompiledAssetGeometry): number {
  type Area = CompiledAssetGeometry["motion_data"]["layers"][number][number];
  const areas = new Map<string, Area>();
  let sector = 0;
  geometry.motion_data.layers.forEach((entries, layer) => {
    for (const entry of entries) {
      areas.set(`${sector}/${layer}`, entry);
      sector += 1 + entry.obstacles.length;
    }
  });
  let prepared = 0;
  const changingReceivers = new Set(
    (geometry.movement_transitions ?? []).flatMap((transition) => [
      ...(transition.initial_sight ?? []),
      ...(transition.applied_sight ?? []),
    ]),
  );
  for (const lift of geometry.lifts ?? []) {
    const navigation = lift.physical_navigation;
    if (
      lift.lift_type !== 1 ||
      !navigation ||
      navigation.floor_patches ||
      navigation.obstacles.length
    )
      continue;
    const first = lift.doors[0];
    if (!first || navigation.doors.length !== lift.doors.length) continue;
    const motion = areas.get(`${lift.motion_area_index}/${first.layer_in}`);
    if (!motion || motion.obstacles.length) continue;
    const boundary = motion.precise_polygon ?? motion.polygon.points;
    const oldCenters = supportedFootprintCenters([[boundary]], [[boundary]], [5, 2]);
    const floor: Polygon = [navigation.boundary];
    const [a, b, c] = navigation.plane;
    if (Math.abs(1 - b) < 1e-6) continue;
    const supports: Polygon[] = [floor];
    for (const [index, door] of lift.doors.entries()) {
      const outside = navigation.doors[index]!.outside;
      const receiverMotion = areas.get(`${door.sector_out}/${door.layer_out}`);
      if (!receiverMotion || receiverMotion.is_lift || receiverMotion.state_id !== 0) continue;
      for (const [receiverIndex, receiver] of geometry.sight_obstacles.entries()) {
        if (changingReceivers.has(receiverIndex)) continue;
        if (
          !Array.isArray(receiver.projection_area) ||
          receiver.projection_area[0] !== door.sector_out ||
          receiver.projection_area[1] !== door.layer_out
        )
          continue;
        const vertices =
          receiver.projection_plane ??
          receiver.points.slice(0, 3).map((p): Vec3 => [p.x, p.y, p.z_top]);
        // The native receiver uses its explicit anchors or first three top
        // vertices. Remaining volume vertices do not redefine that plane.
        const plane = heightPlane(vertices);
        if (
          Math.abs(plane[1] - 1) < 1e-6 ||
          Math.abs(planeHeight(plane, [outside[0], outside[1]]) - outside[2]) >= 1e-4
        )
          continue;
        const polygon = receiver.points.map((p): Point => [p.x, p.y]);
        if (!pointInGameplayPolygon([outside[0], outside[1]], polygon, true)) continue;
        const world = ([x, y]: Point): Point => [x, (y + plane[0] * x + plane[2]) / (1 - plane[1])];
        let support = clipping.intersection([polygon], [receiverMotion.polygon.points.map(world)]);
        // A state-dependent landing obstruction cannot supply unconditional
        // clearance. Keep every collision state excluded from this support.
        for (const obstacle of receiverMotion.obstacles)
          support = clipping.difference(support, [obstacle.polygon.points.map(world)]);
        for (const piece of support)
          if (connectedAtHeight(floor, navigation.plane, piece, plane)) supports.push(piece);
      }
    }
    if (supports.length === 1) continue;
    const centers = supportedFootprintCenters([floor], supports, [5, 2]);
    if (centers.length !== 1 || centers[0]!.length !== 1) continue;
    const projected = centers[0]![0]!.map(([x, y]): Point => [x, y - (a * x + b * y + c)]);
    function* candidates() {
      const kernel = stairCenterRegion(projected, [0, 0]);
      if (kernel.length >= 3) yield kernel;
      yield* convexSupportRegions(projected);
    }
    // A convex kernel can be represented exactly by the existing box-based
    // movement boundary. Preserve every center already admitted by that boundary.
    for (const kernel of candidates()) {
      const expanded = footprintBoundaryHull(
        kernel.flatMap(([x, y]) =>
          [
            [-4.99, -1.99],
            [4.99, -1.99],
            [4.99, 1.99],
            [-4.99, 1.99],
          ].map(([dx, dy]): Point => [x + dx!, y + dy!]),
        ),
      );
      const newCenters = supportedFootprintCenters([[expanded]], [[expanded]], [5, 2]);
      if (!newCenters.length || area(clipping.difference(oldCenters, newCenters)) > 1e-7) continue;
      const worldCenters = newCenters.map((polygon) =>
        polygon.map((ring) => ring.map(([x, y]): Point => [x, (y + a * x + c) / (1 - b)])),
      );
      if (area(clipping.difference(worldCenters, centers)) > 1e-7) continue;
      if (expanded.some((point) => point.some((value) => value < -32768 || value > 32767)))
        continue;
      motion.precise_polygon = expanded;
      motion.polygon.points = expanded.map(([x, y]): Point => [Math.round(x), Math.round(y)]);
      prepared++;
      break;
    }
  }
  if (prepared)
    geometry.motion_data.graph_bytes = compileNavigationGraph(geometry.motion_data.layers);
  return prepared;
}
