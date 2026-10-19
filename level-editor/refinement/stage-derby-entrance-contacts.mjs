// Stage exterior landing sockets for independent placement verification.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the reviewed doorway-structure stage");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const definitions = [
  ["derby-east-watchtower", "building-213-walk-0", [5, 0]],
  ["derby-keep-central-gallery", "building-157-walk-0", [20, 21]],
];
const changes = [];
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const [id, surfaceId, indices] of definitions) {
  const edit = edits.find((entry) => entry.asset === id);
  assert.ok(edit);
  const root = `library/3d-assets/derby/${id}`;
  const bytes = await fs.readFile(`${root}/asset.json`);
  assert.equal(hash(bytes), edit.descriptorSha256);
  const asset = JSON.parse(bytes);
  assert.equal(hash(await fs.readFile(`${root}/${asset.model}`)), edit.modelSha256);
  const surface = edit.gameplay.surfaces.find((entry) => entry.id === surfaceId);
  assert.ok(surface && !surface.navigationJoins && !surface.navigationRegion);
  const before = structuredClone(surface);
  surface.navigationRegion = surface.id;
  surface.navigationJoins = [indices.map((i) => [...surface.polygon[i], surface.height[i]])];
  surface.preserveMovementPrecision = true;
  validateAssetGameplay(edit.gameplay, asset);
  changes.push({ asset: id, before, after: surface });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/derby-entrance-contacts-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify(
    {
      scope: "unpublished-exterior-sockets-require-terrain-click-verification",
      stage,
      changes,
    },
    null,
    2,
  ),
);
console.log(output);
