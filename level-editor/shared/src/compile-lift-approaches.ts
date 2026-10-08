import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

type Area = CompiledAssetGeometry["motion_data"]["layers"][number][number];

function edgeHitsBox(a: Point, b: Point, center: Point): boolean {
  let low = 0;
  let high = 1;
  for (const axis of [0, 1]) {
    const radius = axis === 0 ? 6 : 3;
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

function touches(polygon: Point[], point: Point): boolean {
  return polygon.some((a, index) => edgeHitsBox(a, polygon[(index + 1) % polygon.length]!, point));
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
  for (const [liftIndex, lift] of (geometry.lifts ?? []).entries()) {
    // World-space construction metadata is not part of native passage execution.
    delete lift.physical_navigation;
    if (lift.lift_type < 0 || lift.lift_type > 3) continue;
    for (const [doorIndex, door] of lift.doors.entries()) {
      for (const side of ["in", "out"] as const) {
        const area = areas.get(`${door[`sector_${side}`]}/${door[`layer_${side}`]}`);
        if (!area) throw new Error(`Lift ${liftIndex} door ${doorIndex}: missing ${side} area`);
        const blockers = area.obstacles.filter((obstacle) => obstacle.state_id === 0);
        const fits = (point: Point) =>
          pointInGameplayPolygon(point, area.polygon.points) &&
          !touches(area.polygon.points, point) &&
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
          side === "in" && lift.lift_type === 3
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
        let adjusted: Point | undefined;
        const reachable = (candidate: Point) =>
          candidate.every((value) => value >= -32768 && value <= 32767) &&
          !crosses(area.polygon.points, source, candidate) &&
          blockers.every(
            (obstacle) =>
              !pointInGameplayPolygon(source, obstacle.polygon.points) &&
              !crosses(obstacle.polygon.points, source, candidate),
          );
        if (length > 0 && pointInGameplayPolygon(source, area.polygon.points)) {
          if (radius === undefined) {
            for (let distance = 0; distance <= 64; distance++) {
              const candidate: Point = [
                Math.round(point[0] + (delta[0] * distance) / length),
                Math.round(point[1] + (delta[1] * distance) / length),
              ];
              if (
                !reachable(candidate) ||
                !pointInGameplayPolygon(candidate, area.polygon.points) ||
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
                if (x * delta[0] + y * delta[1] < 0) continue;
                const candidate: Point = [point[0] + x, point[1] + y];
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
        else
          (geometry.warnings ??= []).push(
            `Lift ${liftIndex} door ${doorIndex}: no actor-sized ${side} approach near the authored passage; traversal may be unavailable.`,
          );
      }
    }
  }
}
