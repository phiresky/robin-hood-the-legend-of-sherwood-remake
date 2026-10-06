import clipping, { type MultiPolygon, type Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import type { NavigationPiece } from "./assemble-navigation-regions.ts";
import { simplifyMotionRing } from "./motion-quantization.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { restoreReceivingBoundary } from "./restore-receiving-boundary.ts";

export function motionBoundsKey(points: Point[]): string {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const [x, y] of points) {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  }
  return `${minX},${minY},${maxX},${maxY}`;
}
export function indexPreciseBlockers(contours: Point[][]) {
  const index = new Map<string, { exact: Point[]; rounded: Point[] }[]>();
  for (const contour of contours) {
    const exact = simplifyMotionRing(contour, 2 / 1048576);
    if (!exact.some((point) => point.some((v) => v !== Math.round(v)))) continue;
    const rounded = simplifyMotionRing(
      exact.map(([x, y]): Point => [Math.round(x), Math.round(y)]),
    );
    if (rounded.length < 3) continue;
    const key = motionBoundsKey(rounded);
    const bucket = index.get(key) ?? [];
    bucket.push({ exact, rounded });
    index.set(key, bucket);
  }
  return index;
}
/** A gap can become an enclosed obstacle only after separate floor pieces join.
 * Derive its exact blocked coverage from the placed floors and their own holes,
 * rather than from any one piece's obstacle list.
 */
export function joinedBlockedCoverage(pieces: NavigationPiece[], frame: Polygon): MultiPolygon {
  return fixedPolygonBoolean("difference", frame, [joinedFloorCoverage(pieces)]);
}

/** Retain a landing seam even when its motion region spans several receiving planes. */
export function joinedReceivingBoundary(
  pieces: NavigationPiece[],
  boundary: Point[],
): Point[] | undefined {
  // Motion regions merge outer contours before compiling holes as separate
  // obstacles. Subtracting those holes here would change the boundary identity.
  const floors = fixedPolygonBoolean(
    "union",
    pieces.map((p) => [p.receivingPolygon ?? p.polygon]),
  );
  const candidates = (
    indexPreciseBlockers(floors.map((p) => p[0]!)).get(motionBoundsKey(boundary)) ?? []
  ).filter(({ rounded }) => clipping.xor([rounded], [boundary]).length === 0);
  return candidates.length === 1
    ? candidates[0]!.exact
    : restoreReceivingBoundary(boundary, floors);
}

function joinedFloorCoverage(pieces: NavigationPiece[]): MultiPolygon {
  const floors = pieces.flatMap((piece) => {
    const exactHoles = indexPreciseBlockers(piece.preciseBlockers ?? []);
    const holes = piece.blockers.map((hole) => {
      const matches = (exactHoles.get(motionBoundsKey(hole)) ?? []).filter(
        ({ rounded }) => clipping.xor([rounded], [hole]).length === 0,
      );
      return [matches.length === 1 ? matches[0]!.exact : hole];
    });
    return fixedPolygonBoolean("difference", [piece.receivingPolygon ?? piece.polygon], holes);
  });
  return fixedPolygonBoolean("union", floors);
}
