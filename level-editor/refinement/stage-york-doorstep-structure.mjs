// Stage explicit doorway crossings without changing building collision geometry.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const candidates = [
  [
    "narrow-timber-house",
    "building-582",
    "44bb736bed262f663f55d10e8fda137bafa13d21d70471433e9407c27cd01efe",
  ],
  [
    "stone-tower",
    "building-579",
    "173313f792b583e428698757684f77659831d65e2d9d2b197bff707cc5dbe4aa",
  ],
];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const edits = [];
for (const [suffix, volume, expected] of candidates) {
  const asset = `york-west-town-lane-${suffix}`;
  const directory = `library/3d-assets/york/${asset}`;
  const bytes = await fs.readFile(`${directory}/asset.json`);
  assert.equal(hash(bytes), expected, `${asset}: re-review changed asset`);
  const descriptor = JSON.parse(bytes),
    gameplay = descriptor.gameplay;
  const before = structuredClone(gameplay);
  const door = gameplay.interiors[0].doors[0];
  assert.equal(door.node, volume);
  assert.equal(door.passageStructure, undefined);
  assert.ok(gameplay.movementSolids.includes(volume));
  assert.ok(
    descriptor.parts.some((part) => part.node === volume && part.obstacle_local_game.solid),
  );
  door.passageStructure = { volumes: [volume], blockers: [] };
  validateAssetGameplay(gameplay, descriptor);
  const unannotated = structuredClone(gameplay);
  delete unannotated.interiors[0].doors[0].passageStructure;
  assert.deepEqual(unannotated, before);
  edits.push({
    asset,
    descriptorSha256: hash(bytes),
    modelSha256: hash(await fs.readFile(`${directory}/${descriptor.model}`)),
    gameplay,
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-doorstep-structure-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
console.log(output);
