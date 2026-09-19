import derby from "../assets/derby.json" with { type: "json" };
import { IDENTITY_TRANSFORM, isIdentity, type Level3D, type Level3DGroup, type Level3DObject } from "./level3d.ts";

/** Upgrade only untouched generated groups; saved user edits keep their ownership. */
export function upgradeGeneratedAssetGroups(document: Level3D): boolean {
  if (document.map.toLowerCase() !== "derby" ||
      document.groups.some(group => !/^group-\d+$/.test(group.id) || group.name || group.hidden || !isIdentity(group.transform)) ||
      document.objects.some(object => object.name || object.hidden || !isIdentity(object.transform) || object.id !== object.node)) return false;
  const groups = authoredAssetGroups(document.map, document.objects);
  if (!groups) return false;
  document.groups = groups;
  return true;
}

/** Authored ownership is independent of touching/overlapping collision volumes. */
export function authoredAssetGroups(map: string, objects: Level3DObject[]): Level3DGroup[] | null {
  if (map.toLowerCase() !== derby.map.toLowerCase()) return null;
  const parts = new Map(derby.groups.flatMap(group => group.parts.map(part =>
    [part.obstacle, { group, part }] as const,
  )));
  const ids = new Set(objects.map(object => object.source.obstacle));
  if (parts.size !== ids.size || objects.length !== ids.size || [...ids].some(id => !parts.has(id))) {
    throw new Error("Derby asset catalog does not match this reconstruction's obstacle set");
  }
  for (const object of objects) {
    const { group, part } = parts.get(object.source.obstacle)!;
    object.group = group.id;
    object.name = part.name;
  }
  return derby.groups.map(group => ({
    id: group.id, name: group.name, transform: { ...IDENTITY_TRANSFORM },
  }));
}
