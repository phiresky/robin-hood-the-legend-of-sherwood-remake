import clipping, { type MultiPolygon, type Polygon } from "polygon-clipping";

/** Partition one movement sector using projection priority, preserving source-order ties. */
export function partitionRecoverySurfaces(
  boundary: Polygon,
  holes: Polygon[],
  supports: { polygon: Polygon; maximumHeight: number }[],
  preserveSingleLiftBoundary = false,
) {
  const free = holes.length ? clipping.difference(boundary, ...holes) : [boundary];
  if (preserveSingleLiftBoundary) {
    if (supports.length !== 1)
      throw new Error("A complete lift contour requires one receiving owner");
    // Movement clearance can extend beyond the visible receiving footprint.
    // Keep that contour on its sole asset; receiving geometry stays independent.
    return { surfaces: [free], ground: [] };
  }
  const surfaces: MultiPolygon[] = supports.map(() => []);
  let remaining = free;
  const priority = supports
    .map((support, index) => ({ ...support, index }))
    .sort((a, b) => b.maximumHeight - a.maximumHeight || a.index - b.index);
  for (const support of priority) {
    surfaces[support.index] = clipping.intersection(remaining, support.polygon);
    remaining = clipping.difference(remaining, support.polygon);
  }
  return { surfaces, ground: remaining };
}
