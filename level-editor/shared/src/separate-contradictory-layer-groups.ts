import type { NavigationRegion } from "./assemble-navigation-regions.ts";

export interface NavigationLayerContact {
  a: NavigationRegion;
  b: NavigationRegion;
  above: boolean;
  below: boolean;
}

/** Split shared layer IDs only when their individual regions have consistent height order. */
export function separateContradictoryLayerGroups(
  regions: NavigationRegion[],
  contacts: NavigationLayerContact[],
  group: number[],
): boolean {
  const relevant = contacts.filter(
    (contact) => group.includes(contact.a.layer) && group.includes(contact.b.layer),
  );
  const members = new Set(relevant.flatMap((contact) => [contact.a, contact.b]));
  const edges = new Map([...members].map((region) => [region, new Set<NavigationRegion>()]));
  const incoming = new Map([...members].map((region) => [region, 0]));
  const add = (below: NavigationRegion, above: NavigationRegion) => {
    if (edges.get(below)!.has(above)) return;
    edges.get(below)!.add(above);
    incoming.set(above, incoming.get(above)! + 1);
  };
  for (const contact of relevant) {
    if (contact.above) add(contact.b, contact.a);
    if (contact.below) add(contact.a, contact.b);
  }
  const ready = [...members].filter((region) => incoming.get(region) === 0);
  let visited = 0;
  while (ready.length) {
    const region = ready.pop()!;
    visited++;
    for (const above of edges.get(region)!) {
      incoming.set(above, incoming.get(above)! - 1);
      if (incoming.get(above) === 0) ready.push(above);
    }
  }
  // A real cycle between regions cannot be repaired by giving them new IDs.
  if (visited !== members.size) return false;
  let nextLayer =
    Math.max(0, ...regions.filter((region) => !region.lift).map((region) => region.layer)) + 1;
  let split = false;
  for (const layer of group) {
    const shared = regions.filter((region) => members.has(region) && region.layer === layer);
    for (const region of shared.slice(1)) {
      region.layer = nextLayer++;
      for (const piece of region.pieces) piece.layer = region.layer;
      split = true;
    }
  }
  return split;
}
