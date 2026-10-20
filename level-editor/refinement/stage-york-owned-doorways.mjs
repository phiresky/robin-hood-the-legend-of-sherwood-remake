// Explicitly reviewed doorway walls; foreign obstacles remain independent.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const reviewed = [
  ["york-castle-north-round-watchtower", "building-837", [0, 1, 2]],
  ["york-castle-northeast-square-watchtower", "building-805", [0]],
  ["york-precinct-north-connecting-gateway", "building-919", [0]],
  ["york-precinct-west-great-hall", "building-918", [0, 2]],
];
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const edits = [];
for (const [asset, node, indices] of reviewed) {
  const source = scene.assetSources.find((entry) => entry.id === asset);
  const bytes = await fs.readFile(`library/${source.descriptor}`);
  assert.equal(hash(bytes), source.descriptor_sha256);
  assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
  const descriptor = JSON.parse(bytes);
  const gameplay = structuredClone(descriptor.gameplay);
  const room = gameplay.interiors.find((entry) => entry.id === "interior-0");
  assert.equal(room.node, node);
  assert.ok(descriptor.parts.find((part) => part.node === node).obstacle_local_game.solid);
  for (const index of indices) {
    const door = room.doors.find((entry) => entry.id === `interior-0/door-${index}`);
    assert.equal(door.node, node);
    assert.equal(door.passageStructure, undefined);
    door.passageStructure = { volumes: [node], blockers: [] };
  }
  validateAssetGameplay(gameplay, descriptor);
  edits.push({
    asset,
    descriptorSha256: source.descriptor_sha256,
    modelSha256: source.model_sha256,
    gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-owned-doorways-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
