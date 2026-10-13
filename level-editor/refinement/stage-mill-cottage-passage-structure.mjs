// Declare only the local structure crossed by the two entrance animations.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const asset = "leicester-mill-north-cottage";
const directory = `library/3d-assets/leicester/${asset}`;
const bytes = await fs.readFile(`${directory}/asset.json`);
const descriptorSha256 = hash(bytes);
assert.equal(descriptorSha256, "054fba217c0dc7f5c6a2dd0a57efecd504700ff3b92803e9396af45aa2a338f5");
const descriptor = JSON.parse(bytes);
const before = structuredClone(descriptor.gameplay);
for (const [index, blocker] of [
  "building-016-ground-blocker-38-0-0",
  "building-018-ground-blocker-40-0-0",
].entries()) {
  const door = descriptor.gameplay.interiors[index].doors[0];
  assert.equal(
    descriptor.gameplay.movementBlockers.find((row) => row.id === blocker)?.node,
    door.node,
  );
  door.passageStructure = { volumes: [], blockers: [blocker] };
}
validateAssetGameplay(descriptor.gameplay, descriptor);
const withoutAnnotations = structuredClone(descriptor.gameplay);
for (const room of withoutAnnotations.interiors)
  for (const door of room.doors) delete door.passageStructure;
assert.deepEqual(
  withoutAnnotations,
  before,
  "Walking, sight and render definitions must remain unchanged",
);
const modelSha256 = hash(await fs.readFile(`${directory}/${descriptor.model}`));
const output = await fs.mkdtemp("work/needs-review/map-compile/mill-cottage-passage-structure-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([{ asset, descriptorSha256, modelSha256, gameplay: descriptor.gameplay }]),
);
console.log(output);
