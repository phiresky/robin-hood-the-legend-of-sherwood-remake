import test from "node:test";
import assert from "node:assert/strict";
import { parseLevel3D } from "@rle/shared";
import {
  deleteSelection,
  duplicateSelection,
  patchGroup,
  patchPart,
} from "./document-commands.ts";

function fixture() {
  return parseLevel3D({
    version: 1,
    map: "York",
    glb: "york.glb",
    size: [100, 200],
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: [
      { id: "house", transform: { dx: 10, dy: 20, dz: 0, rot_deg: 15 } },
    ],
    objects: [
      {
        id: "part",
        group: "house",
        node: "building-000",
        kind: "building",
        source: { map: "York", obstacle: 0 },
        transform: { dx: 1, dy: 2, dz: 0, rot_deg: 0 },
        obstacle: {
          points: [
            { x: 1, y: 2, z_bottom: 0, z_top: 4 },
            { x: 8, y: 2, z_bottom: 0, z_top: 4 },
            { x: 3, y: 9, z_bottom: 0, z_top: 4 },
          ],
          opaque: true,
          solid: true,
          mouse: false,
          show_shadow_polygon: false,
          default_material: 0,
          material_indices: [],
          projection_area: {},
        },
      },
    ],
  });
}

test("group duplicate preserves source/local transforms and allocates collision-free identities", () => {
  const original = fixture();
  const before = structuredClone(original);
  original.objects.push({
    ...original.objects[0]!,
    id: "part-house-copy1",
    group: undefined,
  });
  const result = duplicateSelection(original, { kind: "group", id: "house" });
  assert.equal(result.selection.id, "house-copy1");
  assert.equal(result.document.objects[2]!.id, "part-house-copy1-copy1");
  assert.equal(
    result.document.objects[2]!.transform,
    original.objects[0]!.transform,
  );
  assert.equal(
    result.document.objects[2]!.obstacle,
    original.objects[0]!.obstacle,
  );
  assert.deepEqual(result.document.groups[1]!.transform, {
    dx: 50,
    dy: 40,
    dz: 0,
    rot_deg: 15,
  });
  assert.deepEqual(original.groups, before.groups);
  assert.equal(original.objects.length, 2);
  parseLevel3D(result.document);
});

test("part duplicate offsets locally and preserves membership; delete group removes only members", () => {
  const original = fixture();
  const first = duplicateSelection(original, { kind: "part", id: "part" });
  const second = duplicateSelection(first.document, {
    kind: "part",
    id: "part",
  });
  assert.equal(second.selection.id, "part-copy2");
  assert.equal(second.document.objects[2]!.group, "house");
  assert.deepEqual(second.document.objects[2]!.transform, {
    dx: 41,
    dy: 22,
    dz: 0,
    rot_deg: 0,
  });
  const deleted = deleteSelection(second.document, {
    kind: "part",
    id: "part-copy1",
  });
  assert.equal(deleted.objects.length, 2);
  assert.equal(deleted.groups.length, 1);
  const removed = deleteSelection(deleted, { kind: "group", id: "house" });
  assert.deepEqual(removed.objects, []);
  assert.deepEqual(removed.groups, []);
  assert.equal(original.objects.length, 1);
});

test("transform and visibility edits leave earlier revisions unchanged and reject stale targets", () => {
  const original = fixture();
  const transform = { dx: 9, dy: 8, dz: 7, rot_deg: 90 };
  const next = patchGroup(patchPart(original, "part", { transform }), "house", {
    hidden: true,
  });
  assert.equal(next.objects[0]!.transform, transform);
  assert.equal(next.groups[0]!.hidden, true);
  assert.equal(original.groups[0]!.hidden, undefined);
  assert.equal(original.objects[0]!.transform.dx, 1);
  assert.throws(() => patchPart(original, "missing", {}), /Unknown part/);
  assert.throws(() => patchGroup(original, "missing", {}), /Unknown group/);
  assert.throws(
    () => duplicateSelection(original, { kind: "part", id: "missing" }),
    /Unknown part/,
  );
  assert.throws(
    () => deleteSelection(original, { kind: "group", id: "missing" }),
    /Unknown group/,
  );
});
