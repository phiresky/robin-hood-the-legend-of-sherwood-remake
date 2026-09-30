import type { Polygon, MultiPolygon } from "polygon-clipping";
import { fixedClipping as clipping } from "./fixed-polygon-boolean.ts";
import earcut, { flatten } from "earcut";
import type { Point, SightObstacle } from "./level.ts";
import { simplifyMotionRing } from "./motion-quantization.ts";
import { equivalentProjectionPlanes } from "./native-projection-plane.ts";

export interface ProjectionMaterialSupport {
  polygon: Point[];
  /** Existing physical receiver; partitioning must not replace it with thin geometry. */
  obstacleIndex?: number;
  planePoints?: SightObstacle["projection_plane"];
  footprint?: Point[];
  defaultMaterial: number;
  materialIndices: number[];
  /** Equivalent placed definitions can have different regenerated indices. */
  materialSignature?: string;
  explicit: boolean;
  owner?: string;
  priority?: number;
  tiePriority?: number;
}
const shape = (points: Point[]): Polygon => [[...points, points[0]!]];
type Bounds = [number, number, number, number];
const geometryBounds = (geometry: MultiPolygon): Bounds => {
  const bounds: Bounds = [Infinity, Infinity, -Infinity, -Infinity];
  for (const polygon of geometry)
    for (const [x, y] of polygon[0]!) {
      bounds[0] = Math.min(bounds[0], x);
      bounds[1] = Math.min(bounds[1], y);
      bounds[2] = Math.max(bounds[2], x);
      bounds[3] = Math.max(bounds[3], y);
    }
  return bounds;
};
const area = (polygons: MultiPolygon) =>
  polygons.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce(
        (total, ring, index) =>
          total +
          ((index ? -1 : 1) *
            Math.abs(
              ring.reduce((a, p, i) => {
                const q = ring[(i + 1) % ring.length]!;
                return a + p[0] * q[1] - q[0] * p[1];
              }, 0),
            )) /
            2,
        0,
      ),
    0,
  );

/** Keep material boundaries independent of the merged navigation region. */
export function partitionProjectionMaterials(
  boundary: Point[],
  supports: ProjectionMaterialSupport[],
  warnings: string[] = [],
): ProjectionMaterialSupport[] {
  if (!supports.some((support) => support.explicit))
    return [{ polygon: boundary, defaultMaterial: 0, materialIndices: [], explicit: false }];
  const members: { support: ProjectionMaterialSupport; geometry: MultiPolygon; bounds: Bounds }[] =
    [];
  for (const support of supports) {
    if (!support.explicit) continue;
    const coverage = support.footprint
      ? clipping.union(shape(support.polygon), shape(support.footprint))
      : shape(support.polygon);
    const geometry = clipping.intersection(shape(boundary), coverage);
    if (!geometry.length) continue;
    members.push({ support, geometry, bounds: geometryBounds(geometry) });
  }
  for (let index = 0; index < members.length; index++) {
    const member = members[index]!;
    for (let otherIndex = index + 1; otherIndex < members.length; otherIndex++) {
      const other = members[otherIndex]!;
      // Bounds use already clipped fixed-point geometry, so touching boxes cannot
      // hide positive-area intersections through a rounding discrepancy.
      if (
        member.bounds[2] <= other.bounds[0] ||
        other.bounds[2] <= member.bounds[0] ||
        member.bounds[3] <= other.bounds[1] ||
        other.bounds[3] <= member.bounds[1]
      )
        continue;
      const mixedReceivers =
        (member.support.obstacleIndex !== undefined) !==
        (other.support.obstacleIndex !== undefined);
      const conflictingMaterials =
        !(member.support.owner && member.support.owner === other.support.owner) &&
        (member.support.priority ?? 0) === (other.support.priority ?? 0) &&
        (member.support.tiePriority ?? 0) === (other.support.tiePriority ?? 0) &&
        (member.support.defaultMaterial !== other.support.defaultMaterial ||
          !equivalentProjectionPlanes(member.support.planePoints, other.support.planePoints) ||
          (member.support.materialSignature ?? JSON.stringify(member.support.materialIndices)) !==
            (other.support.materialSignature ?? JSON.stringify(other.support.materialIndices)));
      if (!mixedReceivers && !conflictingMaterials) continue;
      const overlap = area(clipping.intersection(member.geometry, other.geometry));
      if (overlap <= 1e-7) continue;
      if (mixedReceivers)
        throw new Error(
          "Overlapping physical and generated receivers require explicit volumes for both surfaces",
        );
      if (conflictingMaterials)
        throw new Error(
          `Overlapping receiving surfaces have conflicting projection materials: ${member.support.owner ?? "unnamed"} and ${other.support.owner ?? "unnamed"}`,
          {
            cause: {
              overlap,
              supports: [structuredClone(member.support), structuredClone(other.support)],
            },
          },
        );
    }
  }
  members.sort(
    (a, b) =>
      (b.support.priority ?? 0) - (a.support.priority ?? 0) ||
      (b.support.tiePriority ?? 0) - (a.support.tiePriority ?? 0),
  );
  let remaining: MultiPolygon = [shape(boundary)];
  for (const member of members) {
    member.geometry = clipping.intersection(remaining, member.geometry);
    remaining = clipping.difference(remaining, member.geometry);
  }
  // A merged motion boundary can enclose gaps between receiving supports.
  // Default material is supplied only by an authored implicit surface, not by
  // the absence of an explicit receiver (which must remain uncovered).
  const implicit = supports.filter((support) => !support.explicit);
  if (implicit.length)
    members.push({
      support: { polygon: boundary, defaultMaterial: 0, materialIndices: [], explicit: false },
      bounds: geometryBounds([shape(boundary)]),
      geometry: clipping.intersection(
        remaining,
        clipping.union(
          shape(implicit[0]!.polygon),
          ...implicit.slice(1).map((s) => shape(s.polygon)),
        ),
      ),
    });
  return members.flatMap(({ support, geometry }) =>
    geometry.flatMap((polygon) => {
      const rings = polygon.map((ring) => simplifyMotionRing(ring));
      if (rings[0]!.length < 3) {
        warnings.push("Receiving material partition collapsed to zero area and was omitted.");
        return [];
      }
      for (let index = rings.length - 1; index > 0; index--)
        if (rings[index]!.length < 3) {
          warnings.push(
            "Receiving material partition hole collapsed to zero area and was omitted.",
          );
          rings.splice(index, 1);
        }
      if (rings.length === 1) return [{ ...support, polygon: rings[0]! }];
      const { vertices, holes, dimensions } = flatten(rings);
      const indices = earcut(vertices, holes, dimensions);
      if (!indices.length) throw new Error("Cannot triangulate receiving material region");
      const pieces: ProjectionMaterialSupport[] = [];
      for (let i = 0; i < indices.length; i += 3)
        pieces.push({
          ...support,
          polygon: indices
            .slice(i, i + 3)
            .map((index): Point => [vertices[index * 2]!, vertices[index * 2 + 1]!]),
        });
      return pieces;
    }),
  );
}
