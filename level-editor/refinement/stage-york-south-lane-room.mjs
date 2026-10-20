// Review a ground entrance against its receiving house rather than a nearby roof.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const source = assets.get("york-south-wall-lane-square-stone-tower");
const target = assets.get("york-south-gate-lane-rear-timber-house");
assert.ok(source && target);
const before = new Map(
  [source, target].map((asset) => [asset.id, structuredClone(asset.gameplay)]),
);
assert.equal(source.gameplay.interiors.length, 1);
assert.equal(target.gameplay.interiors.length, 0);
const room = structuredClone(source.gameplay.interiors[0]);
assert.equal(room.doors.length, 1);
assert.ok(
  !document.interiorConnections.some((link) =>
    [link.from, link.to].some((end) => end.asset === source.id && end.interior === room.id),
  ),
);
const door = room.doors[0],
  original = structuredClone(door);
const translation = (asset, node) => {
  const parts = document.objects.filter((part) => part.node === `asset:${asset.id}:${node}`);
  assert.equal(parts.length, 1);
  const shape = asset.parts.find((part) => part.node === node).obstacle_local_game;
  const matrix = partMatrix(document.camera, document, { ...parts[0], obstacle: shape });
  for (let i = 0; i < 16; i++) {
    if ([12, 13, 14].includes(i)) continue;
    assert.ok(Math.abs(matrix[i] - (i % 5 === 0 ? 1 : 0)) < 1e-8);
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const node = "building-421";
const from = translation(source, door.node),
  to = translation(target, node);
const blocker = target.gameplay.movementBlockers.find((blocker) => blocker.node === node);
assert.ok(blocker && blocker.height.every((height) => Math.abs(height - blocker.height[0]) < 1e-8));
const ground = blocker.height[0] + to[2];
assert.ok(Math.abs(ground) < 1e-8);
const project = (point) => [point[0] + from[0], point[1] + from[1] - point[2] - from[2]];
const rebase = (point) => {
  const [x, y] = project(point);
  return [x - to[0], y + ground - to[1], ground - to[2]];
};
for (const key of ["inside", "outside", "middle"]) door[key] = rebase(original[key]);
door.polygon = original.polygon.map((point) => rebase([...point, original.outside[2]]).slice(0, 2));
room.node = door.node = node;
const footprints = target.parts.map((part) => {
  const offset = translation(target, part.node);
  return part.obstacle_local_game.points.map((point) => [
    point.x + offset[0] - to[0],
    point.y + offset[1] - to[1],
  ]);
});
const contained = (point) =>
  footprints.some((footprint) => pointInGameplayPolygon(point.slice(0, 2), footprint, true));
assert.ok(contained(door.inside));
assert.ok(contained(door.middle));
assert.ok(!contained(door.outside));
assert.equal(target.gameplay.movementSolids, undefined);
target.gameplay.movementSolids = target.parts
  .filter((part) => part.obstacle_local_game?.solid && part.collision !== "none")
  .map((part) => part.node);
door.passageStructure = { volumes: target.gameplay.movementSolids, blockers: [] };
source.gameplay.interiors = [];
target.gameplay.interiors.push(room);
for (const key of ["inside", "outside", "middle"]) {
  const [x, y, z] = door[key];
  const expected = project(original[key]);
  assert.ok(Math.abs(x + to[0] - expected[0]) < 1e-8);
  assert.ok(Math.abs(y + to[1] - z - to[2] - expected[1]) < 1e-8);
}
const edits = [source, target].map((asset) => {
  validateAssetGameplay(asset.gameplay, asset);
  const unchanged = structuredClone(asset.gameplay);
  unchanged.interiors = before.get(asset.id).interiors;
  if (asset === target) delete unchanged.movementSolids;
  assert.deepEqual(unchanged, before.get(asset.id));
  const ref = document.assetSources.find((ref) => ref.id === asset.id);
  return {
    asset: asset.id,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay: asset.gameplay,
  };
});
const output = await fs.mkdtemp("work/needs-review/map-compile/york-south-lane-room-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-ground-room-ownership-candidate-not-parity-proof",
    previousWorldHeight: original.outside[2] + from[2],
    ground,
    projectedOutside: project(original.outside),
    source: source.id,
    target: target.id,
  }),
);
const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
const length = Math.hypot(...direction);
assert.ok(length > 0);
for (const neighbors of [[], ["york-south-town-raised-terrace"]])
  await fs.writeFile(
    `${output}/${neighbors.length ? "terrace" : "house"}.json`,
    JSON.stringify({
      map: "york",
      assets: [target.id, ...neighbors],
      size: [4000, 3600],
      copyXs: [1000.25, 3000.25],
      assemblyY: 2200.75,
      terrainHeight: ground,
      tracePassages: true,
      entrances: [
        {
          asset: target.id,
          door: door.id,
          sourceLocal: direction.map((value, axis) => door.outside[axis] + (value * 64) / length),
        },
      ],
    }),
  );
console.log(output);
