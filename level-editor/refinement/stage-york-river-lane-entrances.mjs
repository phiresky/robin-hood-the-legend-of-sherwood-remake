// Keep each house's structural doorway crossing local when copied and rotated.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const reviewed = [
  [
    "stone-house",
    "a81f11965f1d86aea41a0bfeae54261098c215a93c0b1a1b0469b97b713d3306",
    ["building-710-ground-blocker-182-0-0"],
  ],
  [
    "west-low-house",
    "c14b80a506edcca8d7a9aca8fc43c5c6eff7269067e184180be448f7ce896bee",
    ["building-722-ground-blocker-187-0-0", "building-729-ground-blocker-189-0-0"],
  ],
];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const edits = [];
for (const [suffix, expected, blockers] of reviewed) {
  const asset = `york-north-river-lane-${suffix}`;
  const directory = `library/3d-assets/york/${asset}`;
  const bytes = await fs.readFile(`${directory}/asset.json`);
  assert.equal(hash(bytes), expected);
  const descriptor = JSON.parse(bytes),
    gameplay = descriptor.gameplay;
  const before = structuredClone(gameplay);
  const door = gameplay.interiors[0].doors[0];
  assert.equal(door.id, "interior-0/door-0");
  assert.equal(door.passageStructure, undefined);
  for (const id of blockers) {
    const blocker = gameplay.movementBlockers.find((blocker) => blocker.id === id);
    assert.ok(blocker && descriptor.parts.some((part) => part.node === blocker.node));
  }
  door.passageStructure = { volumes: [], blockers };
  validateAssetGameplay(gameplay, descriptor);
  const unannotated = structuredClone(gameplay);
  delete unannotated.interiors[0].doors[0].passageStructure;
  assert.deepEqual(unannotated, before, "Keep movement, receivers and render geometry intact");
  edits.push({
    asset,
    descriptorSha256: hash(bytes),
    modelSha256: hash(await fs.readFile(`${directory}/${descriptor.model}`)),
    gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-river-lane-entrances-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
