import {
  assetNodeKey,
  assetVariantId,
  assetPartOrigin,
  IDENTITY_TRANSFORM,
  parseExternalAssetSources,
  parseLevel3D,
  parseProjectionAssetDescriptor,
  type ExternalAssetSource,
  type GameplayAssetDescriptor,
  type Level3D,
  type Level3DObject,
  type ProjectionAssetDescriptor,
} from "@rle/shared";

export function hasGameplayEndpoints(descriptor: GameplayAssetDescriptor) {
  return (
    !!descriptor.state_variants &&
    !!descriptor.gameplay?.movementTransitions?.some((transition) =>
      transition.appearances?.includes("state"),
    )
  );
}

export interface PlacementAsset {
  descriptor: ProjectionAssetDescriptor;
  reference: ExternalAssetSource;
}

/** A new instance shares immutable model resources but owns its document parts. */
export function insertProjectionAsset(
  document: Level3D,
  descriptor: ProjectionAssetDescriptor,
  reference: ExternalAssetSource,
  placement: [number, number, number],
  additionalAssets: readonly PlacementAsset[] = [],
) {
  parseProjectionAssetDescriptor(descriptor);
  if (descriptor.editor_usage === "map-background")
    throw new Error("Map backgrounds are part of the map and cannot be inserted as objects");
  parseExternalAssetSources([reference]);
  if (descriptor.id !== reference.id) throw new Error("Asset identity mismatch");
  const endpoints = hasGameplayEndpoints(descriptor);
  if (endpoints) {
    const applied = additionalAssets[0];
    if (
      reference.state_variant ||
      additionalAssets.length !== 1 ||
      !applied ||
      applied.reference.id !== assetVariantId(descriptor.id, "applied") ||
      applied.reference.state_variant !== "applied" ||
      applied.reference.descriptor !== reference.descriptor ||
      applied.reference.descriptor_sha256 !== reference.descriptor_sha256
    )
      throw new Error("Gameplay endpoint insertion requires its pinned applied model");
  } else if (additionalAssets.length) throw new Error("Unexpected additional placement models");
  const family = [{ descriptor, reference }, ...additionalAssets];
  for (const member of family) {
    parseProjectionAssetDescriptor(member.descriptor);
    parseExternalAssetSources([member.reference]);
    if (member.descriptor.id !== member.reference.id) throw new Error("Asset identity mismatch");
    const previous = document.assetSources?.find((source) => source.id === member.reference.id);
    if (
      previous &&
      (
        [
          "descriptor",
          "model",
          "model_scene",
          "state_variant",
          "descriptor_sha256",
          "model_sha256",
        ] as const
      ).some((key) => previous[key] !== member.reference[key])
    )
      throw new Error("A different revision of this asset is already in the document");
  }
  if (placement.length !== 3 || placement.some((value) => !Number.isFinite(value)))
    throw new Error("Invalid asset placement");
  const primaryNodes = new Set(descriptor.parts.map((part) => part.node));
  const members = family.flatMap((member, index) =>
    member.descriptor.parts
      .filter((part) => index === 0 || !primaryNodes.has(part.node))
      .map((part) => ({ descriptor: member.descriptor, part })),
  );
  const occupied = new Set([
    ...document.groups.map((group) => group.id),
    ...document.objects.map((part) => part.id),
  ]);
  let number = 1;
  let id: string;
  do {
    id = `${descriptor.id}-instance${number++}`;
  } while (occupied.has(id) || members.some(({ part }) => occupied.has(`${id}:${part.node}`)));
  const parts: Level3DObject[] = members.map(({ descriptor: owner, part }) => ({
    id: `${id}:${part.node}`,
    node: assetNodeKey(owner.id, part.node),
    ...assetPartOrigin(owner, part),
    ...(part.obstacle_local_game ? { obstacle: structuredClone(part.obstacle_local_game) } : {}),
    transform: { ...IDENTITY_TRANSFORM },
    group: id,
    name: part.name,
    ...(part.default_hidden ? { hidden: true } : {}),
  }));
  // A reviewed ground height allows foundations below an asset's entrance.
  // Otherwise retain the lowest authored surface/collision placement rule.
  const gameplay = (descriptor as GameplayAssetDescriptor).gameplay;
  if (
    gameplay?.placementGroundHeight !== undefined &&
    !Number.isFinite(gameplay.placementGroundHeight)
  )
    throw new Error(`Asset ${descriptor.id}: invalid placement ground height`);
  const surfaces = gameplay?.surfaces ?? [];
  const heights = [
    ...surfaces.flatMap((s) => (typeof s.height === "number" ? [s.height] : s.height)),
    ...descriptor.parts.flatMap((p) => p.obstacle_local_game?.points.map((v) => v.z_bottom) ?? []),
  ];
  const baseHeight = gameplay?.placementGroundHeight ?? (heights.length ? Math.min(...heights) : 0);
  const next: Level3D = {
    ...document,
    assetSources: [
      ...(document.assetSources ?? []),
      ...family
        .filter(
          (member) => !document.assetSources?.some((source) => source.id === member.reference.id),
        )
        .map((member) => ({ ...member.reference })),
    ],
    groups: [
      ...document.groups,
      {
        id,
        name: descriptor.name,
        ...(endpoints ? { patches: { [descriptor.id]: { state: `${id}/state` } } } : {}),
        ...(descriptor.states && !endpoints
          ? {
              states: {
                active: descriptor.states.active,
                initial: descriptor.states.initial.map((node) => `${id}:${node}`),
                applied: descriptor.states.applied.map((node) => `${id}:${node}`),
              },
            }
          : {}),
        transform: {
          dx: placement[0],
          dy: placement[1],
          dz: placement[2] - baseHeight,
          rot_deg: 0,
        },
      },
    ],
    objects: [...document.objects, ...parts],
  };
  parseLevel3D(next);
  return { document: next, selection: { kind: "group" as const, id } };
}
