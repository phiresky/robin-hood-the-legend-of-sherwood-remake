// Freeze the exact meshes reviewed alongside staged gameplay before browser verification.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";

const [map, stage] = process.argv.slice(2);
assert.ok(map && stage, "Supply a saved map and gameplay stage");
const scene = JSON.parse(await fs.readFile(`library/scenes/${map}.rhlos-map.json`));
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const review = [];
for (const edit of edits) {
  const source = scene.assetSources.find((source) => source.id === edit.asset);
  assert.ok(source, `Missing scene asset ${edit.asset}`);
  assert.equal(edit.descriptorSha256, source.descriptor_sha256);
  assert.equal(hash(await fs.readFile(`library/${source.descriptor}`)), edit.descriptorSha256);
  const modelSha256 = hash(await fs.readFile(`library/${source.model}`));
  assert.equal(modelSha256, source.model_sha256);
  if (edit.modelSha256 !== undefined) assert.equal(edit.modelSha256, modelSha256);
  review.push({ asset: edit.asset, previouslyPinned: edit.modelSha256 !== undefined, modelSha256 });
  edit.modelSha256 = modelSha256;
}
const output = await fs.mkdtemp("work/needs-review/map-compile/pinned-gameplay-stage-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ map, stage, review }, null, 2));
console.log(output);
