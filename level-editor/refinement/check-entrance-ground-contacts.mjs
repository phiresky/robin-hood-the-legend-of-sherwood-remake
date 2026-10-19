// Start full-character click tests beyond an authored exterior landing socket.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame, applyAffineMatrix } from "../shared/src/geometry.ts";
import { orientNavigationJoin } from "../shared/src/assemble-navigation-joins.ts";

const [placements, stage] = process.argv.slice(2);
assert.ok(placements && stage, "Supply a completed placement audit and socket stage");
const input = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`));
assert.equal(input.complete, true);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const review = JSON.parse(await fs.readFile(`${stage}/review.json`));
const change = review.changes.find((entry) => entry.asset === input.asset);
assert.ok(change, "Socket stage must identify the audited landing");
const output = await fs.mkdtemp("work/needs-review/map-compile/entrance-ground-contacts-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "copied-rotated-raised-entrance-ground-contacts-awaiting-native-click-test",
      complete,
      placements,
      stage,
      results,
    }),
  );
await save(false);
for (const row of input.results) {
  const document = JSON.parse(await fs.readFile(`${placements}/${row.file}.scene.json`));
  const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
  assert.equal(assets.size, 1);
  const asset = assets.get(input.asset);
  const edit = edits.find((entry) => entry.asset === input.asset);
  const ref = document.assetSources.find((entry) => entry.id === input.asset);
  assert.equal(ref.descriptor_sha256, edit.descriptorSha256);
  assert.equal(ref.model_sha256, edit.modelSha256);
  asset.gameplay = edit.gameplay;
  const surface = asset.gameplay.surfaces.find((entry) => entry.id === change.after.id);
  const terrainHeight = document.terrain.vertices[0].position[2];
  assert.ok(document.terrain.vertices.every((vertex) => vertex.position[2] === terrainHeight));
  const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
  const doors = compiled.descriptor.asset_geometry.buildings.flatMap(
    ({ Building }) => Building.doors,
  );
  assert.equal(doors.length, row.expectedDoors);
  const probes = [];
  for (const group of document.groups) {
    const transform = (node, p) => {
      const part = document.objects.find(
        (entry) => entry.group === group.id && entry.node === `asset:${asset.id}:${node}`,
      );
      assert.ok(part);
      const definition = asset.parts.find((entry) => entry.node === node);
      const matrix = partMatrix(document.camera, document, {
        ...part,
        obstacle: definition.obstacle_local_game,
      });
      return sceneToGame(
        document.camera,
        applyAffineMatrix(matrix, gameToScene(document.camera, ...p)),
      );
    };
    const polygon = surface.polygon.map((p, i) =>
      transform(surface.node, [...p, surface.height[i]]),
    );
    const [a, b] = orientNavigationJoin(
      polygon,
      surface.navigationJoins[0].map((p) => transform(surface.node, p)),
    );
    assert.ok([a, b].every((p) => Math.abs(p[2] - terrainHeight) < 1e-4));
    const dx = b[0] - a[0],
      dy = b[1] - a[1],
      length = Math.hypot(dx, dy);
    const source = [
      (a[0] + b[0]) / 2 + (dy / length) * 32,
      (a[1] + b[1]) / 2 - (dx / length) * 32,
      terrainHeight,
    ];
    const entrances = asset.gameplay.interiors
      .flatMap((room) => room.doors)
      .filter((door) => Math.abs(transform(door.node, door.outside)[2] - terrainHeight) < 1e-4);
    assert.equal(entrances.length, 1, "Select exactly one ground-height entrance per placement");
    const [x, y, z] = transform(entrances[0].node, entrances[0].middle);
    const matches = doors.filter(
      (door) => door.point_mid[0] === Math.round(x) && door.point_mid[1] === Math.round(y - z),
    );
    assert.equal(matches.length, 1);
    const door = matches[0];
    probes.push({
      point_out: door.point_out,
      layer: door.layer_out,
      sector: door.sector_out,
      source_layer: door.layer_out,
      source_world: source,
    });
  }
  await fs.writeFile(`${output}/${row.file}`, JSON.stringify(compiled.descriptor));
  results.push({
    file: row.file,
    rotation: row.rotation,
    height: row.height,
    building_approaches: probes,
    warnings: compiled.warnings,
  });
  await save(false);
}
await save(true);
