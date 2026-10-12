import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { compileLiftApproaches } from "./compile-lift-approaches.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

/** Ordinary gates entering stairs need the same prepared approaches as lift-owned gates. */
export function prepareOrdinaryStairPassages(geometry: CompiledAssetGeometry): void {
  const candidates = new Map<
    CompiledAssetGeometry["doors"][number],
    { door: CompiledAssetGeometry["doors"][number]; reversed: boolean }
  >();
  const lifts = (geometry.lifts ?? []).flatMap((lift) => {
    if (lift.lift_type !== 1) return [];
    const ordinary = geometry.doors.filter(
      (door) =>
        door.door_type === 0 &&
        !candidates.has(door) &&
        door.world_endpoints &&
        (door.sector_in === lift.motion_area_index || door.sector_out === lift.motion_area_index),
    );
    if (!ordinary.length) return [];
    const copies = ordinary.map((door) => {
      const copy = structuredClone(door);
      const reversed = door.sector_in !== lift.motion_area_index;
      if (reversed) {
        [copy.point_in, copy.point_out] = [copy.point_out, copy.point_in];
        [copy.sector_in, copy.sector_out] = [copy.sector_out, copy.sector_in];
        [copy.layer_in, copy.layer_out] = [copy.layer_out, copy.layer_in];
        [copy.world_endpoints!.inside, copy.world_endpoints!.outside] = [
          copy.world_endpoints!.outside,
          copy.world_endpoints!.inside,
        ];
      }
      candidates.set(door, { door: copy, reversed });
      return copy;
    });
    return [{ ...lift, doors: [...lift.doors, ...copies] }];
  });
  if (!candidates.size) return;
  const prepared = { ...geometry, lifts, warnings: [...(geometry.warnings ?? [])] };
  compileLiftApproaches(prepared);
  geometry.warnings = prepared.warnings;
  for (const [original, { door: candidate, reversed }] of candidates) {
    const endpoints = candidate.world_endpoints!;
    let supported = true;
    for (const side of ["in", "out"] as const) {
      const point = candidate[`point_${side}`];
      const receiver = geometry.sight_obstacles
        .filter(
          (obstacle) =>
            Array.isArray(obstacle.projection_area) &&
            obstacle.projection_area[0] === candidate[`sector_${side}`] &&
            obstacle.projection_area[1] === candidate[`layer_${side}`] &&
            pointInGameplayPolygon(
              point,
              obstacle.points.map((p): Point => [p.x, p.y - p.z_top]),
              true,
            ),
        )
        .sort(
          (a, b) =>
            Math.max(...b.points.map((p) => p.z_top)) - Math.max(...a.points.map((p) => p.z_top)),
        )[0];
      if (!receiver) {
        const old = original[`point_${reversed ? (side === "in" ? "out" : "in") : side}`];
        if (side === "in" || old[0] !== point[0] || old[1] !== point[1]) supported = false;
        continue;
      }
      const anchors =
        receiver.projection_plane ?? receiver.points.map((p): Vec3 => [p.x, p.y, p.z_top]);
      const z = planeHeight(heightPlane(anchors.map(([x, y, z]): Vec3 => [x, y - z, z])), point);
      endpoints[side === "in" ? "inside" : "outside"] = [point[0], point[1] + z, z];
    }
    if (!supported) {
      geometry.warnings.push(
        "Ordinary stair passage: prepared endpoint lacks receiving support; retaining authored approach.",
      );
      continue;
    }
    // Complete the approach before the ordinary gate installs the destination
    // receiver. A later crossing of the stair entry bond would swap it back.
    original.point_in = reversed ? candidate.point_out : candidate.point_in;
    original.point_out = reversed ? candidate.point_in : candidate.point_out;
    original.point_mid = [...candidate.point_in];
    original.world_endpoints = {
      inside: reversed ? endpoints.outside : endpoints.inside,
      outside: reversed ? endpoints.inside : endpoints.outside,
      middle: [...endpoints.inside],
    };
  }
}
