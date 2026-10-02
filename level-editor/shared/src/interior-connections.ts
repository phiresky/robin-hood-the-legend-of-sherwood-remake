import type { GameplayAssetDescriptor } from "./asset-gameplay.ts";
import type { Level3D, Level3DObject } from "./level3d.ts";

export interface InteriorEndpoint {
  placement: string;
  asset: string;
  interior: string;
}

/** A map-owned connection between rooms of independently placed assets. */
export interface InteriorConnection {
  id: string;
  from: InteriorEndpoint;
  to: InteriorEndpoint;
}

export function interiorEndpointId(endpoint: InteriorEndpoint): string {
  return `${endpoint.placement}/${endpoint.asset}/${endpoint.interior}`;
}

export function hasInteriorPlacement(
  objects: Level3DObject[],
  endpoint: InteriorEndpoint,
): boolean {
  return objects.some(
    (part) =>
      (part.group ?? part.id) === endpoint.placement &&
      part.node.startsWith(`asset:${endpoint.asset}:`),
  );
}

export function validateInteriorConnections(value: unknown, objects: Level3DObject[]): void {
  if (!Array.isArray(value)) throw new Error("Interior connections must be an array");
  const ids = new Set<string>();
  const pairs = new Set<string>();
  const endpoint = (value: unknown): InteriorEndpoint => {
    if (
      !value ||
      typeof value !== "object" ||
      !("placement" in value) ||
      typeof value.placement !== "string" ||
      !value.placement ||
      !("asset" in value) ||
      typeof value.asset !== "string" ||
      !value.asset ||
      !("interior" in value) ||
      typeof value.interior !== "string" ||
      !value.interior
    )
      throw new Error("Invalid interior connection endpoint");
    const result = { placement: value.placement, asset: value.asset, interior: value.interior };
    if (!hasInteriorPlacement(objects, result))
      throw new Error(`Interior connection has missing placement: ${interiorEndpointId(result)}`);
    return result;
  };
  for (const item of value) {
    if (
      !item ||
      typeof item !== "object" ||
      typeof item.id !== "string" ||
      !item.id ||
      ids.has(item.id)
    )
      throw new Error("Invalid or duplicate interior connection ID");
    ids.add(item.id);
    const from = endpoint(item.from),
      to = endpoint(item.to);
    if (from.placement === to.placement && from.asset === to.asset)
      throw new Error(
        `Interior connection ${item.id}: rooms within an asset are defined by the asset`,
      );
    const pair = JSON.stringify([interiorEndpointId(from), interiorEndpointId(to)].sort());
    if (pairs.has(pair)) throw new Error(`Duplicate interior connection: ${item.id}`);
    pairs.add(pair);
  }
}

/** Each option is a whole local room; its entrances never need individual map links. */
export function placedInteriorOptions(
  document: Level3D,
  assets: Map<string, GameplayAssetDescriptor>,
) {
  const options = new Map<string, { endpoint: InteriorEndpoint; label: string }>();
  for (const part of document.objects) {
    const match = /^asset:([^:]+):/.exec(part.node);
    const asset = match && assets.get(match[1]!);
    if (!asset) continue;
    const placement = part.group ?? part.id;
    const group = document.groups.find((group) => group.id === part.group);
    for (const room of asset.gameplay?.interiors ?? []) {
      const endpoint = { placement, asset: asset.id, interior: room.id };
      const label = `${group?.name ?? part.name ?? asset.name ?? asset.id} (${placement}) / ${room.id} · ${room.doors.length} doors`;
      options.set(interiorEndpointId(endpoint), { endpoint, label });
    }
  }
  return [...options.values()];
}

export function connectInteriors(
  document: Level3D,
  from: InteriorEndpoint,
  to: InteriorEndpoint,
): Level3D {
  const occupied = new Set(document.interiorConnections?.map((link) => link.id));
  let n = 1;
  while (occupied.has(`interior-link-${n}`)) n++;
  const interiorConnections = [
    ...(document.interiorConnections ?? []),
    {
      id: `interior-link-${n}`,
      from: { ...from },
      to: { ...to },
    },
  ];
  validateInteriorConnections(interiorConnections, document.objects);
  return { ...document, interiorConnections };
}
