import test from "node:test";
import assert from "node:assert/strict";
import { gameplayOwnerDependencies } from "./gameplay-owner-dependencies.mjs";

test("ownership audit distinguishes initial, applied, shared and unrelated state parts", () => {
  const descriptor = {
    id: "assembly",
    gameplay: {
      movementTransitions: [
        { id: "roof", initialSight: ["whole", "shared"], appliedSight: ["broken", "shared"] },
        { id: "door", initialSight: ["closed"], appliedSight: [] },
      ],
    },
  };
  for (const [node, initial, applied] of [
    ["whole", true, false],
    ["broken", false, true],
    ["shared", true, true],
  ]) {
    assert.deepEqual(gameplayOwnerDependencies(descriptor, node), {
      asset: "assembly",
      movementTransitions: [
        {
          id: "roof",
          initial,
          applied,
          references: [
            ...(initial ? ["initial:sight"] : []),
            ...(applied ? ["applied:sight"] : []),
          ],
        },
      ],
    });
  }
  assert.deepEqual(gameplayOwnerDependencies(descriptor, "unrelated"), {
    asset: "assembly",
    movementTransitions: [],
  });
  assert.deepEqual(gameplayOwnerDependencies({ id: "static" }, "whole"), {
    asset: "static",
    movementTransitions: [],
  });
});

test("ownership audit follows movement, volume, mask, room, control and appearance references", () => {
  const descriptor = {
    id: "assembly",
    parts: [{ node: "visual", appearance: { hide: ["change"] } }],
    gameplay: {
      volumes: [{ id: "volume", node: "solid" }],
      masks: [{ id: "mask", node: "mask-frame", obstacles: ["volume"] }],
      interiors: [{ node: "room", doors: [{ id: "door", node: "door-frame" }] }],
      movementTransitions: [
        {
          id: "change",
          node: "control-frame",
          initial: [{ node: "floor" }],
          applied: [],
          initialSight: ["volume"],
          appliedMasks: ["mask"],
          appearances: ["change"],
          doorLinks: { mode: "swap-rights", ids: ["door"] },
        },
      ],
    },
  };
  for (const [node, references] of [
    ["floor", ["initial:movement"]],
    ["solid", ["initial:sight", "applied:mask"]],
    ["mask-frame", ["applied:mask"]],
    ["room", ["doors"]],
    ["door-frame", ["doors"]],
    ["control-frame", ["control"]],
    ["visual", ["appearance"]],
  ]) {
    const dependencies = gameplayOwnerDependencies(descriptor, node).movementTransitions;
    assert.equal(dependencies.length, 1, node);
    assert.deepEqual(dependencies[0].references, references, node);
  }
  assert.deepEqual(gameplayOwnerDependencies(descriptor, "unrelated").movementTransitions, []);
});
