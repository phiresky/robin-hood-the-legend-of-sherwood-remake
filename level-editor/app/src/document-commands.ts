import type { Level3D, Level3DGroup, Level3DObject } from "@rle/shared";

export type Selection = { kind: "group" | "part"; id: string } | null;

/** Commands return immutable revisions; the session, not a command, owns history. */
export function patchPart(
  document: Level3D,
  id: string,
  patch: Partial<Level3DObject>,
): Level3D {
  if (!document.objects.some((part) => part.id === id))
    throw new Error(`Unknown part ${id}`);
  return {
    ...document,
    objects: document.objects.map((part) =>
      part.id === id ? { ...part, ...patch } : part,
    ),
  };
}

export function patchGroup(
  document: Level3D,
  id: string,
  patch: Partial<Level3DGroup>,
): Level3D {
  if (!document.groups.some((group) => group.id === id))
    throw new Error(`Unknown group ${id}`);
  return {
    ...document,
    groups: document.groups.map((group) =>
      group.id === id ? { ...group, ...patch } : group,
    ),
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
    const id = uniqueId(
      group.id,
      new Set(document.groups.map((item) => item.id)),
    );
    const occupied = new Set(document.objects.map((item) => item.id));
    const copies = document.objects
      .filter((part) => part.group === group.id)
      .map((part) => {
        const preferred = `${part.id}-${id}`;
        const copyId = occupied.has(preferred)
          ? uniqueId(preferred, occupied)
          : preferred;
        occupied.add(copyId);
        return { ...part, id: copyId, group: id };
      });
    return {
      document: {
        ...document,
        groups: [
          ...document.groups,
          {
            ...group,
            id,
            transform: {
              ...group.transform,
              dx: group.transform.dx + 40,
              dy: group.transform.dy + 20,
            },
          },
        ],
        objects: [...document.objects, ...copies],
      },
      selection: { kind: "group", id },
    };
  }
  const part = document.objects.find((item) => item.id === selection.id);
  if (!part) throw new Error(`Unknown part ${selection.id}`);
  const id = uniqueId(
    part.id,
    new Set(document.objects.map((item) => item.id)),
  );
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

export function deleteSelection(
  document: Level3D,
  selection: NonNullable<Selection>,
): Level3D {
  if (selection.kind === "group") {
    if (!document.groups.some((group) => group.id === selection.id))
      throw new Error(`Unknown group ${selection.id}`);
    return {
      ...document,
      groups: document.groups.filter((group) => group.id !== selection.id),
      objects: document.objects.filter((part) => part.group !== selection.id),
    };
  }
  if (!document.objects.some((part) => part.id === selection.id))
    throw new Error(`Unknown part ${selection.id}`);
  return {
    ...document,
    objects: document.objects.filter((part) => part.id !== selection.id),
  };
}
