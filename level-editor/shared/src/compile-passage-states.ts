import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";

export interface PassageStateRequirement {
  layer: number;
  area: number;
  /** Any matching mask permits this part of the passage; an empty list blocks it. */
  allowed_states: number[];
}

/**
 * Prepare corridor checks indexed by motion sector and layer.
 * Requirements from different areas must all hold. Their state words are local
 * to those areas, so equal bit numbers never couple unrelated placed assets.
 * This prepares data only; callers must bind it to native gate authorization.
 */
function passageAreas(geometry: CompiledAssetGeometry, changingOnly = false) {
  type Area = CompiledAssetGeometry["motion_data"]["layers"][number][number];
  const areas = new Map<
    string,
    { area: number; entry: Area; allowed: (a: Point, b: Point) => number[] }
  >();
  let sector = 0;
  geometry.motion_data.layers.forEach((entries, layer) => {
    entries.forEach((entry, area) => {
      areas.set(`${sector}/${layer}`, {
        area,
        entry,
        allowed: prepareCorridorStates(
          changingOnly
            ? entry.obstacles.filter((obstacle) => obstacle.state_id !== 0)
            : entry.obstacles,
        ),
      });
      sector += 1 + entry.obstacles.length;
    });
  });
  return areas;
}

/** Prepare both legs of ordinary passages using each receiving area's state word. */
export function compileOrdinaryPassageStates(
  geometry: CompiledAssetGeometry,
  /** Generated crossings already checked against both floors' permanent free coverage. */
  preparedPermanent: ReadonlySet<CompiledAssetGeometry["doors"][number]> = new Set(),
): PassageStateRequirement[][] {
  const areas = passageAreas(geometry);
  const changing = preparedPermanent.size ? passageAreas(geometry, true) : areas;
  return geometry.doors.map((door, index) => {
    const result: PassageStateRequirement[] = [];
    for (const side of ["in", "out"] as const) {
      const layer = door[`layer_${side}`];
      const entry = (preparedPermanent.has(door) ? changing : areas).get(
        `${door[`sector_${side}`]}/${layer}`,
      );
      if (!entry) throw new Error(`Door ${index}: missing ${side} area`);
      const allowed_states = entry.allowed(door.point_mid, door[`point_${side}`]);
      if (allowed_states.length !== 1 || allowed_states[0] !== 0)
        result.push({ layer, area: entry.area, allowed_states });
    }
    return result;
  });
}

/** Virtual interiors have no motion-area state word; check only the outdoor leg. */
export function compileBuildingPassageStates(
  geometry: CompiledAssetGeometry,
  /** Doors whose permanent structural/foreign collision was checked during placement compilation. */
  preparedPermanent: ReadonlySet<CompiledAssetGeometry["doors"][number]> = new Set(),
): PassageStateRequirement[][][] {
  const areas = passageAreas(geometry);
  const changing = preparedPermanent.size ? passageAreas(geometry, true) : areas;
  return (geometry.buildings ?? []).map(({ Building }, buildingIndex) =>
    Building.doors.map((door, doorIndex) => {
      const layer = door.layer_out;
      const entry = (preparedPermanent.has(door) ? changing : areas).get(
        `${door.sector_out}/${layer}`,
      );
      if (!entry) throw new Error(`Building ${buildingIndex} door ${doorIndex}: missing out area`);
      const allowed_states = entry.allowed(door.point_mid, door.point_out);
      return allowed_states.length === 1 && allowed_states[0] === 0
        ? []
        : [{ layer, area: entry.area, allowed_states }];
    }),
  );
}

export function compileLiftPassageStates(
  geometry: CompiledAssetGeometry,
): PassageStateRequirement[][][] {
  const areas = passageAreas(geometry);
  return (geometry.lifts ?? []).map((lift, liftIndex) =>
    lift.doors.map((door, doorIndex) => {
      const result: PassageStateRequirement[] = [];
      for (const side of ["in", "out"] as const) {
        const layer = door[`layer_${side}`];
        let entry = areas.get(`${door[`sector_${side}`]}/${layer}`);
        if (!entry) throw new Error(`Lift ${liftIndex} door ${doorIndex}: missing ${side} area`);
        // A high ladder exit keeps its authored midpoint on the structure being
        // climbed; its clamber animation starts against that permanent solid.
        // Other permanent solids and every switchable obstacle still apply.
        // TODO: a foreign permanent solid touching the midpoint is also exempted;
        // separating it would need per-asset obstacle ownership here.
        if (lift.lift_type === 2 && door.door_type !== 5) {
          const obstacles = entry.entry.obstacles;
          const climbed = obstacles.filter(
            (obstacle) =>
              obstacle.state_id === 0 &&
              prepareCorridorStates([obstacle])(door.point_mid, door.point_mid).length === 0,
          );
          if (climbed.length)
            entry = {
              ...entry,
              allowed: prepareCorridorStates(
                obstacles.filter((obstacle) => !climbed.includes(obstacle)),
              ),
            };
        }
        let point = door[`point_${side}`];
        const middle = door.point_mid;
        if (side === "in" && lift.lift_type === 3 && [4, 6].includes(door.door_type)) {
          const radius = door.door_type === 6 ? 65 : 60;
          const dx = point[0] - middle[0];
          const dy = point[1] - middle[1];
          const length = Math.hypot(dx, dy);
          if (length > 0)
            point = [middle[0] + (dx * radius) / length, middle[1] + (dy * radius) / length];
        }
        const allowed_states = entry.allowed(middle, point);
        if (allowed_states.length !== 1 || allowed_states[0] !== 0)
          result.push({ layer, area: entry.area, allowed_states });
      }
      return result;
    }),
  );
}
