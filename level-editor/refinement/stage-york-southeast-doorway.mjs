// Declare the house wall crossed by its own virtual-room doorway animation.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const asset = "york-southeast-lane-eastern-timber-house";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const source = scene.assetSources.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(hash(bytes), source.descriptor_sha256);
assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const room = gameplay.interiors.find((entry) => entry.id === "interior-0");
const door = room.doors.find((entry) => entry.id === "interior-0/door-0");
assert.equal(room.node, "building-382");
assert.equal(door.node, room.node);
assert.equal(door.passageStructure, undefined);
assert.ok(descriptor.parts.find((part) => part.node === room.node).obstacle_local_game.solid);
door.passageStructure = { volumes: [room.node], blockers: [] };
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-southeast-doorway-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      gameplay,
    },
  ]),
);
console.log(output);
