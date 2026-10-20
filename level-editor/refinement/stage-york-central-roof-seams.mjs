// Review bounded ridge corrections before publishing reusable roof metadata.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const edits = [];
const changes = [];
for (const [id, firstId, secondId, firstIndices, secondIndices, third] of [
  [
    "york-central-south-red-timber-house",
    "building-323-walk-0",
    "building-322-walk-0",
    [1, 2],
    [2, 1],
    0,
  ],
  [
    "york-central-south-rear-timber-house",
    "building-331-walk-0",
    "building-330-walk-0",
    [1, 2],
    [0, 3],
    1,
  ],
]) {
  const ref = scene.assetSources.find((source) => source.id === id);
  assert.ok(ref);
  const bytes = await fs.readFile(`library/${ref.descriptor}`);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), ref.descriptor_sha256);
  const asset = JSON.parse(bytes);
  const gameplay = structuredClone(asset.gameplay);
  const first = gameplay.surfaces.find((surface) => surface.id === firstId);
  const second = gameplay.surfaces.find((surface) => surface.id === secondId);
  assert.ok(first && second && first.navigationRegion === second.navigationRegion);
  assert.ok(Array.isArray(first.height) && Array.isArray(second.height));
  const before = structuredClone(second);
  const ridge = firstIndices.map((index) => [...first.polygon[index], first.height[index]]);
  for (const [i, index] of secondIndices.entries()) {
    const old = [...second.polygon[index], second.height[index]];
    assert.ok(Math.hypot(...old.map((n, axis) => n - ridge[i][axis])) < 0.3);
    second.polygon[index] = ridge[i].slice(0, 2);
  }
  const plane = heightPlane([...ridge, [...second.polygon[third], second.height[third]]]);
  second.height = second.polygon.map((point) => planeHeight(plane, point));
  const vertices = second.polygon.map((point, index) => [...point, second.height[index]]);
  if (second.projectionMaterials) {
    second.projectionMaterials.planePoints = vertices.slice(0, 3);
    second.projectionMaterials.footprint = vertices;
    second.projectionMaterials.priorityHeight = Math.max(...second.height);
  }
  assert.ok(second.height.every((height, index) => Math.abs(height - before.height[index]) < 0.3));
  validateAssetGameplay(gameplay, asset);
  edits.push({
    asset: id,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay,
  });
  changes.push({ asset: id, before, after: second, ridge, plane });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/york-central-roof-seams-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-subpixel-ridge-candidate-requires-mesh-and-traversal-review",
    changes,
  }),
);
console.log(output);
