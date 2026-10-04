import type { NavigationRegion } from "./assemble-navigation-regions.ts";

/** Remove plane layers left empty by navigation joins before assigning runtime references. */
export function compactNavigationLayers(regions: NavigationRegion[]): number {
  // Zero retains ground lookup semantics even when all its authored walking space is blocked.
  const ordinary = [
    ...new Set([0, ...regions.filter((region) => !region.lift).map((region) => region.layer)]),
  ].sort((a, b) => a - b);
  const remap = new Map(ordinary.map((layer, index) => [layer, index]));
  // Independent lifts can overlap in projection at different elevations.
  // Sharing one layer would make their perimeters block each other's routes.
  const liftLayers = new Map<string, number>();
  for (const region of regions) {
    if (region.lift) {
      let layer = liftLayers.get(region.lift);
      if (layer === undefined) {
        layer = ordinary.length + liftLayers.size;
        liftLayers.set(region.lift, layer);
      }
      region.layer = layer;
    } else region.layer = remap.get(region.layer)!;
    for (const piece of region.pieces) piece.layer = region.layer;
  }
  regions.sort((a, b) => a.layer - b.layer);
  return ordinary.length + Math.max(0, liftLayers.size - 1);
}
