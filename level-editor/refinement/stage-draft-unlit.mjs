// Restore the authored unlit material flag without rewriting mesh or texture buffers.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";

const [descriptorFile] = process.argv.slice(2);
assert.ok(descriptorFile, "Supply an unfinished physical asset descriptor");
const descriptorBytes = await fs.readFile(descriptorFile);
const descriptor = JSON.parse(descriptorBytes);
assert.equal(descriptor.asset_type, "Unfinished physical asset");
assert.ok(descriptor.tags.includes("appearance-incomplete"));
assert.equal(descriptor.model, "model.glb");
const source = path.join(path.dirname(descriptorFile), descriptor.model);
const before = await fs.readFile(source);
assert.equal(before.readUInt32LE(0), 0x46546c67);
assert.equal(before.readUInt32LE(4), 2);
assert.equal(before.readUInt32LE(8), before.length);
assert.equal(before.readUInt32LE(16), 0x4e4f534a);
const jsonLength = before.readUInt32LE(12);
const model = JSON.parse(before.toString("utf8", 20, 20 + jsonLength));
const original = structuredClone(model);
const changed = [];
for (const [index, material] of model.materials.entries()) {
  if (material.name !== "Unfinished volume preview" || material.extensions?.KHR_materials_unlit)
    continue;
  assert.deepEqual(material.pbrMetallicRoughness?.baseColorFactor, [0.55, 0.45, 0.3, 1]);
  material.extensions = { ...material.extensions, KHR_materials_unlit: {} };
  changed.push(index);
}
assert.ok(changed.length, "No legacy draft material requires repair");
model.extensionsUsed = [...new Set([...(model.extensionsUsed ?? []), "KHR_materials_unlit"])];
const json = Buffer.from(JSON.stringify(model));
const padded = Buffer.alloc(Math.ceil(json.length / 4) * 4, 0x20);
json.copy(padded);
const header = Buffer.from(before.subarray(0, 20));
const remainder = before.subarray(20 + jsonLength);
header.writeUInt32LE(20 + padded.length + remainder.length, 8);
header.writeUInt32LE(padded.length, 12);
const after = Buffer.concat([header, padded, remainder]);
assert.deepEqual(after.subarray(20 + padded.length), remainder, "Binary chunks changed");
for (const key of Object.keys(original))
  if (key !== "materials" && key !== "extensionsUsed") assert.deepEqual(model[key], original[key]);
const output = await fs.mkdtemp("work/needs-review/map-compile/draft-unlit-");
await fs.writeFile(`${output}/model.glb`, after);
await fs.writeFile(`${output}/asset.json`, descriptorBytes);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify({
    scope: "staged-draft-material-only-not-published-or-render-verified",
    asset: descriptor.id,
    source,
    sourceSha256: sha(before),
    descriptorSha256: sha(descriptorBytes),
    modelSha256: sha(after),
    binaryChunksSha256: sha(remainder),
    changedMaterials: changed,
  }),
);
console.log(output);
