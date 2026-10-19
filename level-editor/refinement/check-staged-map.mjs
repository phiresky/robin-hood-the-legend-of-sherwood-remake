// Compile a staged editor scene and asset definitions without changing library pins.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { savedMapBakeBounds } from "../app/tests/saved-map-bounds.mjs";
import { savedMapWallCalibration } from "../app/tests/saved-map-wall-calibration.mjs";

const [stage, map] = process.argv.slice(2);
assert.ok(stage && map && /^[a-z0-9-]+$/.test(map), "Supply stage directory and map name");
const document = JSON.parse(await fs.readFile(`${stage}/scene.json`));
delete document.mission;
delete document.population;
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
for (const edit of JSON.parse(await fs.readFile(`${stage}/edits.json`))) {
  const ref = document.assetSources.find((entry) => entry.id === edit.asset);
  assert.equal(ref?.descriptor_sha256, edit.descriptorSha256);
  assert.equal(ref?.model_sha256, edit.modelSha256);
  assert.ok(assets.has(edit.asset));
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/needs-review/map-compile/staged-map-export-");
console.log(output);
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({ complete: false, stage }));
const bounds = await savedMapBakeBounds(document);
const prepared = await savedMapWallCalibration(document, assets);
const collisions = [];
const compiled = compileMap(document, bounds, prepared.assets, {
  bestEffort: true, onProgress: console.log,
  onPassageCollision: (collision) => collisions.push(collision),
});
assert.deepEqual(compiled.descriptor.spawn_points, []);
assert.equal(compiled.descriptor.soldiers, undefined);
const file = `${map}.level.json`;
await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
await fs.writeFile(`${output}/${map}.passage-collisions.json`, JSON.stringify(collisions));
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "static-geometry-only-not-gameplay-parity", complete: true, mapOnly: true, stage,
  results: [{ map, file, bounds, warnings: [...prepared.warnings, ...compiled.warnings] }],
}));
console.log(`${map}: compiled`);
