import clipping from "polygon-clipping";
import {
  compilePhysicalStair,
  compilePhysicalStairArea,
  type PhysicalStairInput,
} from "./compile-physical-stair.ts";
import {
  compilePhysicalTransitionObstacles,
  type PlacedTransitionBlocker,
} from "./compile-movement-transitions.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { movementVolumeHeightSlice } from "./movement-volume-height-slice.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

export interface PhysicalStairRegionInput extends Omit<PhysicalStairInput, "obstacles"> {
  solids: {
    owner: string;
    polygon: Point[];
    holes: Point[][];
    top: HeightPlane;
    /** Includes any authored upright movement headroom. */
    bottom: HeightPlane;
  }[];
  clearances: { owner: string; polygon: Point[]; holes: Point[][]; plane: HeightPlane }[];
  /** Placed world contours, including any permanent planar blockers. */
  blockers: PlacedTransitionBlocker[];
}

/** Assemble collision and control identities without deriving geometry from screen coordinates. */
export function compilePhysicalStairRegion(input: PhysicalStairRegionInput) {
  const floor = compilePhysicalStair({ ...input, obstacles: [] });
  const { plane, boundary } = floor.navigation;
  const blockers = [...input.blockers];
  for (const [index, solid] of input.solids.entries()) {
    const slice = movementVolumeHeightSlice(solid.polygon, plane, solid.bottom, solid.top);
    if (slice.length < 3) continue;
    let regions = clipping.intersection([solid.polygon, ...solid.holes], [slice]);
    for (const clearance of input.clearances) {
      if (
        clearance.owner !== solid.owner ||
        !plane.every((value, i) => Math.abs(value - clearance.plane[i]!) < 1e-7)
      )
        continue;
      regions = clipping.difference(regions, [clearance.polygon, ...clearance.holes]);
    }
    for (const region of regions)
      blockers.push({
        transition: `solid/${index}`,
        fixed: true,
        applied: false,
        plane,
        polygon: region[0]!,
        holes: region.slice(1),
      });
  }
  const changing = compilePhysicalTransitionObstacles(boundary, floor.holes, plane, blockers);
  const compiled = compilePhysicalStairArea({
    surfaces: input.surfaces,
    doors: input.doors,
    obstacles: changing.obstacles.map((obstacle) => ({
      stateId: obstacle.state_id,
      polygon: obstacle.polygon.points.map(([x, y]): Vec3 => [x, y, planeHeight(plane, [x, y])]),
    })),
  });
  return {
    ...compiled,
    pairs: changing.pairs,
    initialBlockers: [...floor.holes, ...changing.initial],
  };
}
