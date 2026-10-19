// Stage an ownership correction for placement review; never modify the library.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { createHash } from "node:crypto";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { connectInteriors } from "../shared/src/interior-connections.ts";

const scenePath = "library/scenes/derby.rhlos-map.json";
let document = await readStoredMap(scenePath, "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const hall = assets.get("derby-keep-main-hall");
const tower = assets.get("derby-keep-north-tower");
assert.ok(hall && tower);
const before = new Map([hall, tower].map((asset) => [asset.id, structuredClone(asset.gameplay)]));
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const part = (asset, node) => {
  const matches = document.objects.filter((object) => object.node === `asset:${asset.id}:${node}`);
  assert.equal(matches.length, 1, "Ownership review requires a single saved placement");
  const definition = asset.parts.find((candidate) => candidate.node === node);
  assert.ok(definition);
  return { ...matches[0], obstacle: definition.obstacle_local_game };
};
const translation = (asset, node) => {
  const matrix = partMatrix(document.camera, document, part(asset, node));
  for (let i = 0; i < 16; i++) {
    if ([12, 13, 14].includes(i)) continue;
    assert.ok(Math.abs(matrix[i] - (i % 5 === 0 ? 1 : 0)) < 1e-8,
      "Rebase requires translation-only authoring placements");
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const transfers = [
  { room: "interior-3", door: "interior-3/door-0", node: "building-164" },
  { room: "interior-3", door: "interior-3/door-1", node: "building-171" },
  { room: "interior-6", door: "interior-6/door-1", node: "building-165" },
];
const evidence = [];
for (const transfer of transfers) {
  const sourceRoom = hall.gameplay.interiors.find((room) => room.id === transfer.room);
  const source = sourceRoom?.doors.find((door) => door.id === transfer.door);
  assert.ok(source);
  assert.equal(source.passageStructure, undefined);
  assert.equal(source.outsideAnchor, undefined);
  assert.equal(source.outsideReceiverSegment, undefined);
  const from = translation(hall, source.node), to = translation(tower, transfer.node);
  const delta = from.map((value, axis) => value - to[axis]);
  const door = structuredClone(source);
  door.node = transfer.node;
  for (const key of ["inside", "outside", "middle"])
    door[key] = source[key].map((value, axis) => value + delta[axis]);
  door.polygon = source.polygon.map((point) => point.map((value, axis) => value + delta[axis]));
  door.passageStructure = { volumes: [transfer.node], blockers: [] };
  let destination = tower.gameplay.interiors.find((room) => room.id === transfer.room);
  if (!destination) {
    destination = { id: transfer.room, node: transfer.node, doors: [] };
    tower.gameplay.interiors.push(destination);
  }
  assert.deepEqual(translation(tower, destination.node), to,
    "Doors of one room must share their owning part transform");
  door.node = destination.node;
  assert.ok(!destination.doors.some((candidate) => candidate.id === door.id));
  destination.doors.push(door);
  sourceRoom.doors = sourceRoom.doors.filter((candidate) => candidate.id !== door.id);
  evidence.push({ ...transfer, delta, worldOutside: source.outside.map((v, i) => v + from[i]) });
}
hall.gameplay.interiors = hall.gameplay.interiors.filter((room) => room.doors.length);
const endpoint = (asset, node, interior) => ({
  placement: part(asset, node).group ?? part(asset, node).id, asset: asset.id, interior,
});
document = connectInteriors(document,
  endpoint(hall, "building-227", "interior-6"),
  endpoint(tower, "building-165", "interior-6"));
const edits = [];
for (const asset of [hall, tower]) {
  validateAssetGameplay(asset.gameplay, asset);
  const unchanged = structuredClone(asset.gameplay);
  unchanged.interiors = before.get(asset.id).interiors;
  assert.deepEqual(unchanged, before.get(asset.id), "Only interior ownership may change");
  const ref = document.assetSources.find((reference) => reference.id === asset.id);
  assert.ok(ref);
  edits.push({ asset: asset.id, descriptorSha256: ref.descriptor_sha256,
    modelSha256: ref.model_sha256, gameplay: asset.gameplay });
}
const output = await fs.mkdtemp("work/needs-review/map-compile/derby-tower-ownership-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(`${output}/review.json`, JSON.stringify({
  scope: "unpublished-ownership-candidate-not-traversal-proof", scenePath,
  sceneSha256: hash(await fs.readFile(scenePath)), transfers: evidence,
  interiorConnections: document.interiorConnections,
}));
console.log(output);
