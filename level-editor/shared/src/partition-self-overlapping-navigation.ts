import {
  assembleNavigationRegions,
  type NavigationPiece,
  type NavigationRegion,
} from "./assemble-navigation-regions.ts";
import type { NavigationConnection } from "./assemble-navigation-joins.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { planeHeight } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { projectStairClearance } from "./project-stair-clearance.ts";
import { NAVIGATION_HALF_DIAGONAL } from "./navigation-footprint.ts";

export interface PartitionPassage {
  from: NavigationRegion;
  to: NavigationRegion;
  connection: NavigationConnection;
}

/** Prepare separate floors and matching-height seams; callers must validate and emit passages. */
export function partitionSelfOverlappingNavigation(
  region: NavigationRegion,
  warnings: string[],
): { regions: NavigationRegion[]; passages: PartitionPassage[] } {
  if (region.lift) return { regions: [region], passages: [] };
  const folded = region.pieces.some((piece) => 1 + piece.plane[1] < 0);
  const pieces = region.pieces.map((piece) => {
    if (!folded || !piece.worldPlane || !piece.receivingPolygon) return piece;
    const boundary = piece.receivingPolygon.map((p): Point => [
      p[0],
      p[1] + planeHeight(piece.plane, p),
    ]);
    const corrected = projectStairClearance(boundary, piece.worldPlane, true, {
      movementHalf: NAVIGATION_HALF_DIAGONAL,
    });
    if (!corrected) return piece;
    const rounded = quantizeGeneratedMotionPolygon(
      [corrected, ...piece.blockers],
      Math.round,
      "Folded navigation clearance",
      warnings,
    );
    return rounded ? { ...piece, polygon: rounded[0]!, blockers: rounded.slice(1) } : piece;
  });
  const corrected = pieces.some((piece, i) => piece !== region.pieces[i]);
  if (corrected) {
    region = { ...region, pieces };
    if (pieces.length === 1)
      return { regions: assembleNavigationRegions(pieces, warnings), passages: [] };
  }
  if (region.pieces.length < 2) return { regions: [region], passages: [] };
  const bounds = region.pieces.map((piece) => {
    const polygon = piece.receivingPolygon ?? piece.polygon;
    return [
      Math.min(...polygon.map((p) => p[0])),
      Math.min(...polygon.map((p) => p[1])),
      Math.max(...polygon.map((p) => p[0])),
      Math.max(...polygon.map((p) => p[1])),
    ];
  });
  const coverage = new Map<number, ReturnType<typeof fixedPolygonBoolean>>();
  const free = (i: number) => {
    let result = coverage.get(i);
    if (!result) {
      const piece = region.pieces[i]!;
      result = fixedPolygonBoolean(
        "difference",
        [[piece.receivingPolygon ?? piece.polygon]],
        piece.blockers.map((hole) => [hole]),
      );
      coverage.set(i, result);
    }
    return result;
  };
  const conflicts = region.pieces.map(() => new Set<number>());
  const ordered = bounds.map((_, i) => i).sort((a, b) => bounds[a]![0]! - bounds[b]![0]!);
  let active: number[] = [];
  for (const i of ordered) {
    active = active.filter((j) => bounds[j]![2]! > bounds[i]![0]!);
    for (const j of active) {
      const a = region.pieces[i]!,
        b = region.pieces[j]!;
      if (a.plane.every((n, k) => Math.abs(n - b.plane[k]!) < 1e-7)) continue;
      const ab = bounds[i]!,
        bb = bounds[j]!;
      if (ab[0]! >= bb[2]! || bb[0]! >= ab[2]! || ab[1]! >= bb[3]! || bb[1]! >= ab[3]!) continue;
      const overlap = fixedPolygonBoolean("intersection", free(i), [free(j)]);
      if (
        !overlap.some((polygon) =>
          polygon[0]!.some(
            (point) => Math.abs(planeHeight(a.plane, point) - planeHeight(b.plane, point)) > 1e-4,
          ),
        )
      )
        continue;
      conflicts[i]!.add(j);
      conflicts[j]!.add(i);
    }
    active.push(i);
  }
  if (conflicts.every((neighbors) => neighbors.size === 0))
    return {
      regions: corrected ? assembleNavigationRegions(pieces, warnings) : [region],
      passages: [],
    };
  const groups: number[][] = [];
  for (let i = 0; i < region.pieces.length; i++) {
    let group = groups.find((members) => members.every((j) => !conflicts[i]!.has(j)));
    if (!group) {
      group = [];
      groups.push(group);
    }
    group.push(i);
  }
  // Reassemble each group independently without changing authored region identities.
  // Disconnected components need individual native sectors even in the same group.
  let collapsed = false;
  const regions = groups.flatMap((members) =>
    assembleNavigationRegions(
      members.flatMap((i) => {
        const piece = structuredClone(region.pieces[i]!);
        const rounded = quantizeGeneratedMotionPolygon(
          [piece.polygon, ...piece.blockers],
          Math.round,
          "Partitioned navigation receiver",
          warnings,
        );
        if (!rounded) {
          collapsed = true;
          return [];
        }
        return [{ ...piece, polygon: rounded[0]!, blockers: rounded.slice(1) }];
      }),
      warnings,
    ),
  );
  // A failed split must retain every original receiver for best-effort output.
  if (collapsed) return { regions: [region], passages: [] };
  const passages: PartitionPassage[] = [];
  const seen = new Set<string>();
  for (let i = 0; i < regions.length; i++)
    for (let j = 0; j < i; j++) {
      const from = regions[j]!,
        to = regions[i]!;
      for (const a of from.pieces)
        for (const b of to.pieces)
          for (const edge of matchingSeams(a, b)) {
            const key = `${j}/${i}/${JSON.stringify(edge)}`;
            if (seen.has(key)) continue;
            seen.add(key);
            const name = a.navigationRegion ?? b.navigationRegion ?? "unnamed";
            passages.push({
              from,
              to,
              connection: {
                from: { region: name, owner: name, edge },
                to: { region: name, owner: name, edge: [edge[1], edge[0]] },
                overlap: [0, 1],
              },
            });
          }
    }
  for (const { from, to, connection } of passages)
    for (const [x, y, z] of connection.from.edge) {
      for (const ring of [from.polygon, ...from.blockers, to.polygon, ...to.blockers])
        alignRoundedSeam(ring, [x, y - z]);
    }
  return { regions, passages };
}

