// Stage a receiving plane from an existing walkable surface instead of a collision volume.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { pinnedDescriptors, readStoredMap } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const [map, asset, surfaceId] = process.argv.slice(2);
assert.ok(map && asset && surfaceId, "Supply map, asset ID and walkable surface ID");
const document = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const descriptors = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const descriptor = descriptors.get(asset);
assert.ok(descriptor?.gameplay, `Missing gameplay for ${asset}`);
const source = document.assetSources.find((entry) => entry.id === asset);
assert.ok(source, `Missing asset source for ${asset}`);
const gameplay = structuredClone(descriptor.gameplay);
const surface = gameplay.surfaces.find((entry) => entry.id === surfaceId);
assert.ok(surface?.projectionVolume, `Surface ${surfaceId} has no volume binding`);
const previousVolume = surface.projectionVolume;
delete surface.projectionVolume;
validateAssetGameplay(gameplay, descriptor);
const bytes = await fs.readFile(`library/${source.descriptor}`);
const digest = createHash("sha256").update(bytes).digest("hex");
assert.equal(digest, source.descriptor_sha256);
const output = await fs.mkdtemp("work/needs-review/map-compile/surface-receiver-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([{ asset, descriptorSha256: digest, modelSha256: source.model_sha256, gameplay }]),
);
await fs.writeFile(
  `${output}/report.json`,
  JSON.stringify({ asset, surface: surfaceId, previousVolume, applied: false }),
);
console.log(output);
