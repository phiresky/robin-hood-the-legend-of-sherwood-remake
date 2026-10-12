// Stage matching level spans on the lane's existing upper and lower receivers.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const scene = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", scene.assetSources, scene.sceneAssets);
const id = "york-west-town-raised-lane";
const asset = assets.get(id);
const source = scene.assetSources.find((entry) => entry.id === id);
assert.ok(asset && source);
const gameplay = structuredClone(asset.gameplay);
const pair = gameplay.jumpPairs.find((entry) => entry.id === "jump-pair-56");
const receiver = asset.parts.find((part) => part.node === "building-088");
const upper = gameplay.surfaces.find((surface) => surface.id === "building-613-walk-0");
assert.ok(pair && receiver?.obstacle_local_game && upper && Array.isArray(upper.height));
const before = structuredClone(pair);
const plane = heightPlane(
  receiver.obstacle_local_game.points.slice(0, 3).map((point) => [point.x, point.y, point.z_top]),
);
const upperPlane = heightPlane(upper.polygon.map(([x, y], i) => [x, y, upper.height[i]]));
assert.ok(Math.abs(upperPlane[0]) + Math.abs(upperPlane[1]) < 1e-6);
const length = Math.hypot(plane[0], plane[1]);
assert.ok(length > 1e-6);
let direction = [-plane[1] / length, plane[0] / length];
const originalDirection = pair.edges[0].b.map((n, axis) => n - pair.edges[0].a[axis]);
if (direction[0] * originalDirection[0] + direction[1] * originalDirection[1] < 0)
  direction = direction.map((n) => -n);
for (const [side, edge] of pair.edges.entries()) {
  const center = edge.a.map((n, axis) => (n + edge.b[axis]) / 2);
  const height = planeHeight(side ? plane : upperPlane, center);
  const sign = side ? -1 : 1;
  // Keep the reviewed three-unit span inside the narrow upper walkway. Both
  // sides share its direction, so projected distance transfer survives rotation.
  edge.a = [center[0] - 1.5 * direction[0] * sign, center[1] - 1.5 * direction[1] * sign, height];
  edge.b = [center[0] + 1.5 * direction[0] * sign, center[1] + 1.5 * direction[1] * sign, height];
}
validateAssetGameplay(gameplay, asset);
const output = await fs.mkdtemp("work/needs-review/map-compile/york-lane-level-jump-");
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
    scope: "staged-level-jump-requires-mesh-placement-and-full-scene-validation",
    source,
    before,
    after: pair,
    receiverPlane: plane,
    span: 3,
  }),
);
console.log(output);
