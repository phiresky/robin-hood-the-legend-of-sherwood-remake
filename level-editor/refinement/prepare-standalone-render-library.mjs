// Build a browser round-trip fixture through the editor's normal insertion path.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { serializeStoredMap } from "../shared/src/stored-level.ts";

const [idList, rotationText = "37", heightText = "40", sizeText = "1000"] = process.argv.slice(2);
assert.ok(idList, "Supply comma-separated published asset IDs");
const ids = idList.split(",");
assert.equal(new Set(ids).size, ids.length, "Duplicate fixture asset");
const rotation = Number(rotationText),
  height = Number(heightText);
assert.ok(Number.isFinite(rotation) && Number.isFinite(height));
const size = Number(sizeText);
assert.ok(Number.isSafeInteger(size) && size >= 1000);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json"));
const assets = new Map(),
  references = new Map();
for (const id of ids) {
  const entry = index.assets.find((entry) => entry.id === id);
  assert.ok(entry);
  const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
  const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
  assert.equal(hash(bytes), entry.descriptor_sha256);
  const descriptor = JSON.parse(bytes);
  assert.ok(descriptor.gameplay, "Publish gameplay before preparing this fixture");
  const modelHash = hash(await fs.readFile(`library/3d-assets/${entry.model}`));
  if (entry.model_sha256) assert.equal(modelHash, entry.model_sha256);
  const reference = {
    id,
    descriptor: `3d-assets/${entry.descriptor}`,
    descriptor_sha256: hash(bytes),
    model: `3d-assets/${entry.model}`,
    model_sha256: modelHash,
    model_scene: descriptor.model_scene,
    resources: descriptor.resources,
  };
  assets.set(id, descriptor);
  references.set(id, reference);
}
let document = {
  version: 1,
  map: "standalone-review",
  size: [size, size],
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  objects: [],
  groups: [],
  sceneAssets: [],
  assetSources: [],
  terrain: createTerrainGrid([0, 0, size, size], size / 2, height),
};
for (const [assetIndex, id] of ids.entries()) {
  for (const x of [size * 0.25 + 0.25, size * 0.7 + 0.25]) {
    const descriptor = assets.get(id),
      reference = references.get(id);
    const inserted = insertProjectionAsset(document, descriptor, reference, [
      x,
      (size * (assetIndex + 1)) / (ids.length + 1) + 0.75,
      height,
    ]);
    document = inserted.document;
    document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg =
      rotation;
  }
}
const output = await fs.mkdtemp("work/needs-review/map-compile/standalone-render-library-");
await fs.mkdir(`${output}/scenes`);
for (const directory of ["3d-assets", "game-data", "population"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
await fs.writeFile(
  `${output}/scenes/standalone-review.rhlos-map.json`,
  JSON.stringify(serializeStoredMap(document, assets)),
);
await fs.writeFile(
  `${output}/preparation.json`,
  JSON.stringify({
    scope: "browser-fixture-preparation-not-render-or-gameplay-proof",
    asset: ids.length === 1 ? references.get(ids[0]) : undefined,
    assets: [...references.values()],
    rotation,
    height,
    copies: 2,
  }),
);
console.log(output);
