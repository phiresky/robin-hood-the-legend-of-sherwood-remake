// Restore an asset-local roof outline and a supported takeoff for placement review.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const scene = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const id = "york-west-town-lane-rear-timber-house";
const asset = assets.get(id);
const source = scene.assetSources.find((source) => source.id === id);
assert.ok(asset && source);
const gameplay = structuredClone(asset.gameplay);
const roof = gameplay.surfaces.find((surface) => surface.id === "building-594-walk-0");
assert.ok(roof && gameplay.surfaces.length === 2);
assert.ok(gameplay.surfaces.some((surface) => surface.id === "building-594-walk-1"));
const edge = gameplay.jumpSegments.find((segment) => segment.id === "jump-pair-55-side-0")?.edge;
assert.ok(edge && Array.isArray(roof.height));
const before = { surfaces: structuredClone(gameplay.surfaces), edge: structuredClone(edge) };
const authoredPlane = heightPlane(roof.polygon.map(([x, y], i) => [x, y, roof.height[i]]));
const physicalPlane = heightPlane(roof.projectionMaterials.planePoints);
const footprint = roof.projectionMaterials.footprint;
assert.ok(footprint.length >= 3 && Math.abs(physicalPlane[1]) > 1e-6);
assert.ok(footprint.every(([x, y, z]) => Math.abs(z - planeHeight(physicalPlane, [x, y])) < 0.001));
roof.polygon = footprint.map(([x, y]) => [x, y]);
roof.height = roof.polygon.map((point) => planeHeight(physicalPlane, point));
gameplay.surfaces = [roof];

// Keep the short level takeoff on the roof, away from its eave. The shift
// follows a constant-height contour; its physical mesh contact is reviewed
// separately, as are native walking and flight clearances after rotation.
const midpoint = edge.a.map((n, axis) => (n + edge.b[axis]) / 2);
const height = planeHeight(authoredPlane, midpoint);
const dx = -8;
const dy = (-physicalPlane[0] * dx) / physicalPlane[1];
for (const point of [edge.a, edge.b]) {
  point[0] += dx;
  point[1] += dy;
  point[2] = height;
}
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/map-compile/york-west-lane-roof-");
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
    scope: "staged-roof-definition-requires-mesh-and-native-placement-review",
    source,
    before,
    after: { surfaces: gameplay.surfaces, edge },
    inset: [dx, dy],
  }),
);
console.log(output);
