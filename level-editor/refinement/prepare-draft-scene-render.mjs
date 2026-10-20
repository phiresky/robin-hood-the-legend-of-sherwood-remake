// Package an existing placement fixture with reviewed asset-local gameplay edits.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { serializeStoredMap } from "../shared/src/stored-level.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [scenePath, editsPath, name, mode] = process.argv.slice(2);
assert.ok(scenePath && editsPath && name, "Supply scene.json edits.json fixture-name");
assert.ok(
  mode === undefined || mode === "--publication" || mode.startsWith("--model-overrides="),
  "Unknown preparation mode",
);
const catalog =
  mode === "--publication"
    ? JSON.parse(await fs.readFile("library/3d-assets/index.json"))
    : undefined;
const overrides = mode?.startsWith("--model-overrides=")
  ? JSON.parse(await fs.readFile(mode.slice("--model-overrides=".length)))
  : [];
assert.ok(Array.isArray(overrides));
assert.equal(new Set(overrides.map((entry) => entry.asset)).size, overrides.length);
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
  const replacement = overrides.find((entry) => entry.asset === reference.id);
  let replacementBytes;
  if (replacement) {
    assert.ok(edit, "A replaced model requires paired gameplay metadata");
    assert.equal(replacement.sourceSha256, hash(model), "Replacement source model changed");
    replacementBytes = await fs.readFile(replacement.path);
    assert.equal(hash(replacementBytes), replacement.sha256, "Reviewed replacement changed");
    assert.equal(replacementBytes.subarray(0, 4).toString(), "glTF", "Replacement must be GLB");
    reference.model_sha256 = replacement.sha256;
  }
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
  if (replacementBytes) await fs.writeFile(path.join(output, reference.model), replacementBytes);
  else await fs.link(path.resolve("library", reference.model), path.join(output, reference.model));
  descriptors.set(descriptor.id, descriptor);
  const publicationFields = {};
  if (catalog) {
    const current = catalog.assets.find((entry) => entry.id === descriptor.id);
    assert.ok(current, `Missing library catalog entry: ${descriptor.id}`);
    assert.equal(current.descriptor_sha256, hash(bytes));
    if (current.model_sha256 !== undefined) assert.equal(current.model_sha256, hash(model));
    assert.equal(`3d-assets/${current.descriptor}`, reference.descriptor);
    assert.equal(`3d-assets/${current.model}`, reference.model);
    for (const key of ["lossy_model", "preview_model"]) {
      assert.ok(current[key], `Missing ${key}: ${descriptor.id}`);
      publicationFields[key] = current[key];
      for (const suffix of ["", ".receipt.json"]) {
        const file = relative(`3d-assets/${current[key]}${suffix}`);
        const target = path.join(output, file);
        await fs.mkdir(path.dirname(target), { recursive: true });
        await fs.copyFile(path.join("library", file), target);
      }
    }
    if (descriptor.model_scene !== undefined && descriptor.resources === undefined) {
      const receipt = path.join(path.dirname(reference.descriptor), "bundle.receipt.json");
      await fs.copyFile(path.join("library", receipt), path.join(output, receipt));
    }
  }
  entries.push({
    id: descriptor.id,
    name: descriptor.name,
    source_map: descriptor.source_map,
    descriptor: reference.descriptor.slice(10),
    model: reference.model.slice(10),
    descriptor_sha256: reference.descriptor_sha256,
    model_sha256: reference.model_sha256,
    model_scene: descriptor.model_scene,
    ...publicationFields,
    editor: descriptor,
  });
}
for (const edit of edits) assert.ok(descriptors.has(edit.asset), "Unused gameplay edit");
for (const replacement of overrides)
  assert.ok(descriptors.has(replacement.asset), "Unused model replacement");
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
    modelOverrides: overrides,
    assets: document.assetSources,
  }),
);
for (const directory of ["game-data", "population"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
console.log(output);
