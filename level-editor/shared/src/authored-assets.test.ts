import test from "node:test";
import assert from "node:assert/strict";
import derby from "../assets/derby.json" with { type: "json" };
import { authoredAssetGroups, upgradeGeneratedAssetGroups } from "./authored-assets.ts";
import { IDENTITY_TRANSFORM, type Level3D, type Level3DObject } from "./level3d.ts";

function objects(): Level3DObject[] {
  return Array.from({ length: 271 }, (_, obstacle) => obstacle)
    .filter(obstacle => obstacle !== 35)
    .map(obstacle => ({
      id: `building-${String(obstacle).padStart(3, "0")}`,
      node: `building-${String(obstacle).padStart(3, "0")}`,
      kind: "building", source: { map: "Derby", obstacle },
      obstacle: { points: [], opaque: true, solid: true, mouse: false,
        show_shadow_polygon: false, default_material: 0, material_indices: [], projection_area: {} },
      transform: { ...IDENTITY_TRANSFORM },
    }));
}

test("Derby assigns every exported obstacle to exactly one named logical asset", () => {
  const parts = objects();
  const before = structuredClone(parts);
  const groups = authoredAssetGroups("Derby", parts)!;
  const members = derby.groups.flatMap(group => group.parts.map(part => part.obstacle));
  assert.equal(members.length, 270);
  assert.equal(new Set(members).size, 270);
  assert.equal(groups.length, 30);
  assert.equal(new Set(groups.map(group => group.id)).size, groups.length);
  assert.ok(groups.every(group => group.name && !group.name.startsWith("group-")));
  for (const [i, part] of parts.entries()) {
    assert.ok(groups.some(group => group.id === part.group));
    assert.ok(part.name);
    const { name, group, ...unchanged } = part;
    assert.deepEqual(unchanged, before[i]);
  }
  const owner = (id: number) => parts.find(part => part.source.obstacle === id)!.group;
  assert.equal(owner(49), owner(50)); // Both sides of the east cottage.
  assert.equal(owner(130), owner(173)); // Keep walls, towers, and roofs.
  assert.equal(owner(183), owner(210)); // East hall and its roof turret.
  assert.equal(owner(7), owner(12)); // Curtain wall and access stair.
  assert.notEqual(owner(183), owner(214)); // Hall and freestanding watchtower.
  assert.notEqual(owner(49), owner(55)); // Neighboring houses stay independent.
});

test("catalog mismatch fails before mutating a document; other maps retain inferred grouping", () => {
  const parts = objects().slice(1);
  const before = structuredClone(parts);
  assert.throws(() => authoredAssetGroups("Derby", parts), /obstacle set/);
  assert.deepEqual(parts, before);
  assert.equal(authoredAssetGroups("York", parts), null);
  assert.deepEqual(parts, before);
});

test("untouched saved groups upgrade once, but saved transforms and custom ownership survive", () => {
  const document: Level3D = { version: 1, map: "Derby", glb: "derby-volumes.scene.glb",
    size: [1920, 2752], camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    objects: objects(), groups: [{ id: "group-000", transform: { ...IDENTITY_TRANSFORM } }] };
  for (const part of document.objects) part.group = "group-000";
  const edited = structuredClone(document);
  edited.groups[0]!.transform.dx = 10;
  const snapshot = structuredClone(edited);
  assert.equal(upgradeGeneratedAssetGroups(edited), false);
  assert.deepEqual(edited, snapshot);
  assert.equal(upgradeGeneratedAssetGroups(document), true);
  assert.equal(document.groups.length, 30);
  assert.equal(upgradeGeneratedAssetGroups(document), false);
});
