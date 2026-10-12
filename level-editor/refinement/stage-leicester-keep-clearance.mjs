// Restore reviewed movement contours while retaining each roof's receiving footprint.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const scene = await readStoredMap("library/scenes/leicester.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const id = "leicester-great-keep";
const asset = assets.get(id);
const source = scene.assetSources.find((entry) => entry.id === id);
const stored = JSON.parse(await fs.readFile("library/scenes/leicester.rhlos-map.json"));
const placement = stored.placements.find((entry) => entry.id === id);
assert.ok(asset && source && placement);
assert.equal(placement.transform.rot_deg, 0);
const imported = JSON.parse(await fs.readFile("library/game-data/Data/Levels/Leicester.rhp.json"));
const areas = new Map();
let index = 0;
for (const layer of imported.motion_data.layers)
  for (const area of layer) {
    areas.set(index, area);
    index += 1 + area.obstacles.length;
  }
const gameplay = structuredClone(asset.gameplay);
const review = [];
for (const [node, sector] of [
  [312, 74],
  [311, 75],
  [308, 76],
  [307, 77],
]) {
  const surface = gameplay.surfaces.find((entry) => entry.id === `building-${node}-walk-0`);
  const area = areas.get(sector);
  assert.ok(surface && area && surface.projectionMaterials?.footprint);
  assert.equal(surface.navigationRegion, `walk-region-${sector}`);
  assert.equal(area.obstacles.length, 0);
  assert.ok(Array.isArray(surface.height));
  const height = surface.height[0];
  assert.ok(surface.height.every((value) => Math.abs(value - height) < 1e-6));
  const before = structuredClone(surface);
  surface.polygon = area.polygon.points.map(([x, y]) => [
    x - placement.transform.dx,
    y + height + placement.transform.dz - placement.transform.dy,
  ]);
  surface.height = surface.polygon.map(() => height);
  review.push({ before, after: surface });
}
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/leicester-keep-clearance-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "staged-asset-local-clearance-requires-placement-and-traversal-validation",
    source,
    review,
  }),
);
console.log(output);
