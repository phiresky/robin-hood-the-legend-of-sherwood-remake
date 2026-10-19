// Compare a mixed-asset gallery with explicit room links and with those links removed.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { connectInteriors } from "../shared/src/interior-connections.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { gameToScene } from "../shared/src/scene.ts";

const [gallery, stage, roomId] = process.argv.slice(2);
assert.ok(gallery && stage && roomId, "Supply gallery, gameplay stage and shared room ID");
const base = JSON.parse(await fs.readFile(`${gallery}/scene.json`));
const assets = await pinnedDescriptors("library", base.assetSources, base.sceneAssets);
assert.equal(assets.size, 2);
for (const edit of JSON.parse(await fs.readFile(`${stage}/edits.json`))) {
  const ref = base.assetSources.find((reference) => reference.id === edit.asset);
  assert.equal(ref?.descriptor_sha256, edit.descriptorSha256);
  assert.equal(ref?.model_sha256, edit.modelSha256);
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const members = [...assets.values()].map((asset) => {
  const room = asset.gameplay.interiors.find((room) => room.id === roomId);
  assert.equal(room?.doors.length, 1, "This fixture links one entrance on each side");
  return base.groups.filter((group) => base.objects.some((part) =>
    part.group === group.id && part.node === `asset:${asset.id}:${room.node}`))
    .map((group) => ({ group, asset, room }));
});
assert.ok(members[0].length > 0);
assert.equal(members[0].length, members[1].length);
const endpoint = ({ group, asset, room }) => ({ placement: group.id, asset: asset.id, interior: room.id });
let linked = structuredClone(base);
for (let i = 0; i < members[0].length; i++)
  linked = connectInteriors(linked, endpoint(members[0][i]), endpoint(members[1][i]));
const output = await fs.mkdtemp("work/needs-review/map-compile/linked-building-gallery-");
console.log(output);
const results = [];
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({ complete: false }));
for (const connected of [false, true]) {
  const document = connected ? linked : base;
  const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
  const doors = compiled.descriptor.asset_geometry.buildings.flatMap(({ Building }) => Building.doors);
  const expected = [...assets.values()].reduce((n, asset) =>
    n + asset.gameplay.interiors.reduce((n, room) => n + room.doors.length, 0), 0) * members[0].length;
  assert.equal(doors.length, expected);
  const outside = ({ group, asset, room }) => {
    const source = room.doors[0];
    const object = document.objects.find((part) => part.group === group.id && part.node === `asset:${asset.id}:${source.node}`);
    const definition = asset.parts.find((part) => part.node === source.node);
    const matrix = partMatrix(document.camera, document, { ...object, obstacle: definition.obstacle_local_game });
    const [x, y, z] = sceneToGame(document.camera,
      applyAffineMatrix(matrix, gameToScene(document.camera, ...source.middle)));
    const matches = doors.filter((door) => door.point_mid[0] === Math.round(x) && door.point_mid[1] === Math.round(y - z));
    assert.equal(matches.length, 1);
    return matches[0].point_out;
  };
  const pairs = members[0].flatMap((member, i) => {
    const from = outside(member), to = outside(members[1][i]);
    return [{ from, to, connected }, { from: to, to: from, connected }];
  });
  const file = `${connected ? "linked" : "separate"}.level.json`;
  await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
  await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
  results.push({ file, building_pairs: pairs, warnings: compiled.warnings,
    building_approaches: doors.map((door) => ({ point_out: door.point_out, layer: door.layer_out, sector: door.sector_out })) });
}
await fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "mixed-asset-room-connections-awaiting-native-verification", complete: true, gallery, stage, results,
}));
