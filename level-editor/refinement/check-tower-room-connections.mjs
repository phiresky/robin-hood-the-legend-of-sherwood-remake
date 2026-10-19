// Exercise independently moved tower copies with and without an editor room link.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { connectInteriors } from "../shared/src/interior-connections.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";

const [placements, stage] = process.argv.slice(2);
assert.ok(placements && stage, "Supply a completed tower placement audit and gameplay stage");
const input = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`));
assert.equal(input.complete, true);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const output = await fs.mkdtemp("work/needs-review/map-compile/tower-room-connections-");
console.log(output);
const results = [];
const save = (complete) => fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "moved-tower-room-connections-awaiting-native-verification", complete, placements, stage, results,
}));
await save(false);
for (const row of input.results) {
  const base = JSON.parse(await fs.readFile(`${placements}/${row.file}.scene.json`));
  assert.equal(base.groups.length, 2);
  base.groups[1].transform.dy += 80;
  base.groups[1].transform.rot_deg += 23;
  const assets = await pinnedDescriptors("library", base.assetSources, base.sceneAssets);
  for (const ref of base.assetSources) {
    const edit = edits.find((entry) => entry.asset === ref.id);
    assert.ok(edit);
    assert.equal(ref.descriptor_sha256, edit.descriptorSha256);
    assert.equal(ref.model_sha256, edit.modelSha256);
    assets.get(ref.id).gameplay = edit.gameplay;
  }
  assert.equal(assets.size, 1);
  const asset = [...assets.values()][0];
  const endpoint = (group) => ({ placement: group.id, asset: asset.id, interior: "interior-6" });
  for (const connected of [false, true]) {
    const document = connected
      ? connectInteriors(base, endpoint(base.groups[0]), endpoint(base.groups[1])) : structuredClone(base);
    const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
    const doors = compiled.descriptor.asset_geometry.buildings.flatMap(({ Building }) => Building.doors);
    assert.equal(doors.length, 6, "All independently placed entrances must bind");
    const find = (group, roomId, doorIndex) => {
      const room = asset.gameplay.interiors.find((room) => room.id === roomId);
      const door = room.doors[doorIndex];
      const part = document.objects.find((part) => part.group === group.id && part.node === `asset:${asset.id}:${door.node}`);
      assert.ok(part);
      const definition = asset.parts.find((candidate) => candidate.node === door.node);
      const matrix = partMatrix(document.camera, document, { ...part, obstacle: definition.obstacle_local_game });
      const [x, y, z] = sceneToGame(document.camera,
        applyAffineMatrix(matrix, gameToScene(document.camera, ...door.middle)));
      const matches = doors.filter((door) => door.point_mid[0] === Math.round(x) && door.point_mid[1] === Math.round(y - z));
      assert.equal(matches.length, 1);
      return matches[0].point_out;
    };
    const pairs = [];
    const add = (from, to, connected) => pairs.push({ from, to, connected }, { from: to, to: from, connected });
    for (const group of document.groups)
      add(find(group, "interior-3", 0), find(group, "interior-3", 1), true);
    add(find(document.groups[0], "interior-6", 0), find(document.groups[1], "interior-6", 0), connected);
    const file = `${connected ? "linked" : "separate"}-${row.file}`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, building_pairs: pairs, warnings: compiled.warnings });
    await save(false);
  }
}
await save(true);
