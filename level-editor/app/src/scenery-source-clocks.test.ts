import test from "node:test";
import assert from "node:assert/strict";
import { ScenerySourceClocks } from "./scenery-source-clocks.ts";
const binding = {
  effectId: "part/animation",
  descriptorSha256: "a".repeat(64),
  sourceIndex: 7,
  sourceSha256: "b".repeat(64),
};
const row = {
  id: "ambient7",
  source: { kind: "map-animation" as const, index: 7, sha256: binding.sourceSha256 },
  epoch: 1,
  key: "identity",
  generation: 1,
  tick: 11,
  frame: 5,
  active: true,
  playing: true,
};
test("explicit scenery correspondence samples current source phase across delayed resource attachment", () => {
  const clocks = new ScenerySourceClocks();
  clocks.bind([binding]);
  clocks.sample([row]);
  assert.equal(clocks.frame(binding.effectId, binding.descriptorSha256, 32), 5);
  clocks.sample([{ ...row, tick: 19, frame: 9 }]);
  assert.equal(clocks.frame(binding.effectId, binding.descriptorSha256, 32), 9);
  assert.equal(clocks.frame("unbound", undefined, 32), undefined);
  clocks.sample([{ ...row, active: false }]);
  assert.equal(clocks.frame(binding.effectId, binding.descriptorSha256, 32), -1);
});
test("changed descriptor/source identities cannot silently fall back to a local load clock", () => {
  const clocks = new ScenerySourceClocks();
  clocks.bind([binding]);
  clocks.sample([row]);
  assert.throws(() => clocks.frame(binding.effectId, "c".repeat(64), 32), /descriptor changed/);
  clocks.sample([{ ...row, source: { ...row.source, sha256: "d".repeat(64) } }]);
  assert.throws(() => clocks.frame(binding.effectId, binding.descriptorSha256, 32), /Missing/);
  clocks.sample([row]);
  assert.throws(() => clocks.frame(binding.effectId, binding.descriptorSha256, 2), /frames differ/);
  clocks.clear();
  assert.equal(clocks.frame(binding.effectId, binding.descriptorSha256, 32), undefined);
});
