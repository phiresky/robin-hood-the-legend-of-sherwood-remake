// Render existing editor-authored diagnostic scenes without recreating placements.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { serializeStoredMap } from "../shared/src/stored-level.ts";

const inputs = process.argv.slice(2);
assert.ok(inputs.length, "Supply name=editor-scene.json entries");
const scenes = [],
  names = new Set();
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
for (const input of inputs) {
  const separator = input.indexOf("=");
  assert.ok(separator > 0);
  const name = input.slice(0, separator),
    file = input.slice(separator + 1);
  assert.match(name, /^[a-z0-9-]+$/);
  assert.ok(!names.has(name), "Duplicate fixture name");
  names.add(name);
  const document = JSON.parse(await fs.readFile(file));
  const assets = new Map();
  for (const reference of document.assetSources) {
    const bytes = await fs.readFile(path.join("library", reference.descriptor));
    assert.equal(hash(bytes), reference.descriptor_sha256, `${reference.id}: descriptor changed`);
    const model = await fs.readFile(path.join("library", reference.model));
    assert.equal(hash(model), reference.model_sha256, `${reference.id}: model changed`);
    const descriptor = JSON.parse(bytes);
    assert.equal(descriptor.id, reference.id);
    assert.ok(descriptor.gameplay, `${reference.id}: missing gameplay`);
    assets.set(reference.id, descriptor);
  }
  document.map = name;
  scenes.push({
    name,
    file,
    assets: document.assetSources,
    stored: serializeStoredMap(document, assets),
  });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/authored-render-library-");
await fs.mkdir(`${output}/scenes`);
for (const directory of ["3d-assets", "game-data", "population"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
for (const scene of scenes)
  await fs.writeFile(`${output}/scenes/${scene.name}.rhlos-map.json`, JSON.stringify(scene.stored));
await fs.writeFile(
  `${output}/preparation.json`,
  JSON.stringify({
    scope: "pinned-editor-scene-preparation-not-render-or-gameplay-proof",
    scenes: scenes.map(({ stored: _stored, ...scene }) => scene),
  }),
);
console.log(output);
