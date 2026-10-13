// Build a browser round-trip fixture through the editor's normal insertion path.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { serializeStoredMap } from "../shared/src/stored-level.ts";

const [id, rotationText = "37", heightText = "40"] = process.argv.slice(2);
assert.ok(id, "Supply a published asset ID");
const rotation = Number(rotationText),
  height = Number(heightText);
assert.ok(Number.isFinite(rotation) && Number.isFinite(height));
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json"));
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
let document = {
  version: 1,
  map: "standalone-review",
  size: [1000, 1000],
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  objects: [],
  groups: [],
  sceneAssets: [],
  assetSources: [],
  terrain: createTerrainGrid([0, 0, 1000, 1000], 500, height),
};
for (const x of [250.25, 700.25]) {
  const inserted = insertProjectionAsset(document, descriptor, reference, [x, 500.75, height]);
  document = inserted.document;
  document.groups.find((group) => group.id === inserted.selection.id).transform.rot_deg = rotation;
}
const output = await fs.mkdtemp("work/needs-review/map-compile/standalone-render-library-");
await fs.mkdir(`${output}/scenes`);
for (const directory of ["3d-assets", "game-data", "population"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
await fs.writeFile(
  `${output}/scenes/standalone-review.rhlos-map.json`,
  JSON.stringify(serializeStoredMap(document, new Map([[id, descriptor]]))),
);
await fs.writeFile(
  `${output}/preparation.json`,
  JSON.stringify({
    scope: "browser-fixture-preparation-not-render-or-gameplay-proof",
    asset: reference,
    rotation,
    height,
    copies: 2,
  }),
);
console.log(output);
