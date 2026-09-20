import assert from "node:assert/strict";
import test from "node:test";
import { partMatrix, type Level3D } from "@rle/shared";
import { mergeRefinedGroups, type AuthoredGltf } from "./refined-map-groups.ts";

const identity = () => ({ dx: 0, dy: 0, dz: 0, rot_deg: 0 });
function gltf(groups: Record<string, string[]>): AuthoredGltf {
  const nodes: AuthoredGltf["nodes"] = [{ name: "map", children: [] }];
  for (const [id, parts] of Object.entries(groups)) {
    nodes[0]!.children!.push(nodes.length);
    const group = { name: id, extras: { asset_group: id }, children: [] as number[] };
    nodes.push(group);
    for (const part of parts) {
      group.children.push(nodes.length);
      nodes.push({ name: part, extras: { part_name: `Name ${part}` } });
    }
  }
  return { nodes };
}
function fixture(): Level3D {
  return { version: 1, map: "test", size: [100, 100], glb: "test.glb",
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    groups: ["hall", "stairs", "yard", "custom"].map(id => ({ id, name: id, transform: identity() })),
    objects: ["a", "b", "c", "d", "e"].map((node, i) => ({ id: node, node, kind: "building",
      source: { map: "test", obstacle: i }, name: `Name ${node}`,
      group: ["hall", "stairs", "stairs", "yard", "custom"][i], transform: identity(),
      obstacle: { points: [{ x: i * 10, y: i * 20, z_bottom: 0, z_top: 10 },
        { x: i * 10 + 4, y: i * 20, z_bottom: 0, z_top: 10 },
        { x: i * 10, y: i * 20 + 4, z_bottom: 0, z_top: 10 }],
        projection_area: null, opaque: true, solid: true, mouse: true, show_shadow_polygon: true,
        default_material: 0, material_indices: [] } })) };
}
const previous = () => gltf({ hall: ["a"], stairs: ["b", "c"], yard: ["d", "e"] });
const refined = () => gltf({ hall: ["a", "b", "c"], trough: ["d", "e"] });

test("migrates authored ownership, creates groups, and retains custom membership and labels", () => {
  const document = fixture();
  document.groups[0]!.name = "My hall";
  document.objects[1]!.name = "My steps";
  const result = mergeRefinedGroups(document, previous(), refined());
  assert.deepEqual(result.migratedParts, ["b", "c", "d"]);
  assert.deepEqual(result.createdGroups, ["trough"]);
  assert.deepEqual(result.removedGroups, ["stairs", "yard"]);
  assert.equal(document.objects[4]!.group, "custom");
  assert.equal(document.objects[1]!.name, "My steps");
  assert.equal(document.groups[0]!.name, "My hall");
  assert.equal(document.objects[3]!.group, "trough");
  assert.deepEqual(mergeRefinedGroups(document, refined(), refined()).migratedParts, []);
});

test("reparenting keeps all world transforms when both groups rotate and pivots change", () => {
  const document = fixture();
  document.groups[0]!.transform = { dx: 19, dy: -3, dz: 12, rot_deg: 43 };
  document.groups[1]!.transform = { dx: -13, dy: 31, dz: 5, rot_deg: -28 };
  document.objects[2]!.transform = { dx: 11, dy: 7, dz: 2, rot_deg: 14 };
  const matrices = document.objects.map(part => partMatrix(document.camera, document, part));
  mergeRefinedGroups(document, previous(), refined());
  document.objects.forEach((part, i) => partMatrix(document.camera, document, part).forEach((value, j) =>
    assert.ok(Math.abs(value - matrices[i]![j]!) < 1e-8, `${part.id} matrix component ${j}`)));
  assert.ok(document.groups.some(group => group.id === "stairs"), "edited empty groups are retained");
});

test("new groups accept multiple parts, while hidden or colliding custom groups are preserved", () => {
  const document = fixture();
  document.objects[4]!.group = "yard";
  assert.deepEqual(mergeRefinedGroups(document, previous(), refined()).migratedParts, ["b", "c", "d", "e"]);
  const hidden = fixture();
  hidden.groups[0]!.hidden = true;
  hidden.groups.push({ id: "trough", name: "Custom trough", transform: identity() });
  assert.deepEqual(mergeRefinedGroups(hidden, previous(), refined()).migratedParts, []);
});
