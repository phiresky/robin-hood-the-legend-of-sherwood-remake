import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { prepareDirectStairApproaches } from "./prepare-direct-stair-approaches.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";

type Area = CompiledAssetGeometry["motion_data"]["layers"][number][number];

function edgeHitsBox(a: Point, b: Point, center: Point, half: Readonly<Point>): boolean {
  let low = 0;
  let high = 1;
  for (const axis of [0, 1]) {
    const radius = half[axis]!;
    const delta = b[axis]! - a[axis]!;
    const minimum = center[axis]! - radius - a[axis]!;
    const maximum = center[axis]! + radius - a[axis]!;
    if (delta === 0) {
      if (minimum > 0 || maximum < 0) return false;
    } else {
      low = Math.max(low, Math.min(minimum / delta, maximum / delta));
      high = Math.min(high, Math.max(minimum / delta, maximum / delta));
    }
  }
  return low <= high;
}

function touches(polygon: Point[], point: Point, half: Readonly<Point> = [6, 3]): boolean {
  return polygon.some((a, index) =>
    edgeHitsBox(a, polygon[(index + 1) % polygon.length]!, point, half),
  );
}

function crosses(polygon: Point[], source: Point, goal: Point): boolean {
  const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];
  const direction: Point = [goal[0] - source[0], goal[1] - source[1]];
  return polygon.some((a, index) => {
    const b = polygon[(index + 1) % polygon.length]!;
    const edge: Point = [b[0] - a[0], b[1] - a[1]];
    const offset: Point = [a[0] - source[0], a[1] - source[1]];
    const determinant = cross(direction, edge);
    if (determinant === 0) return false;
    const t = cross(offset, edge) / determinant;
    const u = cross(offset, direction) / determinant;
    return t > 0 && t <= 1 && u >= 0 && u <= 1;
  });
}

/** Prepare native passage endpoints once, before writing the map descriptor. */
export function compileLiftApproaches(geometry: CompiledAssetGeometry): void {
  compileApproaches(geometry, false);
}

/** Virtual rooms only need actor clearance at their outdoor approach. */
export function compileBuildingApproaches(geometry: CompiledAssetGeometry): void {
  compileApproaches(geometry, true);
}

