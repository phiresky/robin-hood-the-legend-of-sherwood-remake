import earcut from "earcut";
import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { simplifyMotionRing } from "./motion-quantization.ts";
import { footprintBoundaryHull, supportedFootprintCenters } from "./supported-footprint-centers.ts";

function connects(regions: MultiPolygon, entries: Point[]) {
  return regions.some((polygon) =>
    entries.every(
      (point) =>
        pointInGameplayPolygon(point, polygon[0]!, true) &&
        !polygon.slice(1).some((hole) => pointInGameplayPolygon(point, hole, true)),
    ),
  );
}

/**
 * Prepare body clearance along supported routes through a concave climb floor.
 * Only unobstructed floors may use this envelope. Every admitted body center
 * must remain on the original floor, including around concave route bends.
 */
export function climbRouteClearance(floor: Point[], entries: Point[]): Point[] | undefined {
  if (
    entries.length < 2 ||
    floor.length < 3 ||
    [...floor, ...entries].some((point) => point.some((value) => !Number.isFinite(value)))
  )
    return undefined;
  const region = clipping.union([floor]);
  if (region.length !== 1 || region[0]!.length !== 1) return undefined;
  const ring = region[0]![0]!.slice(0, -1);
  if (entries.some((point) => !pointInGameplayPolygon(point, ring, true))) return undefined;
  if (connects(supportedFootprintCenters(region, region, half), entries)) return undefined;
  // Different diagonals give different body envelopes at concave bends. Try a
  // bounded set of triangulations, accepting only a completely supported one.
  // Simplification affects route construction, never the support proof.
  for (const vertices of [ring, simplifyMotionRing(ring)])
    for (let offset = 0; offset < Math.min(vertices.length, 8); offset++) {
      const candidate = prepareRoute(
        region,
        [...vertices.slice(offset), ...vertices.slice(0, offset)],
        entries,
      );
      if (candidate) return candidate;
    }
  return undefined;
}

function prepareRoute(region: MultiPolygon, ring: Point[], entries: Point[]): Point[] | undefined {
  const indices = earcut(ring.flat());
  const triangles: number[][] = [];
  for (let i = 0; i < indices.length; i += 3) triangles.push(indices.slice(i, i + 3));
  if (!triangles.length) return undefined;
  const owners = entries.map((point) =>
    triangles.findIndex((triangle) =>
      pointInGameplayPolygon(
        point,
        triangle.map((index) => ring[index]!),
        true,
      ),
    ),
  );
  if (owners.includes(-1)) return undefined;
  const centroid = (vertices: number[]): Point => [
    vertices.reduce((sum, index) => sum + ring[index]![0], 0) / vertices.length,
    vertices.reduce((sum, index) => sum + ring[index]![1], 0) / vertices.length,
  ];
  const neighbours = triangles.map((triangle, i) =>
    triangles.flatMap((other, j) =>
      i !== j && other.filter((index) => triangle.includes(index)).length === 2 ? [j] : [],
    ),
  );
  const root = owners[0]!;
  const queue = [root];
  const parents = new Map<number, number | null>([[root, null]]);
  for (let i = 0; i < queue.length; i++)
    for (const next of neighbours[queue[i]!]!)
      if (!parents.has(next)) {
        parents.set(next, queue[i]!);
        queue.push(next);
      }
  const paths: Point[][] = [];
  for (let i = 1; i < entries.length; i++) {
    if (!parents.has(owners[i]!)) return undefined;
    const route: number[] = [];
    for (let at: number | null = owners[i]!; at !== null; at = parents.get(at)!) route.unshift(at);
    const points = [entries[0]!, centroid(triangles[route[0]!]!)];
    for (let j = 1; j < route.length; j++) {
      const shared = triangles[route[j - 1]!]!.filter((index) =>
        triangles[route[j]!]!.includes(index),
      );
      points.push(centroid(shared), centroid(triangles[route[j]!]!));
    }
    points.push(entries[i]!);
    paths.push(points);
  }
  // A small margin keeps roots off exact boundary contacts after native float
  // conversion. The final support test also checks this margin.
  const offsets: Point[] = [-1, 1].flatMap((x) =>
    [-1, 1].map((y): Point => [x * (half[0] + 0.001), y * (half[1] + 0.001)]),
  );
  const sweeps = paths.flatMap((points) =>
    points
      .slice(1)
      .map((point, i) => [
        footprintBoundaryHull(
          [points[i]!, point].flatMap(([x, y]) =>
            offsets.map(([dx, dy]): Point => [x + dx, y + dy]),
          ),
        ),
      ]),
  );
  const envelope = clipping.union(region, ...sweeps);
  if (envelope.length !== 1 || envelope[0]!.length !== 1) return undefined;
  const centers = supportedFootprintCenters(envelope, envelope, half);
  if (!connects(centers, entries) || clipping.difference(centers, region).length) return undefined;
  return envelope[0]![0]!.slice(0, -1);
}
