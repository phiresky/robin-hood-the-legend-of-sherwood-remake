/** Authoring evidence only: matching names do not prove equivalent local frames. */
export function gameplayOwnerDependencies(descriptor, node) {
  const gameplay = descriptor.gameplay ?? {};
  const ownsObstacle = (id) =>
    id === node || gameplay.volumes?.some((volume) => volume.id === id && volume.node === node);
  const ownsMask = (id) =>
    gameplay.masks?.some(
      (mask) => mask.id === id && (mask.node === node || mask.obstacles?.some(ownsObstacle)),
    );
  const doors = [
    ...(gameplay.doors ?? []),
    ...(gameplay.interiors ?? []).flatMap((room) =>
      room.doors.map((door) => ({ ...door, roomNode: room.node })),
    ),
  ];
  return {
    asset: descriptor.id,
    movementTransitions: (gameplay.movementTransitions ?? []).flatMap((transition) => {
      const references = [];
      for (const state of ["initial", "applied"]) {
        if (transition[state]?.some((surface) => surface.node === node))
          references.push(`${state}:movement`);
        if (transition[`${state}Sight`]?.some(ownsObstacle)) references.push(`${state}:sight`);
        if (transition[`${state}Masks`]?.some(ownsMask)) references.push(`${state}:mask`);
      }
      if (transition.node === node) references.push("control");
      if (
        transition.doorLinks?.ids.some((id) =>
          doors.some((door) => door.id === id && (door.node === node || door.roomNode === node)),
        )
      )
        references.push("doors");
      if (
        transition.appearances?.some((id) =>
          descriptor.parts?.some(
            (part) =>
              part.node === node &&
              [...(part.appearance?.hide ?? []), ...(part.appearance?.show ?? [])].includes(id),
          ),
        )
      )
        references.push("appearance");
      const initial = references.some((reference) => reference.startsWith("initial:"));
      const applied = references.some((reference) => reference.startsWith("applied:"));
      return references.length ? [{ id: transition.id, initial, applied, references }] : [];
    }),
  };
}
