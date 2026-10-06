import earcut, { flatten } from "earcut";
import polygonClipping, { type Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { simplifyMotionRing, quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";

function coverageError(region: Polygon, pieces: Point[][]): number {
  // Union shared triangle edges before comparison. Fixed-point XOR can create
  // duplicate slivers when many coincident cuts meet a narrow hole.
  const delta = polygonClipping.xor(region, polygonClipping.union(pieces.map((p) => [p])));
  return delta.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce((sum, ring, index) => {
        const doubledArea = ring.reduce((area, point, i) => {
          const next = ring[(i + 1) % ring.length]!;
          return area + point[0] * next[1] - next[0] * point[1];
        }, 0);
        return sum + ((index ? -1 : 1) * Math.abs(doubledArea)) / 2;
      }, 0),
    0,
  );
}

/** Partition before snapping so each obstacle retains its exact outer and hole
 * edges. Merge triangles that collapse on the movement grid into a neighbour;
 * their authored collision must not disappear merely because it is subpixel. */
export function partitionPreciseMovementObstacles(region: Polygon, rounded: Polygon[]) {
  region = region.map((ring) => simplifyMotionRing(ring, 2 / 1048576));
  const exact = partitionMovementObstacles(region, true);
  const quantize = (ring: Point[]) =>
    quantizeGeneratedMotionPolygon([ring], Math.round, "Precise obstacle partition", []);
  for (;;) {
    const collapsed = exact.findIndex((ring) => !quantize(ring));
    if (collapsed === -1) break;
    let merged = false;
    for (let i = 0; i < exact.length; i++) {
      if (i === collapsed) continue;
      const union = polygonClipping.union([exact[collapsed]!], [exact[i]!]);
      if (union.length !== 1 || union[0]!.length !== 1) continue;
      exact[i] = simplifyMotionRing(union[0]![0]!);
      exact.splice(collapsed, 1);
      merged = true;
      break;
    }
    if (!merged) return undefined;
  }
  const grid = exact.map((ring) => simplifyMotionRing(quantize(ring)![0]!));
  // Changing the partition must not change any integer-grid collision.
  if (polygonClipping.xor(polygonClipping.union(grid.map((ring) => [ring])), rounded).length)
    return undefined;
  if (coverageError(region, exact) > 0.001) return undefined;
  return { exact, grid };
}

/** Native obstacles have one ring. Recovery may add temporary cuts before final union and snapping. */
export function partitionMovementObstacles(
  region: Polygon,
  allowNewVertices = false,
  depth = 0,
): Point[][] {
  const rings = region.map((ring) => simplifyMotionRing(ring));
  let pieces: Point[][];
  if (rings.length === 1) pieces = [rings[0]!];
  else {
    const { vertices, holes, dimensions } = flatten(rings);
    const indices = earcut(vertices, holes, dimensions);
    if (!indices.length) throw new Error("Cannot partition movement obstacle islands");
    pieces = [];
    for (let i = 0; i < indices.length; i += 3)
      pieces.push(
        indices.slice(i, i + 3).map((index) => [vertices[index * 2]!, vertices[index * 2 + 1]!]),
      );
  }
  const result = pieces.map((points) => {
    const area = points.reduce((sum, p, i) => {
      const q = points[(i + 1) % points.length]!;
      return sum + p[0] * q[1] - q[0] * p[1];
    }, 0);
    if (points.length < 3 || Math.abs(area) < 1e-8)
      throw new Error("Degenerate movement obstacle partition");
    return area < 0 ? points.reverse() : points;
  });
  const area = coverageError(region, result);
  if (area > 0.001) {
    // TODO: handle touching islands that cannot be triangulated faithfully on
    // the final integer grid without introducing new fractional vertices.
    if (!allowNewVertices || rings.length < 2 || depth >= 16)
      throw new Error(`Movement partition changed coverage by ${area}`, { cause: region });
    // Near-touching fractional holes can defeat triangulation. Recovery may
    // split through a hole; these intermediate cuts are unioned before snapping.
    const xs = rings[1]!.map((p) => p[0]);
    const x = (Math.min(...xs) + Math.max(...xs)) / 2;
    const points = rings.flat(),
      allX = points.map((p) => p[0]),
      allY = points.map((p) => p[1]);
    const left = Math.min(...allX) - 1,
      right = Math.max(...allX) + 1;
    const top = Math.min(...allY) - 1,
      bottom = Math.max(...allY) + 1;
    const split = [
      [left, x],
      [x, right],
    ].flatMap(([a, b]) =>
      fixedPolygonBoolean("intersection", region, [
        [
          [
            [a!, top],
            [b!, top],
            [b!, bottom],
            [a!, bottom],
          ],
        ],
      ]).flatMap((part) => partitionMovementObstacles(part, true, depth + 1)),
    );
    const error = coverageError(region, split);
    if (error > 0.001) throw new Error(`Movement split changed coverage by ${error}`);
    return split;
  }
  return result;
}
