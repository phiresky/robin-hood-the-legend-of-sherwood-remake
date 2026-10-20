// Give the bastion its entrance and migrate the map-owned castle room links.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";
import { validateInteriorConnections } from "../shared/src/interior-connections.ts";

const scenePath = "library/scenes/york.rhlos-map.json";
const document = await readStoredMap(scenePath, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const source = assets.get("york-castle-courtyard-raised-terrain");
const target = assets.get("york-east-water-gate-north-bastion");
assert.ok(source && target);
const before = new Map(
  [source, target].map((asset) => [asset.id, structuredClone(asset.gameplay)]),
);
const original = source.gameplay.interiors.find((room) => room.id === "interior-2");
assert.equal(original?.doors.length, 1);
assert.equal(target.gameplay.interiors.length, 0);
const node = "building-266";
const placement = (asset, node) => {
  const objects = document.objects.filter((part) => part.node === `asset:${asset.id}:${node}`);
  assert.equal(objects.length, 1);
  return objects[0];
};
const translation = (asset, node) => {
  const part = placement(asset, node);
  const shape = asset.parts.find((part) => part.node === node).obstacle_local_game;
  const matrix = partMatrix(document.camera, document, { ...part, obstacle: shape });
  for (let i = 0; i < 16; i++) {
    if ([12, 13, 14].includes(i)) continue;
    assert.ok(Math.abs(matrix[i] - (i % 5 === 0 ? 1 : 0)) < 1e-8);
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const from = translation(source, original.node),
  to = translation(target, node);
const delta = from.map((value, axis) => value - to[axis]);
const room = structuredClone(original),
  door = room.doors[0],
  oldDoor = original.doors[0];
assert.equal(door.node, original.node);
assert.equal(door.passageStructure, undefined);
assert.equal(door.outsideAnchor, undefined);
assert.equal(door.outsideReceiverSegment, undefined);
room.node = door.node = node;
for (const key of ["inside", "outside", "middle"])
  door[key] = oldDoor[key].map((value, axis) => value + delta[axis]);
door.polygon = oldDoor.polygon.map((point) => point.map((value, axis) => value + delta[axis]));
const bodyNode = "building-282--component-bastion";
door.passageStructure = { volumes: [node, bodyNode], blockers: [] };
const bodyTranslation = translation(target, bodyNode);
const footprint = target.parts
  .find((part) => part.node === bodyNode)
  .obstacle_local_game.points.map((point) => [
    point.x + bodyTranslation[0] - to[0],
    point.y + bodyTranslation[1] - to[1],
  ]);
assert.ok(
  pointInGameplayPolygon(door.inside.slice(0, 2), footprint, true),
  "The room's inside anchor must belong to the receiving bastion body",
);
for (const key of ["inside", "outside", "middle"])
  for (let axis = 0; axis < 3; axis++)
    assert.ok(Math.abs(door[key][axis] + to[axis] - oldDoor[key][axis] - from[axis]) < 1e-8);
source.gameplay.interiors = source.gameplay.interiors.filter((room) => room !== original);
target.gameplay.interiors.push(room);
const sourcePart = placement(source, original.node),
  targetPart = placement(target, node);
const sourceEndpoint = {
  placement: sourcePart.group ?? sourcePart.id,
  asset: source.id,
  interior: original.id,
};
const targetEndpoint = {
  placement: targetPart.group ?? targetPart.id,
  asset: target.id,
  interior: room.id,
};
const beforeLinks = structuredClone(document.interiorConnections);
const migrated = [];
for (const link of document.interiorConnections)
  for (const side of ["from", "to"]) {
    if (link[side].asset !== source.id || link[side].interior !== original.id) continue;
    assert.deepEqual(link[side], sourceEndpoint);
    const other = link[side === "from" ? "to" : "from"];
    assert.ok(["york-castle-great-hall", "york-castle-main-keep"].includes(other.asset));
    migrated.push({ id: link.id, side, before: link[side], after: targetEndpoint });
    link[side] = { ...targetEndpoint };
  }
assert.equal(migrated.length, 2);
validateInteriorConnections(document.interiorConnections, document.objects);
const edits = [source, target].map((asset) => {
  validateAssetGameplay(asset.gameplay, asset);
  const unchanged = structuredClone(asset.gameplay);
  unchanged.interiors = before.get(asset.id).interiors;
  assert.deepEqual(unchanged, before.get(asset.id));
  const ref = document.assetSources.find((ref) => ref.id === asset.id);
  return {
    asset: asset.id,
    descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256,
    gameplay: asset.gameplay,
  };
});
const output = await fs.mkdtemp("work/needs-review/map-compile/york-water-gate-room-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-linked-room-ownership-candidate-not-traversal-proof",
    scenePath,
    sceneSha256: createHash("sha256")
      .update(await fs.readFile(scenePath))
      .digest("hex"),
    beforeLinks,
    interiorConnections: document.interiorConnections,
    migrated,
    delta,
    worldOutside: oldDoor.outside.map((value, axis) => value + from[axis]),
  }),
);
const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
const length = Math.hypot(...direction);
assert.ok(length > 0);
for (const { name, neighbors } of [
  { name: "bastion", neighbors: [] },
  { name: "bastion-terrace", neighbors: ["york-southeast-riverside-raised-terrace"] },
])
  await fs.writeFile(
    `${output}/${name}.json`,
    JSON.stringify({
      map: "york",
      assets: [target.id, ...neighbors],
      size: [3200, 3200],
      copyXs: [800.25, 2400.25],
      assemblyY: 1800.75,
      terrainHeight: oldDoor.outside[2] + from[2],
      tracePassages: true,
      entrances: [
        {
          asset: target.id,
          door: door.id,
          sourceLocal: direction.map((value, axis) => door.outside[axis] + (value * 24) / length),
        },
      ],
    }),
  );
console.log(output);
