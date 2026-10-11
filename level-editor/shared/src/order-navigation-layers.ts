import type { NavigationRegion } from "./assemble-navigation-regions.ts";
import { planeHeight } from "./gameplay-plane.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { separateOverlappingNavigationLayers } from "./separate-overlapping-navigation-layers.ts";
import { warnSelfOverlappingNavigation } from "./warn-self-overlapping-navigation.ts";
import {
  separateContradictoryLayerGroups,
  type NavigationLayerContact,
} from "./separate-contradictory-layer-groups.ts";

/** Prepare bottom-to-top click precedence from overlapping receiving geometry. */
export function orderNavigationLayers(regions: NavigationRegion[], warnings: string[]): void {
  separateOverlappingNavigationLayers(regions);
  while (!tryOrderNavigationLayers(regions, warnings)) {
    // Each retry splits a shared layer; individual navigation regions stay intact.
  }
  warnSelfOverlappingNavigation(regions, warnings);
}

function tryOrderNavigationLayers(regions: NavigationRegion[], warnings: string[]): boolean {
  const layers = [
    ...new Set(regions.filter((region) => !region.lift).map((region) => region.layer)),
  ].sort((a, b) => a - b);
  const edges = new Map(layers.map((layer) => [layer, new Set<number>()]));
  const incoming = new Map(layers.map((layer) => [layer, 0]));
  const contacts: (NavigationLayerContact & {
    layers: [number, number];
    point: [number, number];
  })[] = [];
  const conflicts: number[][] = [];
  const pieces = regions
    .filter((region) => !region.lift)
    .flatMap((region) =>
      region.pieces.map((piece) => {
        const polygon = piece.receivingPolygon ?? piece.polygon;
        return {
          region,
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
      if (above || below) {
        const points = overlap.flatMap((polygon) => polygon[0]!);
        contacts.push({
          a: current.region,
          b: other.region,
          above,
          below,
          layers: [current.layer, other.layer],
          point: [
            (Math.min(...points.map((point) => point[0])) +
              Math.max(...points.map((point) => point[0]))) /
              2,
            (Math.min(...points.map((point) => point[1])) +
              Math.max(...points.map((point) => point[1]))) /
              2,
          ],
        });
      }
    }
    active.push(current);
  }
  const pending = new Set(layers);
  const ordered: number[] = [];
  while (pending.size) {
    const next = layers.find((layer) => pending.has(layer) && incoming.get(layer) === 0);
    let group: number[];
    if (next === undefined) {
      // Resolve only a source strongly connected component. Layers above a
      // conflicting group must still follow it, even when their old IDs do not.
      const reachable = new Map<number, Set<number>>();
      for (const layer of pending) {
        const seen = new Set<number>();
        const queue = [layer];
        while (queue.length) {
          const current = queue.pop()!;
          if (seen.has(current)) continue;
          seen.add(current);
          for (const above of edges.get(current)!) if (pending.has(above)) queue.push(above);
        }
        reachable.set(layer, seen);
      }
      group = [...pending]
        .map((layer) =>
          [...pending].filter(
            (other) => reachable.get(layer)!.has(other) && reachable.get(other)!.has(layer),
          ),
        )
        .find((component) =>
          [...pending].every(
            (other) =>
              component.includes(other) ||
              component.every((layer) => !edges.get(other)!.has(layer)),
          ),
        )!;
      if (separateContradictoryLayerGroups(regions, contacts, group)) return false;
      conflicts.push(group);
    } else group = [next];
    ordered.push(...group);
    for (const layer of group) pending.delete(layer);
    for (const layer of group)
      for (const above of edges.get(layer)!) incoming.set(above, incoming.get(above)! - 1);
  }
  // Preserve the reserved empty ground layer when no ordinary region owns it.
  const offset = layers.includes(0) ? 0 : 1;
  const remap = new Map(ordered.map((layer, index) => [layer, index + offset]));
  for (const group of conflicts) {
    const overlaps = contacts.filter((contact) =>
      contact.layers.every((layer) => group.includes(layer)),
    );
    const locations = overlaps
      .slice(0, 4)
      .map(
        (contact) =>
          `layers ${contact.layers.map((layer) => remap.get(layer)!).join("/")} near projected (${contact.point.map((n) => n.toFixed(2)).join(", ")})`,
      );
    warnings.push(
      `Navigation layers ${group.map((layer) => remap.get(layer)!).join(", ")}: crossing receiving heights prevent consistent click ordering; these layers retain their previous relative order. Conflicting overlaps: ${locations.join("; ")}${overlaps.length > 4 ? `; ${overlaps.length - 4} more` : ""}.`,
    );
  }
  for (const region of regions)
    if (!region.lift) {
      region.layer = remap.get(region.layer)!;
      for (const piece of region.pieces) piece.layer = region.layer;
    }
  return true;
}
