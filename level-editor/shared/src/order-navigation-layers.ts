import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import { planeHeight } from "./gameplay-plane.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";

/** Prepare bottom-to-top click precedence from overlapping receiving geometry. */
export function orderNavigationLayers(regions: NavigationRegion[], warnings: string[]): void {
  const layers = [
    ...new Set(regions.filter((region) => !region.lift).map((region) => region.layer)),
  ].sort((a, b) => a - b);
  const edges = new Map(layers.map((layer) => [layer, new Set<number>()]));
  const incoming = new Map(layers.map((layer) => [layer, 0]));
  const pieces = regions
    .filter((region) => !region.lift)
    .flatMap((region) =>
      region.pieces.map((piece) => {
        const polygon = piece.receivingPolygon ?? piece.polygon;
        return {
          layer: region.layer,
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
  const add = (below: number, above: number) => {
    if (!edges.get(below)!.has(above)) {
      edges.get(below)!.add(above);
      incoming.set(above, incoming.get(above)! + 1);
    }
  };
  let active: typeof pieces = [];
  for (const current of pieces) {
    active = active.filter((other) => other.maxX > current.minX);
    for (const other of active) {
      if (current.layer === other.layer || current.minY >= other.maxY || current.maxY <= other.minY)
        continue;
      if (current.plane.every((n, i) => Math.abs(n - other.plane[i]!) < 1e-7)) continue;
      const overlap = fixedPolygonBoolean("intersection", [[current.polygon]], [[other.polygon]]);
      let above = false,
        below = false;
      for (const polygon of overlap)
        for (const point of polygon[0]!) {
          const delta = planeHeight(current.plane, point) - planeHeight(other.plane, point);
          above ||= delta > 1e-4;
          below ||= delta < -1e-4;
        }
      if (above) add(other.layer, current.layer);
      if (below) add(current.layer, other.layer);
    }
    active.push(current);
  }
  const pending = new Set(layers);
  const ordered: number[] = [];
  while (pending.size) {
    const next = layers.find((layer) => pending.has(layer) && incoming.get(layer) === 0);
    if (next === undefined) {
      warnings.push(
        `Navigation layers ${[...pending].join(", ")}: crossing receiving heights prevent consistent click ordering; remaining layers retain their previous order.`,
      );
      ordered.push(...pending);
      break;
    }
    ordered.push(next);
    pending.delete(next);
    for (const above of edges.get(next)!) incoming.set(above, incoming.get(above)! - 1);
  }
  // Preserve the reserved empty ground layer when no ordinary region owns it.
  const offset = layers.includes(0) ? 0 : 1;
  const remap = new Map(ordered.map((layer, index) => [layer, index + offset]));
  for (const region of regions)
    if (!region.lift) {
      region.layer = remap.get(region.layer)!;
      for (const piece of region.pieces) piece.layer = region.layer;
    }
}
