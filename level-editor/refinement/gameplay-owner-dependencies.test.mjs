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
    ["whole", true, false], ["broken", false, true], ["shared", true, true],
  ]) {
    assert.deepEqual(gameplayOwnerDependencies(descriptor, node), {
      asset: "assembly", movementTransitions: [{ id: "roof", initial, applied }],
    });
  }
  assert.deepEqual(gameplayOwnerDependencies(descriptor, "unrelated"), {
    asset: "assembly", movementTransitions: [],
  });
  assert.deepEqual(gameplayOwnerDependencies({ id: "static" }, "whole"), {
    asset: "static", movementTransitions: [],
  });
});
