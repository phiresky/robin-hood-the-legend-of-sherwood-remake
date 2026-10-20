// Stage a shared ridge while preserving both independently authored roof planes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const receiverVolumes = process.argv.includes("--receiver-volumes");
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const id = "york-central-south-golden-timber-house";
const ref = scene.assetSources.find((source) => source.id === id);
assert.ok(ref);
const bytes = await fs.readFile(`library/${ref.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
const asset = JSON.parse(bytes);
const gameplay = structuredClone(asset.gameplay);
const surfaces = ["building-339-walk-0", "building-341-walk-0"].map((surfaceId) =>
  gameplay.surfaces.find((surface) => surface.id === surfaceId),
);
assert.ok(surfaces.every(Boolean));
assert.equal(surfaces[0].navigationRegion, surfaces[1].navigationRegion);
const before = structuredClone(surfaces);
const planes = surfaces.map((surface) => heightPlane(surface.projectionMaterials.planePoints));
const difference = planes[0].map((n, i) => n - planes[1][i]);
const denominator = difference[0] ** 2 + difference[1] ** 2;
assert.ok(denominator > 1e-8);
const ridge = [];
for (const [a, b] of [
  [2, 1],
  [3, 0],
]) {
  const average = surfaces[0].polygon[a].map((n, i) => (n + surfaces[1].polygon[b][i]) / 2);
  const residual = difference[0] * average[0] + difference[1] * average[1] + difference[2];
  const point = average.map((n, i) => n - (difference[i] * residual) / denominator);
  for (const [side, index] of [
    [0, a],
    [1, b],
  ]) {
    assert.ok(Math.hypot(...point.map((n, i) => n - surfaces[side].polygon[index][i])) < 2);
    surfaces[side].polygon[index] = [...point];
  }
  assert.ok(Math.abs(planeHeight(planes[0], point) - planeHeight(planes[1], point)) < 1e-8);
  ridge.push(point);
}
for (const [side, surface] of surfaces.entries()) {
  surface.height = surface.polygon.map((point) => planeHeight(planes[side], point));
  // Keep the asset's movement-grid policy; changing it also moves outer boundaries.
  surface.projectionMaterials.footprint = surface.polygon.map((point, i) => [
    ...point,
    surface.height[i],
  ]);
  surface.projectionMaterials.priorityHeight = Math.max(...surface.height);
  if (receiverVolumes) {
    const points = surface.projectionMaterials.footprint.map(([x, y, z]) => ({
      x,
      y,
      z_bottom: z,
      z_top: z,
    }));
    Object.assign(surface, structuredClone(before[side]));
    const volumeId = `${surface.id}-ridge-receiver`;
    assert.ok(!gameplay.volumes?.some((volume) => volume.id === volumeId));
    (gameplay.volumes ??= []).push({
      id: volumeId,
      node: surface.node,
      shape: {
        points,
        projection_plane: surface.projectionMaterials.planePoints,
        opaque: false,
        solid: false,
        mouse: true,
        show_shadow_polygon: false,
        default_material: surface.projectionMaterials.defaultMaterial,
      },
    });
    surface.projectionVolume = volumeId;
    delete surface.projectionMaterials;
  }
}
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-golden-roof-ridge-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: id,
      descriptorSha256: ref.descriptor_sha256,
      modelSha256: ref.model_sha256,
      gameplay,
    },
  ]),
);
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-ridge-candidate-requires-mesh-and-traversal-review",
    receiverVolumes,
    before,
    after: surfaces,
    planes,
    ridge,
  }),
);
console.log(output);
