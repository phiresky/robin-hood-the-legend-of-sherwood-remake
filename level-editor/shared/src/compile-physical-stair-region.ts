import clipping from "polygon-clipping";
import {
  compilePhysicalStair,
  compilePhysicalStairArea,
  isPhysicalClippingSliver,
  type PhysicalStairInput,
} from "./compile-physical-stair.ts";
import {
  compilePhysicalTransitionObstacles,
  type PlacedTransitionBlocker,
} from "./compile-movement-transitions.ts";
import { clipHeight, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { movementVolumeHeightSlice } from "./movement-volume-height-slice.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

export interface PhysicalStairRegionInput extends Omit<PhysicalStairInput, "obstacles"> {
  /** Visible export rectangle in projected coordinates: left, top, right, bottom. */
  frame?: [number, number, number, number];
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
  let surfaces = input.surfaces;
  let floor = compilePhysicalStair({ ...input, obstacles: [] });
  if (input.frame) {
    const [left, top, right, bottom] = input.frame;
    if (input.frame.some((value) => !Number.isFinite(value)) || left >= right || top >= bottom)
      throw new Error("Physical stair needs a finite nonempty export frame");
    const {
      boundary,
      plane: [a, b, c],
    } = floor.navigation;
    const xs = boundary.map(([x]) => x),
      ys = boundary.map(([, y]) => y);
    const minX = Math.min(...xs),
      maxX = Math.max(...xs);
    const minY = Math.min(...ys),
      maxY = Math.max(...ys);
    let mask: Point[] = [
      [minX, minY],
      [maxX, minY],
      [maxX, maxY],
      [minX, maxY],
    ];
    // Pull the screen rectangle back to linear inequalities on the physical
    // floor. This remains defined when the entire floor projects onto a line.
    const inequalities: HeightPlane[] = [
      [1, 0, -left],
      [-1, 0, right],
      [-a, 1 - b, -c - top],
      [a, b - 1, c + bottom],
    ];
    for (const inequality of inequalities) mask = clipHeight(mask, inequality);
    const clipped =
      mask.length < 3 ? [] : clipping.intersection([boundary, ...floor.holes], [mask]);
    if (!clipped.length) throw new Error("Physical stair has no floor inside the export frame");
    const lift = (ring: Point[]): Vec3[] => ring.map(([x, y]) => [x, y, a * x + b * y + c]);
    surfaces = clipped.map((polygon) => ({
      polygon: lift(polygon[0]!),
      holes: polygon
        .slice(1)
        .filter((ring) => !isPhysicalClippingSliver(ring))
        .map(lift),
    }));
    floor = compilePhysicalStair({ surfaces, doors: input.doors, obstacles: [] });
  }
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
    surfaces,
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
