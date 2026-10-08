import test from "node:test";
import assert from "node:assert/strict";
import {
  assertPatchMappingEquivalent,
  assetPartAppearanceExtras,
  deriveAppearancePatches,
  endpointPatchRule,
  patchBindingExtras,
  patchBindingsFromMetadata,
  remapPatchExtras,
} from "./patch-bindings.ts";

test("part appearances preserve mesh metadata and reject conflicting mesh rules", () => {
  const model = { source_obstacle: 2, reveal_show_when_applied: ["gate"] };
  const before = structuredClone(model);
  assert.deepEqual(assetPartAppearanceExtras(model, { show: ["gate"], hide: ["destroy"] }), {
    ...model,
    reveal_hide_when_applied: ["destroy"],
  });
  assert.deepEqual(model, before);
  assert.throws(() => assetPartAppearanceExtras(model, { show: ["other"] }), /conflicts/);
});

test("publication metadata yields only active placement patch rules", () => {
  const bindings = patchBindingsFromMetadata({
    Wall: {
      reveal_role: "shared",
      reveal_patch_ids: [],
      sight_patch_before_ids: [],
      drawbridge_endpoint_source_sha256: "evidence",
    },
    Roof: {
      reveal_component_role: "removable-cover",
      reveal_hide_when_applied: ["patch-003"],
      reveal_show_when_applied: [],
    },
    Texture: { reveal_material_patch: "patch-005", reveal_material_state: "revealed" },
  });
  assert.deepEqual(bindings, {
    Roof: { hide: ["patch-003"] },
    Texture: { material: { patch: "patch-005", state: "revealed" } },
  });
  assert.deepEqual(patchBindingExtras(bindings!.Roof!), {
    reveal_hide_when_applied: ["patch-003"],
  });
  assert.deepEqual(patchBindingExtras({ material: { patch: "patch-005", state: "revealed" } }), {
    reveal_material_patch: "patch-005",
    reveal_material_state: "revealed",
  });
});

test("asset appearance mapping reproduces every reviewed node rule", () => {
  const nodes = new Map([
    ["wall", { reveal_hide_when_applied: ["appearance-1"] }],
    ["roof", { reveal_show_when_applied: ["appearance-1"] }],
  ]);
  const bindings = {
    wall: { hide: ["patch-003"] },
    roof: { show: ["patch-003"] },
  };
  const mapping: Record<string, string> = {};
  deriveAppearancePatches(bindings, nodes, mapping);
  assert.deepEqual(mapping, { "appearance-1": "patch-003" });
  assertPatchMappingEquivalent(bindings, nodes, mapping);
  assert.deepEqual(remapPatchExtras(nodes.get("wall")!, mapping), {
    reveal_hide_when_applied: ["patch-003"],
  });
  assert.throws(
    () => assertPatchMappingEquivalent({ wall: bindings.wall }, nodes, mapping),
    /changes roof/,
  );
});

test("static endpoint slot switches only parts unique to each appearance", () => {
  const nodes = new Set([
    "asset:bridge:building-221",
    "asset:bridge:building-388",
    "asset:bridge--state-applied:building-221",
    "asset:bridge--state-applied:building-389",
  ]);
  const patches = { bridge: { state: "patch-001" } };
  assert.equal(endpointPatchRule("asset:bridge:building-221", nodes, patches), undefined);
  assert.deepEqual(endpointPatchRule("asset:bridge:building-388", nodes, patches), {
    hide: ["patch-001"],
  });
  assert.equal(
    endpointPatchRule("asset:bridge--state-applied:building-221", nodes, patches),
    undefined,
  );
  assert.deepEqual(endpointPatchRule("asset:bridge--state-applied:building-389", nodes, patches), {
    show: ["patch-001"],
  });
});
