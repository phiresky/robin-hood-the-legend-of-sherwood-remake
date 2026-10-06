import clipping from "polygon-clipping";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import {
  compilePhysicalStair,
  compilePhysicalStairArea,
  isPhysicalClippingSliver,
  type PhysicalStairInput,
} from "./compile-physical-stair.ts";
import {
  compilePhysicalTransitionObstacles,
  MovementTransitionLimit,
  type PlacedTransitionBlocker,
} from "./compile-movement-transitions.ts";
import { clipHeight, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { movementVolumeHeightSlice } from "./movement-volume-height-slice.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

type PhysicalStairRegion = ReturnType<typeof compilePhysicalStairArea> & {
  pairs: Map<string, number>;
  initialBlockers: Point[][];
};

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
export function compilePhysicalStairRegion(input: PhysicalStairRegionInput): PhysicalStairRegion {
  let surfaces = input.surfaces;
  let floor = compilePhysicalStair({ ...input, obstacles: [] });
  if (floor.navigation.floor_patches) {
    const parts = surfaces.flatMap((surface) => {
      try {
        return [compilePhysicalStairRegion({ ...input, surfaces: [surface], doors: [] })];
      } catch (error) {
        if (
          input.frame &&
          error instanceof Error &&
          error.message === "Physical stair has no floor inside the export frame"
        )
          return [];
        throw error;
      }
    });
    const pairs = new Map<string, number>();
    for (const part of parts)
      for (const name of part.pairs.keys()) {
        if (pairs.has(name)) continue;
        if (pairs.size >= 16) throw new MovementTransitionLimit(name);
        pairs.set(name, pairs.size);
      }
    const world = (points: Point[], plane: HeightPlane): Vec3[] =>
      points.map(([x, y]) => [x, y, planeHeight(plane, [x, y])]);
    const compiled = compilePhysicalStairArea({
      surfaces: parts.map((part) => ({
        polygon: world(part.navigation.boundary, part.navigation.plane),
        holes: [],
      })),
      doors: input.doors,
      obstacles: parts.flatMap((part) =>
        part.navigation.obstacles.map((obstacle) => {
          const oldState = part.area.obstacles[obstacle.motion_obstacle]!.state_id;
          let stateId = 0;
          for (const [name, index] of part.pairs) {
            stateId |= ((oldState >>> (2 * index)) & 3) << (2 * pairs.get(name)!);
          }
          return {
            stateId: stateId >>> 0,
            polygon: world(obstacle.polygon, part.navigation.plane),
          };
        }),
      ),
    });
    return {
      ...compiled,
      pairs,
      initialBlockers: compiled.navigation.obstacles
        .filter((obstacle) => {
          const state = compiled.area.obstacles[obstacle.motion_obstacle]!.state_id;
          return state === 0 || (state & 0x55555555) !== 0;
        })
        .map((obstacle) => obstacle.polygon),
    };
  }
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
      const contour = [clearance.polygon, ...clearance.holes];
      try {
        regions = clipping.difference(regions, contour);
      } catch (error) {
        if (
          !(error instanceof Error) ||
          !error.message.startsWith("Unable to complete output ring")
        )
          throw error;
        // Nearly coincident contacts can defeat floating-point ring stitching.
        // Retry that failure at fixed precision without discarding collision.
        regions = fixedPolygonBoolean("difference", regions, [contour]);
      }
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
