// Rehome castle entrances into reusable buildings, retaining explicit room links.
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
const terrain = assets.get("york-castle-courtyard-raised-terrain");
assert.ok(terrain);
const transfers = [
  {
    asset: "york-castle-great-hall",
    door: "interior-2/door-0",
    node: "building-793",
    volumes: [
      "building-793",
      "building-769--component-hall",
      "building-769--component-hall-foundation",
    ],
  },
  {
    asset: "york-castle-main-keep",
    door: "interior-2/door-1",
    node: "building-790",
    volumes: ["building-790"],
  },
];
const changed = [terrain, ...transfers.map((transfer) => assets.get(transfer.asset))];
assert.ok(changed.every(Boolean));
const before = new Map(changed.map((asset) => [asset.id, structuredClone(asset.gameplay)]));
const part = (asset, node) => {
  const objects = document.objects.filter((object) => object.node === `asset:${asset.id}:${node}`);
  assert.equal(objects.length, 1, "Review requires one saved placement");
  const definition = asset.parts.find((candidate) => candidate.node === node);
  assert.ok(definition);
  return { ...objects[0], obstacle: definition.obstacle_local_game };
};
const translation = (asset, node) => {
  const matrix = partMatrix(document.camera, document, part(asset, node));
  for (let index = 0; index < 16; index++) {
    if ([12, 13, 14].includes(index)) continue;
    assert.ok(
      Math.abs(matrix[index] - (index % 5 === 0 ? 1 : 0)) < 1e-8,
      "Ownership review requires translation-only placements",
    );
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const endpoint = (asset, node) => ({
  placement: part(asset, node).group ?? part(asset, node).id,
  asset: asset.id,
  interior: "interior-2",
});
const sourceRoom = terrain.gameplay.interiors.find((room) => room.id === "interior-2");
assert.equal(sourceRoom?.doors.length, 3);
const evidence = [];
const fixtures = [];
for (const transfer of transfers) {
  const asset = assets.get(transfer.asset);
  const source = sourceRoom.doors.find((door) => door.id === transfer.door);
  assert.ok(source);
  assert.equal(source.passageStructure, undefined);
  assert.equal(source.outsideAnchor, undefined);
  assert.equal(source.outsideReceiverSegment, undefined);
  assert.ok(!asset.gameplay.interiors.some((room) => room.id === sourceRoom.id));
  for (const node of transfer.volumes) assert.ok(asset.parts.some((entry) => entry.node === node));
  const from = translation(terrain, source.node),
    to = translation(asset, transfer.node);
  const delta = from.map((value, axis) => value - to[axis]);
  const door = structuredClone(source);
  door.node = transfer.node;
  for (const key of ["inside", "outside", "middle"])
    door[key] = source[key].map((value, axis) => value + delta[axis]);
  door.polygon = source.polygon.map((point) => point.map((value, axis) => value + delta[axis]));
  door.passageStructure = { volumes: transfer.volumes, blockers: [] };
  asset.gameplay.interiors.push({ id: sourceRoom.id, node: transfer.node, doors: [door] });
  sourceRoom.doors = sourceRoom.doors.filter((entry) => entry.id !== door.id);
  document = connectInteriors(
    document,
    endpoint(terrain, sourceRoom.node),
    endpoint(asset, transfer.node),
  );
  for (const key of ["inside", "outside", "middle"])
    for (let axis = 0; axis < 3; axis++)
      assert.ok(Math.abs(door[key][axis] + to[axis] - source[key][axis] - from[axis]) < 1e-8);
  const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
  const length = Math.hypot(...direction);
  assert.ok(length > 0);
  fixtures.push({
    map: "york",
    assets: [asset.id],
    size: [3200, 3200],
    copyXs: [800.25, 2400.25],
    assemblyY: 1800.75,
    terrainHeight: source.outside[2] + from[2],
    tracePassages: true,
    entrances: [
      {
        asset: asset.id,
        door: door.id,
        sourceLocal: direction.map((value, axis) => door.outside[axis] + (value * 24) / length),
      },
    ],
  });
  evidence.push({
    ...transfer,
    delta,
    worldOutside: source.outside.map((value, axis) => value + from[axis]),
  });
}
assert.equal(
  sourceRoom.doors.length,
  1,
  "Retain the separate terrace entrance and room connection",
);
const edits = changed.map((asset) => {
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
const output = await fs.mkdtemp("work/needs-review/map-compile/york-castle-door-ownership-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-ownership-candidate-not-traversal-proof",
    scenePath,
    sceneSha256: createHash("sha256")
      .update(await fs.readFile(scenePath))
      .digest("hex"),
    transfers: evidence,
    interiorConnections: document.interiorConnections,
  }),
);
for (const fixture of fixtures)
  await fs.writeFile(`${output}/${fixture.assets[0]}.json`, JSON.stringify(fixture));
// Retain the near probe, which can project behind the rotated keep's upper floor.
// A separate exposed-ground probe checks normal click routing at every rotation.
const exposed = structuredClone(
  fixtures.find((fixture) => fixture.assets[0] === "york-castle-main-keep"),
);
const keepDoor = assets
  .get(exposed.assets[0])
  .gameplay.interiors.find((room) => room.id === "interior-2").doors[0];
const direction = keepDoor.outside.slice(0, 2).map((value, axis) => value - keepDoor.middle[axis]);
const length = Math.hypot(...direction);
exposed.entrances[0].sourceLocal = direction.map(
  (value, axis) => keepDoor.outside[axis] + (value * 124) / length,
);
await fs.writeFile(`${output}/keep-exposed-ground.json`, JSON.stringify(exposed));
console.log(output);
