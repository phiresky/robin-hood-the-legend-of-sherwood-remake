import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { quantizeGeneratedMotionPolygon, simplifyMotionRing } from "./motion-quantization.ts";

type Edge = { a: Point; b: Point };

/** Locate the part of an authored edge inside a movement vertex's rounding cell. */
function edgePoint(edge: Edge, vertex: Point): Point | undefined {
  let low = 0,
    high = 1;
  const delta: Point = [edge.b[0] - edge.a[0], edge.b[1] - edge.a[1]];
  for (const axis of [0, 1] as const) {
    const from = vertex[axis] - 0.5,
      to = vertex[axis] + 0.5;
    if (delta[axis] === 0) {
      if (edge.a[axis] < from || edge.a[axis] > to) return undefined;
    } else {
      const a = (from - edge.a[axis]) / delta[axis],
        b = (to - edge.a[axis]) / delta[axis];
      low = Math.max(low, Math.min(a, b));
      high = Math.min(high, Math.max(a, b));
    }
  }
  if (low > high) return undefined;
  const length = delta[0] ** 2 + delta[1] ** 2;
  if (length === 0) return undefined;
  const projected =
    ((vertex[0] - edge.a[0]) * delta[0] + (vertex[1] - edge.a[1]) * delta[1]) / length;
  // A cell's positive half-grid edge belongs to the next integer. If the
  // closest point lands there, use the interior of the clipped edge interval.
  for (const t of [Math.max(low, Math.min(high, projected)), (low + high) / 2]) {
    const point: Point = [edge.a[0] + t * delta[0], edge.a[1] + t * delta[1]];
    if (point.every((value, axis) => Math.round(value) === vertex[axis])) return point;
  }
  return undefined;
}

/** Recover a split region's outer receiver without expanding the source contour.
 * New grid intersections can replace a narrow neck with separate regions. Each
 * output edge must have an unambiguous source edge through both rounding cells.
 * At new corners, join the two source edges inside that cell, then clip back to
 * the source. Accept only a single contour rounding to the exact output region.
 * Interior holes retain their separately compiled movement blockers.
 */
export function restoreReceivingBoundary(
  boundary: Point[],
  sources: MultiPolygon,
): Point[] | undefined {
  return restoreBoundary(boundary, sources, false);
}

/** Recover a rounded obstacle from any boundary of its exact blocked coverage,
 * including a notch that became an enclosed hole on the movement grid. */
export function restoreObstacleBoundary(
  boundary: Point[],
  sources: MultiPolygon,
): Point[] | undefined {
  return restoreBoundary(boundary, sources, true);
}

function restoreBoundary(
  boundary: Point[],
  sources: MultiPolygon,
  obstacle: boolean,
): Point[] | undefined {
  const restored: Point[][] = [];
  const contours = sources.flatMap((source) =>
    (obstacle ? source : [source[0]!]).map((contour) => ({
      contour,
      coverage: obstacle ? source : [contour],
    })),
  );
  for (const { contour, coverage } of contours) {
    const outer = simplifyMotionRing(contour, 2 / 1048576);
    const edges = outer.map((a, i): Edge => ({ a, b: outer[(i + 1) % outer.length]! }));
    const choices: Edge[][] = [];
    for (const [i, vertex] of boundary.entries()) {
      const next = boundary[(i + 1) % boundary.length]!;
      const matching = edges.filter((edge) => edgePoint(edge, vertex) && edgePoint(edge, next));
      if (!matching.length) break;
      choices.push(matching);
    }
    if (choices.length !== boundary.length) continue;
    // Short neighbouring edges may share rounding cells. For obstacles, retain
    // the union of valid reconstructions inside the same blocked coverage.
    // Bound the search; more complex ambiguity keeps integer collision.
    const combinations = choices.reduce((n, edges) => n * edges.length, 1);
    if (combinations > (obstacle ? 64 : 1)) continue;
    let paths: Edge[][] = [[]];
    for (const edges of choices)
      paths = paths.flatMap((path) => edges.map((edge) => [...path, edge]));
    const valid: MultiPolygon = [];
    for (const matches of paths) {
      const candidate: Point[] = [];
      for (const [i, vertex] of boundary.entries()) {
        const incoming = matches[(i + boundary.length - 1) % boundary.length]!;
        const outgoing = matches[i]!;
        const shared = [incoming.a, incoming.b].find(
          (point) =>
            [outgoing.a, outgoing.b].some(
              (other) => other[0] === point[0] && other[1] === point[1],
            ) && point.every((value, axis) => Math.round(value) === vertex[axis]),
        );
        if (shared) candidate.push([...shared]);
        else candidate.push(edgePoint(incoming, vertex)!, edgePoint(outgoing, vertex)!);
      }
      const clipped = clipping.intersection([candidate], obstacle ? coverage : [outer]);
      if (clipped.length !== 1 || clipped[0]!.length !== 1) continue;
      const rounded = quantizeGeneratedMotionPolygon(
        clipped[0]!,
        Math.round,
        "Receiving boundary",
        [],
      );
      if (!rounded || clipping.xor(rounded, [boundary]).length) continue;
      valid.push(clipped[0]!);
    }
    if (!valid.length) continue;
    if (!obstacle) {
      restored.push(simplifyMotionRing(valid[0]![0]!));
      continue;
    }
    const combined = clipping.union(valid[0]!, ...valid.slice(1));
    if (combined.length !== 1 || combined[0]!.length !== 1) continue;
    const rounded = quantizeGeneratedMotionPolygon(
      combined[0]!,
      Math.round,
      "Restored obstacle",
      [],
    );
    if (!rounded || clipping.xor(rounded, [boundary]).length) continue;
    restored.push(simplifyMotionRing(combined[0]![0]!, obstacle ? 2 / 1048576 : 0));
  }
  return restored.length === 1 ? restored[0] : undefined;
}
