// Package an existing placement fixture with reviewed asset-local gameplay edits.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { serializeStoredMap } from "../shared/src/stored-level.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [scenePath, editsPath, name] = process.argv.slice(2);
assert.ok(scenePath && editsPath && name, "Supply scene.json edits.json fixture-name");
assert.match(name, /^[a-z0-9-]+$/);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const relative = (value) => {
  assert.ok(typeof value === "string" && value.startsWith("3d-assets/"));
  assert.ok(!/[\\\0:#?%]/.test(value));
  assert.ok(value.split("/").every((part) => part && part !== "." && part !== ".."));
  return value;
};
const document = JSON.parse(await fs.readFile(scenePath));
const edits = JSON.parse(await fs.readFile(editsPath));
assert.equal(new Set(edits.map((edit) => edit.asset)).size, edits.length);
const output = await fs.mkdtemp("work/needs-review/map-compile/draft-scene-render-");
const descriptors = new Map(),
  entries = [];
for (const reference of document.assetSources) {
  relative(reference.descriptor);
  relative(reference.model);
  const bytes = await fs.readFile(path.join("library", reference.descriptor));
  const model = await fs.readFile(path.join("library", reference.model));
  assert.equal(hash(bytes), reference.descriptor_sha256);
  assert.equal(hash(model), reference.model_sha256);
  const descriptor = JSON.parse(bytes);
  assert.equal(descriptor.id, reference.id);
  assert.ok(reference.model.endsWith(".glb"), "Fixture requires bundled GLB models");
  const edit = edits.find((row) => row.asset === reference.id);
  if (edit) {
    assert.equal(hash(bytes), edit.descriptorSha256);
    assert.equal(hash(model), edit.modelSha256);
    descriptor.gameplay = edit.gameplay;
    for (const [node, appearance] of Object.entries(edit.partAppearances ?? {})) {
      const parts = descriptor.parts.filter((part) => part.node === node);
      assert.equal(parts.length, 1, "Appearance edit needs one existing part");
      parts[0].appearance = appearance;
    }
  }
  for (const resource of descriptor.resources ?? []) {
    const file = relative(resource.path);
    const data = await fs.readFile(path.join("library", file));
    assert.equal(hash(data), resource.sha256, "Model resource changed");
    const target = path.join(output, file);
    await fs.mkdir(path.dirname(target), { recursive: true });
    await fs.writeFile(target, data);
  }
  validateAssetGameplay(descriptor.gameplay, descriptor);
  const updated = JSON.stringify(descriptor);
  reference.descriptor_sha256 = hash(updated);
  assert.ok(reference.descriptor.startsWith("3d-assets/"));
  assert.ok(reference.model.startsWith("3d-assets/"));
  await fs.mkdir(path.dirname(path.join(output, reference.descriptor)), { recursive: true });
  await fs.writeFile(path.join(output, reference.descriptor), updated);
  await fs.link(path.resolve("library", reference.model), path.join(output, reference.model));
  descriptors.set(descriptor.id, descriptor);
  entries.push({
    id: descriptor.id,
    name: descriptor.name,
    source_map: descriptor.source_map,
    descriptor: reference.descriptor.slice(10),
    model: reference.model.slice(10),
    descriptor_sha256: reference.descriptor_sha256,
    model_sha256: reference.model_sha256,
    model_scene: descriptor.model_scene,
    editor: descriptor,
  });
}
for (const edit of edits) assert.ok(descriptors.has(edit.asset), "Unused gameplay edit");
await fs.writeFile(
  `${output}/3d-assets/index.json`,
  JSON.stringify({ version: 1, assets: entries }),
);
await fs.mkdir(`${output}/scenes`);
document.map = name;
await fs.writeFile(
  `${output}/scenes/${name}.rhlos-map.json`,
  JSON.stringify(serializeStoredMap(document, descriptors)),
);
await fs.writeFile(
  `${output}/preparation.json`,
  JSON.stringify({
    scope: "draft-scene-render-preparation-not-gameplay-proof",
    scenePath,
    editsPath,
    assets: document.assetSources,
  }),
);
for (const directory of ["game-data", "population"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
console.log(output);
