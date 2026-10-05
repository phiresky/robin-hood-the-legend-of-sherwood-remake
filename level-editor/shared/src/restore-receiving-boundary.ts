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
  const restored: Point[][] = [];
  for (const source of sources) {
    const outer = simplifyMotionRing(source[0]!, 2 / 1048576);
    const edges = outer.map((a, i): Edge => ({ a, b: outer[(i + 1) % outer.length]! }));
    const matches: Edge[] = [];
    for (const [i, vertex] of boundary.entries()) {
      const next = boundary[(i + 1) % boundary.length]!;
      const matching = edges.filter((edge) => edgePoint(edge, vertex) && edgePoint(edge, next));
      if (matching.length !== 1) break;
      matches.push(matching[0]!);
    }
    if (matches.length !== boundary.length) continue;
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
    const clipped = clipping.intersection([candidate], [outer]);
    if (clipped.length !== 1 || clipped[0]!.length !== 1) continue;
    const rounded = quantizeGeneratedMotionPolygon(
      clipped[0]!,
      Math.round,
      "Receiving boundary",
      [],
    );
    if (!rounded || clipping.xor(rounded, [boundary]).length) continue;
    restored.push(simplifyMotionRing(clipped[0]![0]!));
  }
  return restored.length === 1 ? restored[0] : undefined;
}
