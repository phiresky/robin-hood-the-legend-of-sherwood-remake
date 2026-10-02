import {
  gameTransformMatrix,
  groupCentroid,
  groupParts,
  partMatrix,
  partPivot,
  type GameTransform,
  type Level3D,
  type Level3DObject,
} from "./level3d.ts";
import { assetVariantId, type ProjectionAssetDescriptor } from "./projection-assets.ts";
import { assetPartOrigin } from "./asset-instance-document.ts";

function rebaseTransform(
  document: Level3D,
  transform: GameTransform,
  before: [number, number],
  after: [number, number],
): GameTransform {
  const a = gameTransformMatrix(document.camera, transform, before);
  const b = gameTransformMatrix(document.camera, transform, after);
  const sin = Math.sin((document.camera.elevation_deg * Math.PI) / 180);
  return {
    ...transform,
    dx: transform.dx + (a[12]! - b[12]!),
    dy: transform.dy - (a[13]! - b[13]!) * sin,
  };
}

/** Resolve state views only through their matching pinned primary descriptor. */
export function gameplayStateAliases(
  document: Level3D,
  input: ReadonlyMap<string, ProjectionAssetDescriptor>,
) {
  const aliases = new Map<string, string>();
  for (const reference of document.assetSources ?? []) {
    const descriptor = input.get(reference.id);
    if (!reference.state_variant || !descriptor?.state_variants) continue;
    const primary = document.assetSources!.find(
      (source) =>
        !source.state_variant &&
        source.descriptor === reference.descriptor &&
        source.descriptor_sha256 === reference.descriptor_sha256,
    );
    if (!primary || assetVariantId(primary.id, reference.state_variant) !== reference.id)
      throw new Error(`State view ${reference.id} needs its pinned primary asset`);
    aliases.set(reference.id, primary.id);
  }
  return aliases;
}

/** Visual state views of one pinned asset share gameplay identity within each placement. */
export function normalizeGameplayStateViews(
  document: Level3D,
  input: ReadonlyMap<string, ProjectionAssetDescriptor>,
) {
  const descriptors = new Map(input);
  const aliases = gameplayStateAliases(document, input);
  for (const id of aliases.keys()) descriptors.delete(id);
  for (const [id, descriptor] of descriptors) {
    if (!descriptor.state_variants) continue;
    const parts = new Map(descriptor.parts.map((part) => [part.node, part]));
    for (const variant of Object.values(descriptor.state_variants))
      for (const part of variant.parts ?? descriptor.parts) {
        // Profile-backed placeholders carry editor picking bounds, not map collision.
        // TODO: Compile their visual states separately from gameplay obstacle frames.
        if (part.mission_profile !== undefined) continue;
        const previous = parts.get(part.node);
        if (
          previous &&
          JSON.stringify(previous.obstacle_local_game) !== JSON.stringify(part.obstacle_local_game)
        )
          throw new Error(
            `${id}: changing collision needs distinct local state part IDs (${part.node})`,
          );
        if (!previous) parts.set(part.node, part);
      }
    descriptors.set(id, { ...descriptor, parts: [...parts.values()] });
  }
  const objects: Level3DObject[] = [];
  const seen = new Map<string, Level3DObject>();
  const views = new Map<string, Set<string>>();
  for (const original of document.objects) {
    const match = /^asset:([^:]+):(.+)$/.exec(original.node);
    const canonical = match && aliases.get(match[1]!);
    const part = canonical ? { ...original, node: `asset:${canonical}:${match[2]}` } : original;
    const key = `${part.group ?? part.id}/${part.node}`;
    const previous = seen.get(key);
    const sourceId = match?.[1] ?? original.node;
    if (previous && (canonical || (match && descriptors.get(match[1]!)?.state_variants))) {
      if (views.get(key)!.has(sourceId)) throw new Error(`Duplicate state part ${original.node}`);
      views.get(key)!.add(sourceId);
      const a = partMatrix(document.camera, document, previous),
        b = partMatrix(document.camera, document, part);
      if (a.some((value, i) => Math.abs(value - b[i]!) > 1e-9))
        throw new Error(`State views disagree on shared gameplay frame ${part.node}`);
      if (!part.hidden && previous.hidden) {
        const visible = { ...previous, hidden: false };
        objects[objects.indexOf(previous)] = visible;
        seen.set(key, visible);
      }
      continue;
    }
    objects.push(part);
    seen.set(key, part);
    views.set(key, new Set([sourceId]));
  }
  // A newly inserted initial view still needs non-rendering frames for its other states.
  const placements = new Map<string, { asset: string; parts: Level3DObject[] }>();
  for (const part of objects) {
    const match = /^asset:([^:]+):(.+)$/.exec(part.node);
    if (match && part.group && descriptors.get(match[1]!)?.state_variants) {
      const key = `${part.group}/${match[1]}`;
      const placement = placements.get(key);
      if (placement) placement.parts.push(part);
      else placements.set(key, { asset: match[1]!, parts: [part] });
    }
  }
  for (const { asset, parts } of placements.values()) {
    const placed = parts[0]!;
    const descriptor = descriptors.get(asset)!;
    const frame = partMatrix(document.camera, document, placed);
    for (const definition of descriptor.parts) {
      if (definition.mission_profile !== undefined) continue;
      const node = `asset:${asset}:${definition.node}`;
      if (seen.has(`${placed.group}/${node}`)) continue;
      for (const part of parts) {
        const other = partMatrix(document.camera, document, part);
        if (frame.some((value, i) => Math.abs(value - other[i]!) > 1e-9))
          throw new Error(
            `Cannot infer missing state frame for independently edited parts: ${node}`,
          );
      }
      const hidden: Level3DObject = {
        id: `${placed.group}/${node}/gameplay-frame`,
        node,
        group: placed.group,
        ...assetPartOrigin(descriptor, definition),
        transform: { ...placed.transform },
        hidden: true,
        obstacle: definition.obstacle_local_game,
      };
      hidden.transform = rebaseTransform(
        document,
        placed.transform,
        partPivot(placed),
        partPivot(hidden),
      );
      objects.push(hidden);
    }
  }
  // Adding or deduplicating frames must not change an editor group's rotation pivot.
  const normalized = { ...document, objects };
  if (document.interiorConnections)
    normalized.interiorConnections = document.interiorConnections.map((link) => ({
      ...link,
      from: { ...link.from, asset: aliases.get(link.from.asset) ?? link.from.asset },
      to: { ...link.to, asset: aliases.get(link.to.asset) ?? link.to.asset },
    }));
  normalized.groups = document.groups.map((group) => ({
    ...group,
    transform: rebaseTransform(
      document,
      group.transform,
      groupCentroid(groupParts(document, group.id)),
      groupCentroid(groupParts(normalized, group.id)),
    ),
  }));
  return { document: normalized, descriptors };
}
