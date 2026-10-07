import earcut, { flatten } from "earcut";
import polygonClipping, { type MultiPolygon } from "polygon-clipping";
import type { NavigationPiece } from "./assemble-navigation-regions.ts";
import type { Point } from "./level.ts";
import { heightPlane, planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { movementVolumeHeightSlice } from "./movement-volume-height-slice.ts";
import { quantizeGeneratedMotionPolygon, simplifyMotionRing } from "./motion-quantization.ts";
import { preserveMovementBoundary } from "./preserve-movement-boundary.ts";
import { assembleMovementContour } from "./assemble-movement-contour.ts";
import { fixedClipping } from "./fixed-polygon-boolean.ts";
import { subtractMovementCollision } from "./subtract-movement-collision.ts";

export interface PlacedTransitionBlocker {
  transition: string;
  applied: boolean;
  /** An unavailable control may retain its initial barrier as permanent geometry. */
  fixed?: boolean;
  polygon: Point[];
  holes: Point[][];
  plane: HeightPlane;
  movementContour?: string;
  terrainVolume?: {
    polygon: Point[];
    holes: Point[][];
    plane: HeightPlane;
    below: number;
    above: number;
  };
}

export class MovementTransitionLimit extends Error {
  readonly transition: string;
  constructor(transition: string) {
    super("More than 16 independent movement transitions affect one navigation area");
    this.transition = transition;
  }
}

/** Intersect an authored vertical volume with a receiving plane before projecting into movement space. */
function terrainSlice(
  volume: NonNullable<PlacedTransitionBlocker["terrainVolume"]>,
  receiver: { polygon: Point[]; plane: HeightPlane; worldPlane?: HeightPlane },
  physical: boolean,
): MultiPolygon {
  const plane =
    receiver.worldPlane ??
    heightPlane(
      receiver.polygon.map(([x, y]) => {
        const z = planeHeight(receiver.plane, [x, y]);
        return [x, y + z, z];
      }),
    );
  const slice = movementVolumeHeightSlice(
    volume.polygon,
    plane,
    [volume.plane[0], volume.plane[1], volume.plane[2] - volume.below],
    [volume.plane[0], volume.plane[1], volume.plane[2] + volume.above],
    true,
  );
  if (slice.length < 3) return [];
  if (physical) return polygonClipping.intersection([volume.polygon, ...volume.holes], [slice]);
  return fixedClipping
    .intersection([volume.polygon, ...volume.holes], [slice])
    .map((polygon) =>
      polygon.map((ring) => ring.map(([x, y]): Point => [x, y - planeHeight(plane, [x, y])])),
    );
}

/** Allocate independent bit pairs per assembled area; no source-map state IDs survive. */
export function compileTransitionObstacles(
  boundary: Point[],
  holes: Point[][],
  plane: HeightPlane,
  blockers: PlacedTransitionBlocker[],
  warnings: string[],
  receivers?: NavigationPiece[],
  preserveBoundary = false,
  worldPlane?: HeightPlane,
) {
  return compileTransitionObstaclesInFrame(
    boundary,
    holes,
    plane,
    blockers,
    warnings,
    receivers,
    preserveBoundary,
    worldPlane,
    false,
  );
}

/** All contours and receiving planes are in world XY; projection is deferred to area emission. */
export function compilePhysicalTransitionObstacles(
  boundary: Point[],
  holes: Point[][],
  plane: HeightPlane,
  blockers: PlacedTransitionBlocker[],
) {
  return compileTransitionObstaclesInFrame(
    boundary,
    holes,
    plane,
    blockers,
    [],
    undefined,
    false,
    plane,
    true,
  );
}

function compileTransitionObstaclesInFrame(
  boundary: Point[],
  holes: Point[][],
  plane: HeightPlane,
  blockers: PlacedTransitionBlocker[],
  warnings: string[],
  receivers: NavigationPiece[] | undefined,
  preserveBoundary: boolean,
  worldPlane: HeightPlane | undefined,
  physical: boolean,
) {
  const pairs = new Map<string, number>();
  const obstacles: { state_id: number; polygon: { points: Point[] } }[] = [];
  const initial: Point[][] = [];
  // Movement obstacles can cross the area's outer contour.
  const coverage = (polygon: Point[], blockers: Point[][]): MultiPolygon =>
    blockers.length
      ? subtractMovementCollision(
          [[polygon]],
          blockers.map((b) => [b]),
        )
      : [[polygon]];
  const walkable = blockers.length ? coverage(boundary, holes) : [];
  const groups: {
    transition: string;
    applied: boolean;
    fixed?: boolean;
    movementContour?: string;
    regions: MultiPolygon;
  }[] = [];
  for (const blocker of blockers) {
    const samePlane = (plane: HeightPlane) =>
      plane.every((n, i) => Math.abs(n - blocker.plane[i]!) < 1e-7);
    if (!blocker.terrainVolume && !receivers && !samePlane(plane)) continue;
    let clipped: MultiPolygon;
    if (blocker.terrainVolume) {
      const fragments = (
        receivers ?? [{ polygon: boundary, blockers: holes, plane, worldPlane }]
      ).flatMap((receiver) => {
        const slice = terrainSlice(blocker.terrainVolume!, receiver, physical);
        return slice.length
          ? (physical ? polygonClipping : fixedClipping).intersection(
              coverage(receiver.polygon, receiver.blockers),
              walkable,
              slice,
            )
          : [];
      });
      clipped = fragments.length
        ? (physical ? polygonClipping : fixedClipping).union(fragments[0]!, ...fragments.slice(1))
        : [];
    } else if (receivers) {
      const fragments = receivers
        .filter((r) => samePlane(r.plane))
        .flatMap((r) =>
          polygonClipping.intersection(coverage(r.polygon, r.blockers), walkable, [
            blocker.polygon,
            ...blocker.holes,
          ]),
        );
      clipped = fragments.length ? polygonClipping.union(fragments[0]!, ...fragments.slice(1)) : [];
    } else clipped = polygonClipping.intersection(walkable, [blocker.polygon, ...blocker.holes]);
    // Keep the complete contour after testing overlap. Rounding its clipped
    // intersections would change narrow routes along the movement envelope.
    if (preserveBoundary && clipped.length && !blocker.terrainVolume) {
      const otherPlanes = receivers?.filter((r) => !samePlane(r.plane)) ?? [];
      clipped = otherPlanes.length
        ? polygonClipping.difference(
            [blocker.polygon, ...blocker.holes],
            otherPlanes.map((r) => [r.polygon]),
          )
        : [[blocker.polygon, ...blocker.holes]];
    }
    let group =
      blocker.movementContour === undefined
        ? undefined
        : groups.find(
            (g) =>
              g.transition === blocker.transition &&
              g.applied === blocker.applied &&
              g.movementContour === blocker.movementContour,
          );
    if (!group) {
      group = {
        transition: blocker.transition,
        applied: blocker.applied,
        fixed: blocker.fixed,
        movementContour: blocker.movementContour,
        regions: [],
      };
      groups.push(group);
    }
    group.regions.push(...clipped);
  }
  for (const blocker of groups) {
    if (!blocker.regions.length) continue;
    let clipped =
      blocker.movementContour === undefined
        ? blocker.regions
        : assembleMovementContour(blocker.regions);
    if (preserveBoundary)
      clipped = preserveMovementBoundary(boundary, clipped, warnings).blockers.map((points) => [
        points,
      ]);
    for (const region of clipped) {
      const rounded = physical
        ? region
        : quantizeGeneratedMotionPolygon(region, Math.round, blocker.transition, warnings);
      if (!rounded) continue;
      // Clipping can leave a zero-width backtracking spike on a shared seam.
      // Clean physical obstacle rings too: rounding their coordinates in the
      // runtime can otherwise turn that spike into a blocking edge.
      const rings = rounded.map((ring) => simplifyMotionRing(ring));
      let pieces: Point[][];
      if (rings.length === 1) pieces = [rings[0]!];
      else {
        const { vertices, holes, dimensions } = flatten(rings);
        const indices = earcut(vertices, holes, dimensions);
        if (!indices.length)
          throw new Error(`${blocker.transition}: failed to triangulate movement blocker holes`);
        pieces = [];
        for (let i = 0; i < indices.length; i += 3)
          pieces.push(
            indices
              .slice(i, i + 3)
              .map((index) => [vertices[index * 2]!, vertices[index * 2 + 1]!]),
          );
      }
      for (const points of pieces) {
        const origin = physical ? points[0]! : [0, 0];
        const area = points.reduce((sum, p, i) => {
          const q = points[(i + 1) % points.length]!;
          return (
            sum +
            (p[0] - origin[0]!) * (q[1] - origin[1]!) -
            (q[0] - origin[0]!) * (p[1] - origin[1]!)
          );
        }, 0);
        // Clipping tangent world-space solids can leave numerical slivers.
        // Drop them before allocating control identities; no collision survives.
        if (physical && (points.length < 3 || Math.abs(area) < 1e-8)) continue;
        if (points.length < 3 || Math.abs(area) < (physical ? 1e-8 : 1))
          throw new Error(`${blocker.transition}: degenerate state-dependent movement blocker`);
        let pair = pairs.get(blocker.transition);
        if (!blocker.fixed && pair === undefined) {
          pair = pairs.size;
          if (pair >= 16) throw new MovementTransitionLimit(blocker.transition);
          pairs.set(blocker.transition, pair);
        }
        const state_id = blocker.fixed ? 0 : (1 << (2 * pair! + (blocker.applied ? 1 : 0))) >>> 0;
        if (area < 0) points.reverse();
        obstacles.push({ state_id, polygon: { points } });
        if (!blocker.applied) initial.push(points);
      }
    }
  }
  return { pairs, obstacles, initial };
}
