// Restore the sloped lane's own walking definition from its retained receiver.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

const asset = "york-west-town-raised-lane";
const scene = JSON.parse(await fs.readFile("library/scenes/york.rhlos-map.json"));
const source = scene.assetSources.find((entry) => entry.id === asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/${source.descriptor}`);
assert.equal(hash(bytes), source.descriptor_sha256);
assert.equal(hash(await fs.readFile(`library/${source.model}`)), source.model_sha256);
const descriptor = JSON.parse(bytes);
const gameplay = structuredClone(descriptor.gameplay);
const part = descriptor.parts.find((part) => part.node === "building-088");
assert.ok(part?.obstacle_local_game?.projection_area);
assert.ok(!gameplay.surfaces.some((surface) => surface.node === part.node));
const points = part.obstacle_local_game.points;
const plane = heightPlane(
  points.map((point) => [point.x, point.y, point.z_top]),
  false,
);
const heights = points.map((point) => planeHeight(plane, [point.x, point.y]));
assert.ok(heights.every((height, index) => Math.abs(height - points[index].z_top) < 0.001));
gameplay.surfaces.push({
  id: "building-088-walk-0",
  node: part.node,
  navigationRegion: "raised-lane-floor",
  polygon: points.map((point) => [point.x, point.y]),
  height: heights,
  holes: [],
  preserveMovementPrecision: true,
});
assert.equal(gameplay.projectionReceivers.filter((entry) => entry.node === part.node).length, 1);
gameplay.projectionReceivers = gameplay.projectionReceivers.filter(
  (entry) => entry.node !== part.node,
);
gameplay.draft.issues.push(
  "Staged sloped lane receiver restoration; verify moved entrances, neighboring floor joins and rendered contact before publication.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-raised-lane-");
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