/** A short shared edge and its longer neighbor must round the same seam vertices. */
function alignRoundedSeam(polygon: Point[], precise: Point): void {
  const rounded: Point = [Math.round(precise[0]), Math.round(precise[1])];
  if (polygon.some((p) => p[0] === rounded[0] && p[1] === rounded[1])) return;
  let nearest: { index: number; distance: number } | undefined;
  for (const [index, a] of polygon.entries()) {
    const b = polygon[(index + 1) % polygon.length]!;
    const dx = b[0] - a[0],
      dy = b[1] - a[1];
    const length2 = dx * dx + dy * dy;
    if (length2 === 0) continue;
    const t = ((precise[0] - a[0]) * dx + (precise[1] - a[1]) * dy) / length2;
    if (t <= 0 || t >= 1) continue;
    const distance = Math.hypot(precise[0] - a[0] - t * dx, precise[1] - a[1] - t * dy);
    // Only reconcile ordinary nearest-integer boundary displacement. Larger
    // cracks require authored geometry repair, not an invented connecting floor.
    if (distance <= Math.SQRT1_2 + 1e-4 && (!nearest || distance < nearest.distance))
      nearest = { index, distance };
  }
  if (nearest) polygon.splice(nearest.index + 1, 0, rounded);
}

function matchingSeams(a: NavigationPiece, b: NavigationPiece): [Vec3, Vec3][] {
  const result: [Vec3, Vec3][] = [];
  const edges = (piece: NavigationPiece) => {
    const rings = [
      piece.receivingPolygon ?? piece.polygon,
      ...(piece.preciseBlockers ?? []),
      ...piece.blockers,
    ];
    return rings.flatMap((polygon, index) => {
      const area = polygon.reduce((sum, p, i) => {
        const q = polygon[(i + 1) % polygon.length]!;
        return sum + p[0] * q[1] - p[1] * q[0];
      }, 0);
      // Hole boundaries put the walkable side outside their ring. Their seams
      // can meet raised floors just like an exterior terrain edge can.
      const direction = (index === 0 ? 1 : -1) * area * (1 + piece.plane[1]);
      return polygon.map((p, i): [Point, Point] => {
        const q = polygon[(i + 1) % polygon.length]!;
        return direction >= 0 ? [p, q] : [q, p];
      });
    });
  };
  for (const [p, q] of edges(a)) {
    const dx = q[0] - p[0],
      dy = q[1] - p[1];
    const length = Math.hypot(dx, dy);
    if (length < 1e-4) continue;
    for (const [r, s] of edges(b)) {
      if ([r, s].some((v) => Math.abs(dx * (v[1] - p[1]) - dy * (v[0] - p[0])) / length > 1e-4))
        continue;
      const offset = (v: Point) => ((v[0] - p[0]) * dx + (v[1] - p[1]) * dy) / length;
      const r0 = offset(r),
        s0 = offset(s);
      // Both contours put their world-space interior on opposite sides of the seam.
      if (r0 <= s0) continue;
      const start = Math.max(0, s0),
        end = Math.min(length, r0);
      if (end - start < 1e-4) continue;
      const points = [start, end].map((t): Point => [
        p[0] + (dx * t) / length,
        p[1] + (dy * t) / length,
      ]);
      if (points.some((v) => Math.abs(planeHeight(a.plane, v) - planeHeight(b.plane, v)) > 1e-4))
        continue;
      const world = (v: Point): Vec3 => {
        const z = planeHeight(a.plane, v);
        return [v[0], v[1] + z, z];
      };
      result.push([world(points[0]!), world(points[1]!)]);
    }
  }
  return result;
}
