// Preserve this elevated canopy's authored ground datum during editor insertion.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const id = "york-market-stepped-green-canopy-stall";
const base = `library/3d-assets/york/${id}`;
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`${base}/asset.json`);
const descriptorSha256 = hash(bytes);
assert.equal(descriptorSha256, "5b8ba09b0d1558dc3d5d977888c091d85e5e912e1fe4c44ba5c41394543a477e");
const descriptor = JSON.parse(bytes);
assert.equal(descriptor.source_origin_scene[2], 0);
assert.equal(descriptor.gameplay.placementGroundHeight, undefined);
assert.equal(descriptor.parts.length, 1);
assert.equal(descriptor.parts[0].node, "building-044");
assert.ok(descriptor.parts[0].obstacle_local_game.points.every((point) => point.z_bottom === 150));
const gameplay = structuredClone(descriptor.gameplay);
gameplay.placementGroundHeight = 0;
gameplay.draft.issues.push(
  "Elevated canopy uses an explicit local ground datum of zero; adjoining support geometry and access must be provided by the assembled map.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/stepped-canopy-ground-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256,
      modelSha256: hash(await fs.readFile(`${base}/model.glb`)),
      gameplay,
    },
  ]),
);
console.log(output);
