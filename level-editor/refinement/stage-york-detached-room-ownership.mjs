// Move complete single-entrance rooms from terrain into their structural assets.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame } from "../shared/src/geometry.ts";

const document = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const transfers = [
  {
    from: "york-cathedral-precinct-raised-terrain",
    room: "interior-0",
    to: "york-precinct-east-gate-boundary-wall",
    node: "building-867",
  },
  {
    from: "york-cathedral-precinct-raised-terrain",
    room: "interior-1",
    to: "york-cathedral-nave-and-aisles",
    node: "building-901",
  },
  {
    from: "york-west-town-raised-terrain",
    room: "interior-4",
    to: "york-west-bridge-street-east-gallery-house",
    node: "building-213--component-gallery",
  },
];
const changed = new Set(transfers.flatMap((entry) => [entry.from, entry.to]));
const before = new Map([...changed].map((id) => [id, structuredClone(assets.get(id).gameplay)]));
const translation = (asset, node) => {
  const matches = document.objects.filter((object) => object.node === `asset:${asset.id}:${node}`);
  assert.equal(matches.length, 1, "Ownership review requires a single saved placement");
  const definition = asset.parts.find((part) => part.node === node);
  assert.ok(definition);
  const matrix = partMatrix(document.camera, document, {
    ...matches[0],
    obstacle: definition.obstacle_local_game,
  });
  for (let index = 0; index < 16; index++) {
    if ([12, 13, 14].includes(index)) continue;
    assert.ok(Math.abs(matrix[index] - (index % 5 === 0 ? 1 : 0)) < 1e-8);
  }
  return sceneToGame(document.camera, matrix.slice(12, 15));
};
const evidence = [],
  fixtures = [];
for (const transfer of transfers) {
  const source = assets.get(transfer.from),
    target = assets.get(transfer.to);
  const original = source.gameplay.interiors.find((room) => room.id === transfer.room);
  assert.equal(original?.doors.length, 1);
  assert.ok(!target.gameplay.interiors.some((room) => room.id === original.id));
  assert.ok(
    !(document.interiorConnections ?? []).some((link) =>
      [link.from, link.to].some(
        (endpoint) => endpoint.asset === source.id && endpoint.interior === original.id,
      ),
    ),
    "Existing room links require a separate reviewed migration",
  );
  const room = structuredClone(original),
    door = room.doors[0],
    oldDoor = original.doors[0];
  assert.equal(door.node, original.node);
  assert.equal(door.type, 1);
  assert.equal(door.passageStructure, undefined);
  assert.equal(door.outsideAnchor, undefined);
  assert.equal(door.outsideReceiverSegment, undefined);
  const from = translation(source, original.node),
    to = translation(target, transfer.node);
  const delta = from.map((value, axis) => value - to[axis]);
  room.node = door.node = transfer.node;
  for (const key of ["inside", "outside", "middle"])
    door[key] = oldDoor[key].map((value, axis) => value + delta[axis]);
  door.polygon = oldDoor.polygon.map((point) => point.map((value, axis) => value + delta[axis]));
  door.passageStructure = { volumes: [transfer.node], blockers: [] };
  for (const key of ["inside", "outside", "middle"])
    for (let axis = 0; axis < 3; axis++)
      assert.ok(Math.abs(door[key][axis] + to[axis] - oldDoor[key][axis] - from[axis]) < 1e-8);
  target.gameplay.interiors.push(room);
  source.gameplay.interiors = source.gameplay.interiors.filter((entry) => entry !== original);
  const direction = door.outside.slice(0, 2).map((value, axis) => value - door.middle[axis]);
  const length = Math.hypot(...direction);
  assert.ok(length > 0);
  fixtures.push({
    map: "york",
    assets: [target.id],
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
  });
  evidence.push({
    ...transfer,
    delta,
    worldOutside: oldDoor.outside.map((value, axis) => value + from[axis]),
    locked: door.locked,
  });
}
const edits = [...changed].map((id) => {
  const asset = assets.get(id);
  validateAssetGameplay(asset.gameplay, asset);
  const unchanged = structuredClone(asset.gameplay);
  unchanged.interiors = before.get(id).interiors;
  assert.deepEqual(unchanged, before.get(id));
  const reference = document.assetSources.find((entry) => entry.id === id);
  return {
    asset: id,
    descriptorSha256: reference.descriptor_sha256,
    modelSha256: reference.model_sha256,
    gameplay: asset.gameplay,
  };
});
const output = await fs.mkdtemp("work/needs-review/map-compile/york-detached-room-ownership-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({
    scope: "unpublished-room-ownership-candidate-not-traversal-proof",
    transfers: evidence,
  }),
);
for (const fixture of fixtures)
  await fs.writeFile(`${output}/${fixture.assets[0]}.json`, JSON.stringify(fixture));
for (const [name, source, size, copyXs, assemblyY] of [
  ["precinct", "york-cathedral-precinct-raised-terrain", [5000, 4000], [1250.25, 3750.25], 1800.75],
  ["gallery", "york-west-town-raised-terrain", [8400, 6200], [2200.25, 6200.25], 3000.75],
]) {
  const members = transfers.filter((transfer) => transfer.from === source);
  const unlocked = members.filter(
    (transfer) => !evidence.find((row) => row.to === transfer.to).locked,
  );
  const entrances = unlocked.flatMap(
    (transfer) => fixtures.find((fixture) => fixture.assets[0] === transfer.to).entrances,
  );
  await fs.writeFile(
    `${output}/${name}-assembly.json`,
    JSON.stringify({
      map: "york",
      assets: [source, ...members.map((transfer) => transfer.to)],
      size,
      copyXs,
      assemblyY,
      terrainHeight: 0,
      tracePassages: true,
      entrances,
    }),
  );
}
console.log(output);