function compileApproaches(geometry: CompiledAssetGeometry, buildingsOnly: boolean): void {
  type Lift = NonNullable<CompiledAssetGeometry["lifts"]>[number];
  const directStairs = buildingsOnly
    ? new Set<Lift["doors"][number]>()
    : prepareDirectStairApproaches(geometry, false);
  const unresolved: {
    lift: Lift;
    door: Lift["doors"][number];
    fits: (point: Point) => boolean;
    warning: string;
  }[] = [];
  const areas = new Map<string, Area>();
  let sector = 0;
  geometry.motion_data.layers.forEach((entries, layer) => {
    for (const area of entries) {
      areas.set(`${sector}/${layer}`, area);
      sector += 1 + area.obstacles.length;
    }
  });
  let offsets: [number, number, number][] | undefined;
  const nearby = () => {
    if (!offsets) {
      offsets = [];
      for (let x = -64; x <= 64; x++)
        for (let y = -64; y <= 64; y++) {
          const distance = x * x + y * y;
          if (distance > 0 && distance <= 64 * 64) offsets.push([distance, x, y]);
        }
      offsets.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
    }
    return offsets;
  };
  const jobs: { label: string; lift?: Lift; doors: Lift["doors"]; sides: ("in" | "out")[] }[] =
    buildingsOnly
      ? (geometry.buildings ?? []).map(({ Building }, index) => ({
          label: `Building ${index}`,
          doors: Building.doors,
          sides: ["out"],
        }))
      : (geometry.lifts ?? []).map((lift, index) => ({
          label: `Lift ${index}`,
          lift,
          doors: lift.doors,
          sides: ["in", "out"],
        }));
  for (const { label, lift, doors, sides } of jobs) {
    // World-space construction metadata is not part of native passage execution.
    if (lift) delete lift.physical_navigation;
    if (lift && (lift.lift_type < 0 || lift.lift_type > 3)) continue;
    for (const [doorIndex, door] of doors.entries()) {
      for (const side of sides) {
        const area = areas.get(`${door[`sector_${side}`]}/${door[`layer_${side}`]}`);
        if (!area) throw new Error(`${label} door ${doorIndex}: missing ${side} area`);
        const boundary = area.precise_polygon ?? area.polygon.points;
        const blockers = area.obstacles.filter((obstacle) => obstacle.state_id === 0);
        const half: Point = side === "in" && directStairs.has(door) ? [5, 2] : [6, 3];
        const receivers = buildingsOnly
          ? geometry.sight_obstacles
              .filter(
                (obstacle) =>
                  Array.isArray(obstacle.projection_area) &&
                  obstacle.projection_area[0] === door.sector_out &&
                  obstacle.projection_area[1] === door.layer_out,
              )
              .map((obstacle) => obstacle.points.map((p): Point => [p.x, p.y - p.z_top]))
          : [];
        const handoffReceivers = receivers.filter((polygon) =>
          pointInGameplayPolygon(door.point_mid, polygon, true),
        );
        const sameHandoffReceiver = (point: Point) =>
          !handoffReceivers.length ||
          handoffReceivers.some(
            (polygon) =>
              pointInGameplayPolygon(point, polygon, true) &&
              !crosses(polygon, door.point_mid, point),
          );
        const fits = (point: Point) =>
          pointInGameplayPolygon(point, boundary) &&
          sameHandoffReceiver(point) &&
          (!receivers.length ||
            receivers.some((polygon) => pointInGameplayPolygon(point, polygon, true))) &&
          !touches(boundary, point, half) &&
          blockers.every(
            (obstacle) =>
              !pointInGameplayPolygon(point, obstacle.polygon.points) &&
              !touches(obstacle.polygon.points, point),
          );
        const point = door[`point_${side}`];
        const middle = door.point_mid;
        const delta: Point = [point[0] - middle[0], point[1] - middle[1]];
        const length = Math.hypot(...delta);
        const radius =
          side === "in" && lift?.lift_type === 3 && !door.approach_prepared
            ? door.door_type === 6
              ? 65
              : door.door_type === 4
                ? 60
                : undefined
            : undefined;
        const adapt = (p: Point): Point => {
          if (radius === undefined) return p;
          const dx = p[0] - middle[0],
            dy = p[1] - middle[1];
          const magnitude = Math.hypot(dx, dy);
          return magnitude === 0
            ? p
            : [middle[0] + (dx * radius) / magnitude, middle[1] + (dy * radius) / magnitude];
        };
        const source = adapt(point);
        if (fits(source)) continue;
        if (radius !== undefined && fits(point)) {
          door.approach_prepared = true;
          continue;
        }
        let adjusted: Point | undefined;
        const reachable = (candidate: Point) =>
          candidate.every((value) => value >= -32768 && value <= 32767) &&
          !crosses(boundary, source, candidate) &&
          blockers.every(
            (obstacle) =>
              !pointInGameplayPolygon(source, obstacle.polygon.points) &&
              !crosses(obstacle.polygon.points, source, candidate),
          );
        if (length > 0 && pointInGameplayPolygon(source, boundary)) {
          if (radius === undefined) {
            for (let distance = 0; distance <= 64; distance++) {
              const candidate: Point = [
                Math.round(point[0] + (delta[0] * distance) / length),
                Math.round(point[1] + (delta[1] * distance) / length),
              ];
              if (
                !reachable(candidate) ||
                !pointInGameplayPolygon(candidate, boundary) ||
                blockers.some((obstacle) => touches(obstacle.polygon.points, candidate))
              )
                break;
              if (fits(candidate)) {
                adjusted = candidate;
                break;
              }
            }
            if (!adjusted)
              for (const [, x, y] of nearby()) {
                const candidate: Point = [point[0] + x, point[1] + y];
                if (
                  sameHandoffReceiver(source)
                    ? x * delta[0] + y * delta[1] < 0
                    : (candidate[0] - middle[0]) * delta[0] +
                        (candidate[1] - middle[1]) * delta[1] <=
                      0
                )
                  continue;
                if (fits(candidate) && reachable(candidate)) {
                  adjusted = candidate;
                  break;
                }
              }
          } else {
            let best = Infinity;
            for (let x = -64; x <= 64; x++)
              for (let y = -64; y <= 64; y++) {
                const candidate: Point = [point[0] + x, point[1] + y];
                if (
                  candidate.some((value) => value < -32768 || value > 32767) ||
                  (candidate[0] === middle[0] && candidate[1] === middle[1])
                )
                  continue;
                const adapted = adapt(candidate);
                const distance = (adapted[0] - source[0]) ** 2 + (adapted[1] - source[1]) ** 2;
                if (distance <= 64 * 64 && distance < best && fits(adapted) && reachable(adapted)) {
                  best = distance;
                  adjusted = candidate;
                }
              }
          }
        }
        if (adjusted) door[`point_${side}`] = adjusted;
        else {
          if (buildingsOnly) {
            // A narrow landing may allow passage animations but no standalone
            // walking destination. Share a checked passage endpoint so entering
            // the room does not require an impossible intervening walk.
            const clear = prepareCorridorStates(
              blockers.map((obstacle) => ({
                ...obstacle,
                polygon: { points: obstacle.precise_polygon ?? obstacle.polygon.points },
              })),
              boundary,
            );
            const handoffs = geometry.doors
              .filter((passage) => passage.door_type === 0)
              .flatMap((passage) =>
                (["in", "out"] as const).flatMap((endpoint) =>
                  passage[`sector_${endpoint}`] === door.sector_out &&
                  passage[`layer_${endpoint}`] === door.layer_out
                    ? [passage[`point_${endpoint}`]]
                    : [],
                ),
              )
              .filter(
                (candidate) =>
                  Math.hypot(candidate[0] - point[0], candidate[1] - point[1]) <= 64 &&
                  sameHandoffReceiver(candidate) &&
                  (!receivers.length ||
                    receivers.some((polygon) =>
                      pointInGameplayPolygon(candidate, polygon, true),
                    )) &&
                  clear(source, candidate).length > 0,
              )
              .sort(
                (a, b) =>
                  Math.hypot(a[0] - point[0], a[1] - point[1]) -
                  Math.hypot(b[0] - point[0], b[1] - point[1]),
              );
            if (handoffs[0]) {
              door.point_out = [...handoffs[0]];
              (geometry.warnings ??= []).push(
                `${label} door ${doorIndex}: shared passage handoff prepared on a narrow landing; no standalone actor-sized approach is available.`,
              );
              continue;
            }
          }
          const warning = `${label} door ${doorIndex}: no actor-sized ${side} approach near the authored passage; traversal may be unavailable.`;
          (geometry.warnings ??= []).push(warning);
          if (side === "in" && lift?.lift_type === 1)
            unresolved.push({ lift, door, fits, warning });
        }
      }
    }
  }
  if (unresolved.length) {
    const direct = prepareDirectStairApproaches({
      ...geometry,
      lifts: [...new Set(unresolved.map((entry) => entry.lift))],
    });
    const recovered = new Set(
      unresolved
        .filter((entry) => direct.has(entry.door) || entry.fits(entry.door.point_in))
        .map((entry) => entry.warning),
    );
    geometry.warnings = geometry.warnings?.filter((warning) => !recovered.has(warning));
  }
}
