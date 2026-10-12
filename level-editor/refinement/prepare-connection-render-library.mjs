// Prepare a private browser-bake fixture using complete published asset definitions.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, groupParts } from "../shared/src/level3d.ts";
import { serializeStoredMap } from "../shared/src/stored-level.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";

const [map, assetList, rotationText = "0", heightText = "0"] = process.argv.slice(2);
assert.ok(map && assetList, "Supply saved map, comma-separated asset IDs, rotation and elevation");
const ids = assetList.split(",");
assert.equal(new Set(ids).size, ids.length);
const rotation = Number(rotationText),
  height = Number(heightText);
assert.ok(Number.isFinite(rotation) && Number.isFinite(height));
const saved = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", saved.assetSources, saved.sceneAssets);
for (const id of ids) assert.ok(assets.has(id), `Missing pinned asset ${id}`);
const objects = saved.objects.filter((part) =>
  ids.some((id) => part.node.startsWith(`asset:${id}:`)),
);
const groups = saved.groups.filter((group) => objects.some((part) => part.group === group.id));
assert.equal(groups.length, ids.length, "Expected one saved placement per selected asset");
const centers = groups.map((group) => {
  const [x, y] = groupCentroid(groupParts(saved, group.id));
  return [x + group.transform.dx, y + group.transform.dy];
});
for (const group of groups) {
  group.transform.dx += 600 - centers.reduce((sum, point) => sum + point[0] / centers.length, 0);
  group.transform.dy += 900 - centers.reduce((sum, point) => sum + point[1] / centers.length, 0);
}
const source = {
  version: 1,
  map: "connection-review",
  camera: saved.camera,
  size: [2500, 2500],
  objects,
  groups,
  assetSources: saved.assetSources.filter((entry) => ids.includes(entry.id)),
  sceneAssets: [],
};
const document = roofJumpPlacement(source, rotation, height);
document.terrain = createTerrainGrid([0, 0, 2500, 2500], 2500, height);
const output = await fs.mkdtemp("work/needs-review/map-compile/render-connection-library-");
await fs.mkdir(`${output}/scenes`);
for (const directory of ["3d-assets", "game-data"])
  await fs.symlink(path.resolve("library", directory), `${output}/${directory}`, "dir");
await fs.writeFile(
  `${output}/scenes/connection-review.rhlos-map.json`,
  JSON.stringify(serializeStoredMap(document, assets)),
);
await fs.writeFile(
  `${output}/preparation.json`,
  JSON.stringify({
    scope: "browser-fixture-preparation-not-render-or-gameplay-proof",
    sourceMap: map,
    assets: document.assetSources,
    rotation,
    height,
    copies: 2,
  }),
);
console.log(output);
