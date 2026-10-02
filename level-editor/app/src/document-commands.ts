import {
  parseLevel3D,
  hasInteriorPlacement,
  type AssetState,
  type Level3D,
  type Level3DGroup,
  type Level3DObject,
} from "@rle/shared";

export type Selection = { kind: "group" | "part"; id: string } | null;

/** Commands return immutable revisions; the session, not a command, owns history. */
export function patchPart(document: Level3D, id: string, patch: Partial<Level3DObject>): Level3D {
  if (!document.objects.some((part) => part.id === id)) throw new Error(`Unknown part ${id}`);
  if (patch.hidden !== undefined && stateOwner(document, id))
    throw new Error("Use the group State selector to change endpoint visibility");
  return {
    ...document,
    objects: document.objects.map((part) => (part.id === id ? { ...part, ...patch } : part)),
  };
}

export function patchGroup(document: Level3D, id: string, patch: Partial<Level3DGroup>): Level3D {
  if (!document.groups.some((group) => group.id === id)) throw new Error(`Unknown group ${id}`);
  return {
    ...document,
    groups: document.groups.map((group) => (group.id === id ? { ...group, ...patch } : group)),
  };
}

function uniqueId(base: string, occupied: Set<string>) {
  let n = 1;
  while (occupied.has(`${base}-copy${n}`)) n++;
  const id = `${base}-copy${n}`;
  occupied.add(id);
  return id;
}

export function duplicateSelection(
  document: Level3D,
  selection: NonNullable<Selection>,
): { document: Level3D; selection: NonNullable<Selection> } {
  if (selection.kind === "group") {
    const group = document.groups.find((item) => item.id === selection.id);
    if (!group) throw new Error(`Unknown group ${selection.id}`);
    const id = uniqueId(group.id, new Set(document.groups.map((item) => item.id)));
    const occupied = new Set(document.objects.map((item) => item.id));
    const remap = new Map<string, string>();
    const copies = document.objects
      .filter((part) => part.group === group.id)
      .map((part) => {
        const preferred = `${part.id}-${id}`;
        const copyId = occupied.has(preferred) ? uniqueId(preferred, occupied) : preferred;
        occupied.add(copyId);
        remap.set(part.id, copyId);
        return { ...part, id: copyId, group: id };
      });
    return {
      document: {
        ...document,
        groups: [
          ...document.groups,
          {
            ...group,
            ...(group.states
              ? {
                  states: {
                    active: group.states.active,
                    initial: group.states.initial.map((member) => remap.get(member)!),
                    applied: group.states.applied.map((member) => remap.get(member)!),
                  },
                }
              : {}),
            id,
            transform: {
              ...group.transform,
              dx: group.transform.dx + 40,
              dy: group.transform.dy + 20,
            },
          },
        ],
        objects: [...document.objects, ...copies],
        ...(document.interiorConnections
          ? {
              interiorConnections: [
                ...document.interiorConnections,
                ...document.interiorConnections
                  .filter(
                    (link) => link.from.placement === group.id && link.to.placement === group.id,
                  )
                  .map((link) => ({
                    ...link,
                    id: uniqueId(
                      link.id,
                      new Set(document.interiorConnections!.map((link) => link.id)),
                    ),
                    from: { ...link.from, placement: id },
                    to: { ...link.to, placement: id },
                  })),
              ],
            }
          : {}),
      },
      selection: { kind: "group", id },
    };
  }
  if (stateOwner(document, selection.id)) throw new Error("Duplicate the complete state group");
  const part = document.objects.find((item) => item.id === selection.id);
  if (!part) throw new Error(`Unknown part ${selection.id}`);
  const id = uniqueId(part.id, new Set(document.objects.map((item) => item.id)));
  const copy = {
    ...part,
    id,
    transform: {
      ...part.transform,
      dx: part.transform.dx + 40,
      dy: part.transform.dy + 20,
    },
  };
  return {
    document: { ...document, objects: [...document.objects, copy] },
    selection: { kind: "part", id },
  };
}

export function deleteSelection(document: Level3D, selection: NonNullable<Selection>): Level3D {
  const cleanConnections = (next: Level3D): Level3D =>
    document.interiorConnections
      ? {
          ...next,
          interiorConnections: document.interiorConnections.filter(
            (link) =>
              hasInteriorPlacement(next.objects, link.from) &&
              hasInteriorPlacement(next.objects, link.to),
          ),
        }
      : next;
  if (selection.kind === "group") {
    if (!document.groups.some((group) => group.id === selection.id))
      throw new Error(`Unknown group ${selection.id}`);
    return cleanConnections({
      ...document,
      groups: document.groups.filter((group) => group.id !== selection.id),
      objects: document.objects.filter((part) => part.group !== selection.id),
    });
  }
  if (stateOwner(document, selection.id)) throw new Error("Delete the complete state group");
  if (!document.objects.some((part) => part.id === selection.id))
    throw new Error(`Unknown part ${selection.id}`);
  return cleanConnections({
    ...document,
    objects: document.objects.filter((part) => part.id !== selection.id),
  });
}

export function stateOwner(document: Level3D, objectId: string): Level3DGroup | undefined {
  return document.groups.find(
    (group) =>
      group.states && [...group.states.initial, ...group.states.applied].includes(objectId),
  );
}

/** Both endpoint visibilities and the selected state form one undoable revision. */
export function setGroupState(document: Level3D, id: string, active: AssetState): Level3D {
  parseLevel3D(document);
  const group = document.groups.find((group) => group.id === id);
  if (!group?.states) throw new Error(`Group ${id} has no authored states`);
  if (active !== "initial" && active !== "applied") throw new Error("Invalid state");
  const hidden = new Map<string, boolean>();
  for (const endpoint of ["initial", "applied"] as const)
    for (const member of group.states[endpoint]) hidden.set(member, endpoint !== active);
  const next: Level3D = {
    ...document,
    groups: document.groups.map((item) =>
      item.id === id ? { ...item, states: { ...group.states!, active } } : item,
    ),
    objects: document.objects.map((part) =>
      hidden.has(part.id) ? { ...part, hidden: hidden.get(part.id)! } : part,
    ),
  };
  return parseLevel3D(next);
}
