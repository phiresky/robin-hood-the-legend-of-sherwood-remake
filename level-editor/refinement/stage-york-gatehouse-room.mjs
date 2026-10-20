// Give the gatehouse its own entrance while retaining the tower's shared room link.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { connectInteriors } from "../shared/src/interior-connections.ts";

const scenePath = "library/scenes/york.rhlos-map.json";
let document = await readStoredMap(scenePath, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const tower = assets.get("york-precinct-east-tall-square-tower");
const gate = assets.get("york-precinct-east-roofed-gatehouse");
assert.ok(tower && gate);
const before = new Map([tower, gate].map((asset) => [asset.id, structuredClone(asset.gameplay)]));
const part = (asset, node) => {
  const matches = document.objects.filter((object) => object.node === `asset:${asset.id}:${node}`);
  assert.equal(matches.length, 1);
  const definition = asset.parts.find((candidate) => candidate.node === node);
  assert.ok(definition);
  return { ...matches[0], obstacle: definition.obstacle_local_game };
};
const translation = (asset, node) => {
  const matrix = partMatrix(document.camera, document, part(asset, node));
  for (let index = 0; index < 16; index++) {
    if ([12, 13, 14].includes(index)) continue;
    assert.ok(Math.abs(matrix[index] - (index % 5 === 0 ? 1 : 0)) < 1e-8);
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const room = tower.gameplay.interiors.find((entry) => entry.id === "interior-0");
assert.equal(room?.doors.length, 3);
assert.equal(gate.gameplay.interiors.length, 0);
const source = room.doors.find((door) => door.id === "interior-0/door-2");
assert.ok(source);
assert.equal(source.passageStructure, undefined);
assert.equal(source.outsideAnchor, undefined);
assert.equal(source.outsideReceiverSegment, undefined);
const node = "building-864";
assert.ok(gate.parts.some((entry) => entry.node === "building-865"));
const from = translation(tower, source.node),
  to = translation(gate, node);
const delta = from.map((value, axis) => value - to[axis]);
const door = structuredClone(source);
door.node = node;
for (const key of ["inside", "outside", "middle"])
  door[key] = source[key].map((value, axis) => value + delta[axis]);
door.polygon = source.polygon.map((point) => point.map((value, axis) => value + delta[axis]));
door.passageStructure = { volumes: ["building-864", "building-865"], blockers: [] };
for (const key of ["inside", "outside", "middle"])
  for (let axis = 0; axis < 3; axis++)
    assert.ok(Math.abs(door[key][axis] + to[axis] - source[key][axis] - from[axis]) < 1e-8);
gate.gameplay.interiors.push({ id: room.id, node, doors: [door] });
room.doors = room.doors.filter((entry) => entry !== source);
assert.deepEqual(room.doors, before.get(tower.id).interiors[0].doors.slice(0, 2));
const endpoint = (asset, node) => ({
  placement: part(asset, node).group ?? part(asset, node).id,
  asset: asset.id,
  interior: room.id,
});
document = connectInteriors(document, endpoint(tower, room.node), endpoint(gate, node));
const edits = [tower, gate].map((asset) => {
  validateAssetGameplay(asset.gameplay, asset);
  const unchanged = structuredClone(asset.gameplay);
  unchanged.interiors = before.get(asset.id).interiors;
  assert.deepEqual(unchanged, before.get(asset.id));
  const reference = document.assetSources.find((entry) => entry.id === asset.id);
  return {
    asset: asset.id,
    descriptorSha256: reference.descriptor_sha256,
    modelSha256: reference.model_sha256,
    gameplay: asset.gameplay,
  };
});
const output = await fs.mkdtemp("work/needs-review/map-compile/york-gatehouse-room-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-gatehouse-ownership-candidate-not-traversal-proof",
    scenePath,
    sceneSha256: createHash("sha256")
      .update(await fs.readFile(scenePath))
      .digest("hex"),
    delta,
    worldOutside: source.outside.map((value, axis) => value + from[axis]),
    interiorConnections: document.interiorConnections,
  }),
);
const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
const length = Math.hypot(...direction);
assert.ok(length > 0);
await fs.writeFile(
  `${output}/gatehouse.json`,
  JSON.stringify({
    map: "york",
    assets: [gate.id],
    size: [3200, 3200],
    copyXs: [800.25, 2400.25],
    assemblyY: 1800.75,
    terrainHeight: source.outside[2] + from[2],
    tracePassages: true,
    entrances: [
      {
        asset: gate.id,
        door: door.id,
        sourceLocal: direction.map((value, axis) => door.outside[axis] + (value * 24) / length),
      },
    ],
  }),
);
for (const [index, remaining] of room.doors.entries()) {
  const direction = remaining.outside
    .slice(0, 2)
    .map((value, axis) => value - remaining.middle[axis]);
  const length = Math.hypot(...direction);
  assert.ok(length > 0);
  await fs.writeFile(
    `${output}/tower-${index}.json`,
    JSON.stringify({
      map: "york",
      assets: [tower.id],
      size: [3200, 3200],
      copyXs: [800.25, 2400.25],
      assemblyY: 1800.75,
      terrainHeight: remaining.outside[2] + translation(tower, remaining.node)[2],
      tracePassages: true,
      entrances: [
        {
          asset: tower.id,
          door: remaining.id,
          sourceLocal: direction.map(
            (value, axis) => remaining.outside[axis] + (value * 24) / length,
          ),
        },
      ],
    }),
  );
}
console.log(output);
