// Stage a jump selection region on an existing asset-local walking surface.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const [map, assetId, zoneId, surfaceId] = process.argv.slice(2);
assert.ok(map && assetId && zoneId && surfaceId, "Supply map, asset, jump zone and surface IDs");
const scene = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const asset = assets.get(assetId);
const source = scene.assetSources.find((source) => source.id === assetId);
assert.ok(asset && source);
const gameplay = structuredClone(asset.gameplay);
const zone = gameplay.jumpZones.find((zone) => zone.id === zoneId);
const surface = gameplay.surfaces.find((surface) => surface.id === surfaceId);
assert.ok(zone && surface && !surface.holes?.length, "Require a zone and a hole-free surface");
const before = structuredClone(zone);
zone.polygon = surface.polygon.map(([x, y], i) => [
  x,
  y,
  Array.isArray(surface.height) ? surface.height[i] : surface.height,
]);
assert.ok(
  pointInGameplayPolygon(zone.anchor, surface.polygon, true) &&
    Math.abs(planeHeight(heightPlane(zone.polygon), zone.anchor) - zone.anchor[2]) < 0.01,
  "The existing zone anchor must belong to the selected surface",
);
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/map-compile/jump-zone-surface-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: assetId,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "staged-selection-region-requires-placement-and-native-review",
    source,
    surfaceId,
    before,
    after: zone,
  }),
);
console.log(output);
