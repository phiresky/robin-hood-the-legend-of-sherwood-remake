// Stage the raised town terrain as a reusable floor with its existing materials.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane } from "../shared/src/gameplay-plane.ts";

const asset = "york-west-town-raised-terrain";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const source = scene.assetSources.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(hash(bytes), source.descriptor_sha256);
assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const receiver = gameplay.projectionReceivers.find((entry) => entry.id === "building-086-receiver");
assert.equal(receiver.navigationHeight, 0);
const part = descriptor.parts.find((entry) => entry.node === receiver.volume);
const points = part.obstacle_local_game.points;
assert.ok(part.obstacle_local_game.solid);
assert.equal(gameplay.surfaces.length, 0);
assert.equal(gameplay.materials.length, part.obstacle_local_game.material_indices.length);
assert.ok(
  gameplay.materials.every(
    (material) =>
      material.node === part.node &&
      material.obstacles.length === 1 &&
      material.obstacles[0] === part.node,
  ),
);
heightPlane(points.map((point) => [point.x, point.y, point.z_top]));
gameplay.surfaces.push({
  id: "building-086-walk-0",
  node: part.node,
  navigationRegion: "raised-town-floor",
  projectionMaterials: {
    defaultMaterial: part.obstacle_local_game.default_material,
    regions: gameplay.materials.map((material) => material.id),
  },
  polygon: points.map((point) => [point.x, point.y]),
  height: points.map((point) => point.z_top),
  holes: [],
  preserveMovementPrecision: true,
});
gameplay.projectionReceivers = gameplay.projectionReceivers.filter((entry) => entry !== receiver);
gameplay.draft.issues.push(
  "Unpublished physical terrain floor candidate; verify assembled routing, moved approaches and material coverage.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-raised-terrain-");
await fs.writeFile(
  `${output}/edits.json`,
  JSON.stringify([
    {
      asset,
      descriptorSha256: source.descriptor_sha256,
      modelSha256: source.model_sha256,
      gameplay,
    },
  ]),
);
console.log(output);
