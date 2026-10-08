/** Authoring evidence only: matching names do not prove equivalent local frames. */
export function gameplayOwnerDependencies(descriptor, node) {
  return {
    asset: descriptor.id,
    movementTransitions: (descriptor.gameplay?.movementTransitions ?? []).flatMap((transition) => {
      const initial = transition.initialSight?.includes(node) ?? false;
      const applied = transition.appliedSight?.includes(node) ?? false;
      return initial || applied
        ? [{ id: transition.id, initial, applied }]
        : [];
    }),
  };
}
