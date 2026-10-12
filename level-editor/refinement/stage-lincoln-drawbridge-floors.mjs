// Unpublished deck-floor experiment; locked/open traversal must be reviewed separately.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const root = "library/3d-assets/lincoln/lincoln-south-gate-drawbridge";
const bytes = await fs.readFile(`${root}/asset.json`);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
assert.equal(gameplay.surfaces.length, 0, "Review existing surfaces before restaging");
const nodes = ["building-058", "building-059"];
for (const node of nodes) {
  const part = descriptor.parts.find((part) => part.node === node);
  const shape = part?.obstacle_local_game;
  assert.ok(shape?.solid && shape.points.length === 4);
  assert.ok(shape.points.every((point) => Math.abs(point.z_top - 220.001) < 1e-6));
  assert.ok(!shape.material_indices.length);
  const vertices = shape.points.map((point) => [point.x, point.y, point.z_top]);
  const id = `${node}-deck-floor`;
  gameplay.surfaces.push({
    id,
    node,
    navigationRegion: id,
    polygon: vertices.map(([x, y]) => [x, y]),
    height: vertices.map(([, , z]) => z),
    preserveMovementPrecision: true,
    navigationJoins: vertices.map((point, index) => [
      point,
      vertices[(index + 1) % vertices.length],
    ]),
    projectionMaterials: { defaultMaterial: shape.default_material, regions: [] },
  });
}
assert.equal(gameplay.projectionReceivers.length, 2);
assert.ok(gameplay.projectionReceivers.every((receiver) => nodes.includes(receiver.node)));
gameplay.projectionReceivers = [];
gameplay.draft.issues.push(
  "Unpublished drawbridge deck candidate: verify mesh support, locked/open traversal, bank contacts and rotated/elevated placement before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const sha = (data) => createHash("sha256").update(data).digest("hex");
const output = await fs.mkdtemp("work/needs-review/map-compile/lincoln-drawbridge-floors-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset: descriptor.id,
      descriptorSha256: sha(bytes),
      modelSha256: sha(await fs.readFile(`${root}/model.glb`)),
      gameplay,
    },
  ]),
);
console.log(output);
