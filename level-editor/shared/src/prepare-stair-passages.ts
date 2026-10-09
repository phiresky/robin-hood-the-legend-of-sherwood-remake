import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { NAVIGATION_HALF_DIAGONAL } from "./navigation-footprint.ts";

/** Prepare the sector handoff where an actor footprint straddles a stair opening. */
export function prepareStairPassages(geometry: CompiledAssetGeometry): void {
  const areas = new Map<number, (a: Point, b: Point) => number[]>();
  let sector = 0;
  for (const layer of geometry.motion_data.layers)
    for (const area of layer) {
      areas.set(sector, prepareCorridorStates(area.obstacles.filter((o) => o.state_id === 0)));
      sector += 1 + area.obstacles.length;
    }
  const radius = Math.hypot(...NAVIGATION_HALF_DIAGONAL);
  for (const lift of geometry.lifts ?? []) {
    if (lift.lift_type !== 1) continue;
    for (const door of lift.doors) {
      const inside = areas.get(door.sector_in);
      const outside = areas.get(door.sector_out);
      if (!inside || !outside) throw new Error("Stair passage refers to a missing motion area");
      const clear = (point: Point) =>
        inside(point, door.point_in).length > 0 && outside(point, door.point_out).length > 0;
      const original = door.point_mid;
      if (clear(original)) continue;
      const dx = door.point_out[0] - original[0];
      const dy = door.point_out[1] - original[1];
      const length = Math.hypot(dx, dy);
      if (length === 0) continue;
      // Keep the authored approach and both endpoints. The handoff may move
      // toward the landing by at most one actor radius; both animation legs
      // must still clear permanent obstacles on their respective floors.
      for (let distance = 0.25; distance <= Math.min(radius, length); distance += 0.25) {
        const point: Point = [
          Math.round(original[0] + (dx * distance) / length),
          Math.round(original[1] + (dy * distance) / length),
        ];
        if (Math.hypot(point[0] - original[0], point[1] - original[1]) > radius) continue;
        if (clear(point)) {
          door.point_mid = point;
          break;
        }
      }
    }
  }
}
