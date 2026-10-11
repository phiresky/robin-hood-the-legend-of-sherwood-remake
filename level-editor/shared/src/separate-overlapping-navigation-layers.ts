import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { planeHeight } from "./gameplay-plane.ts";

/** A joined ramp may share its starting plane with a floor it later passes over. */
export function separateOverlappingNavigationLayers(regions: NavigationRegion[]): void {
  const ordinary = regions.filter((region) => !region.lift);
  const conflicts = new Map(ordinary.map((region) => [region, new Set<NavigationRegion>()]));
  const pieces = ordinary
    .flatMap((region) =>
      region.pieces.map((piece) => {
        const polygon = piece.receivingPolygon ?? piece.polygon;
        return {
          region,
          plane: piece.plane,
          polygon,
          minX: Math.min(...polygon.map((p) => p[0])),
          maxX: Math.max(...polygon.map((p) => p[0])),
          minY: Math.min(...polygon.map((p) => p[1])),
          maxY: Math.max(...polygon.map((p) => p[1])),
        };
      }),
    )
    .sort((a, b) => a.minX - b.minX);
  let active: typeof pieces = [];
  for (const current of pieces) {
    active = active.filter((other) => other.maxX > current.minX);
    for (const other of active) {
      if (
        current.region === other.region ||
        current.region.layer !== other.region.layer ||
        current.minY >= other.maxY ||
        current.maxY <= other.minY ||
        conflicts.get(current.region)!.has(other.region)
      )
        continue;
      if (current.plane.every((n, i) => Math.abs(n - other.plane[i]!) < 1e-7)) continue;
      const overlap = fixedPolygonBoolean("intersection", [[current.polygon]], [[other.polygon]]);
      if (
        !overlap.some((polygon) =>
          polygon[0]!.some(
            (point) =>
              Math.abs(planeHeight(current.plane, point) - planeHeight(other.plane, point)) > 1e-4,
          ),
        )
      )
        continue;
      conflicts.get(current.region)!.add(other.region);
      conflicts.get(other.region)!.add(current.region);
    }
    active.push(current);
  }
  let nextLayer = Math.max(0, ...ordinary.map((region) => region.layer)) + 1;
  const assigned = new Set<NavigationRegion>();
  const alternatives = new Map<number, number[]>();
  for (const region of ordinary) {
    const candidates = alternatives.get(region.layer) ?? [region.layer];
    alternatives.set(region.layer, candidates);
    const forbidden = new Set(
      [...conflicts.get(region)!]
        .filter((other) => assigned.has(other))
        .map((other) => other.layer),
    );
    let layer = candidates.find((candidate) => !forbidden.has(candidate));
    if (layer === undefined) {
      layer = nextLayer++;
      candidates.push(layer);
    }
    region.layer = layer;
    for (const piece of region.pieces) piece.layer = layer;
    assigned.add(region);
  }
}
