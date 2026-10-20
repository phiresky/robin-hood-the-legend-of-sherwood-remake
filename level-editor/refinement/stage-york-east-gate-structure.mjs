// The tower's ordinary doorway crosses both its main body and perimeter wall.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const asset = "york-south-city-east-gate-tower";
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
assert.equal(room.node, "building-530");
assert.equal(door.node, room.node);
assert.equal(door.type, 1);
assert.equal(door.passageStructure, undefined);
const volumes = ["building-530", "building-531"];
for (const node of volumes) {
  const shape = descriptor.parts.find((part) => part.node === node).obstacle_local_game;
  assert.ok(shape.solid);
  assert.ok(
    shape.points.every((point) => point.z_bottom < door.middle[2] && point.z_top > door.middle[2]),
  );
}
door.passageStructure = { volumes, blockers: [] };
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-east-gate-structure-");
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
