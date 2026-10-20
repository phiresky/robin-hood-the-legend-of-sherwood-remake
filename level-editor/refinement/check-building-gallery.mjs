// Assemble independently verified building placements into one browser/native fixture.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage, ...directories] = process.argv.slice(2);
assert.ok(stage && directories.length, "Supply a gameplay stage and placement audit directories");
const scenes = await Promise.all(
  directories.map(async (directory) =>
    JSON.parse(await fs.readFile(`${directory}/37-0.level.json.scene.json`)),
  ),
);
const document = structuredClone(scenes[0]);
const terrainHeights = scenes.map((scene) => {
  const height = scene.terrain.vertices[0].position[2];
  assert.ok(scene.terrain.vertices.every((vertex) => vertex.position[2] === height));
  return height;
});
const [width, height] = scenes[0].size;
assert.ok(height > 0 && height % 400 === 0);
const rowStride = height + 400;
document.size = [width, rowStride * scenes.length];
document.terrain = createTerrainGrid([0, 0, ...document.size], 400, 0);
// Preserve each input's flat support. A separate 400-unit
// gap absorbs height changes without sloping the last cells under its buildings.
for (const vertex of document.terrain.vertices) {
  const row = Math.min(scenes.length - 1, Math.floor(vertex.position[1] / rowStride));
  vertex.position[2] = terrainHeights[row];
}
document.groups = [];
document.objects = [];
document.assetSources = [];
document.sceneAssets = [];
let expectedDoors = 0;
const approachProbes = [];
for (const [index, scene] of scenes.entries()) {
  assert.deepEqual(scene.camera, document.camera);
  assert.deepEqual(scene.size, [width, height]);
  const report = JSON.parse(await fs.readFile(`${directories[index]}/diagnostics.json`));
  assert.equal(report.complete, true, "Finish placement checks before combining fixtures");
  const compiledSource = JSON.parse(await fs.readFile(`${directories[index]}/37-0.level.json`));
  expectedDoors += compiledSource.asset_geometry.buildings.reduce(
    (count, row) => count + row.Building.doors.length,
    0,
  );
  for (const probe of report.results.find((row) => row.file === "37-0.level.json")
    .building_approaches) {
    assert.equal(
      probe.source_layer,
      probe.layer,
      "Gallery probes must approach on their doorway floor",
    );
    assert.equal(probe.source_world?.length, 3);
    approachProbes.push({
      ...probe,
      point_out: [probe.point_out[0], probe.point_out[1] + index * rowStride],
      source_world: [
        probe.source_world[0],
        probe.source_world[1] + index * rowStride,
        probe.source_world[2],
      ],
    });
  }
  const prefix = `gallery-${index}/`;
  for (const group of scene.groups) {
    group.id = prefix + group.id;
    group.transform.dy += index * rowStride;
  }
  for (const object of scene.objects) {
    object.id = prefix + object.id;
    if (object.group) object.group = prefix + object.group;
  }
  document.groups.push(...scene.groups);
  document.objects.push(...scene.objects);
  document.assetSources.push(...scene.assetSources);
  document.sceneAssets.push(...scene.sceneAssets);
}
assert.equal(
  new Set(document.assetSources.map((source) => source.id)).size,
  document.assetSources.length,
);
const assets = await pinnedDescriptors("library", document.assetSources, document.sceneAssets);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
for (const edit of edits) {
  assert.equal(
    document.assetSources.find((source) => source.id === edit.asset)?.descriptor_sha256,
    edit.descriptorSha256,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
const doors = compiled.descriptor.asset_geometry.buildings.flatMap(
  ({ Building }) => Building.doors,
);
assert.equal(doors.length, expectedDoors);
assert.ok(
  doors.every((door) => !door.passage_states?.some((state) => !state.allowed_states.length)),
);
const output = await fs.mkdtemp("work/needs-review/map-compile/building-gallery-");
const file = "buildings.level.json";
await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "copied-rotated-buildings-on-authored-terrain-not-full-map-parity",
    stage,
    results: [
      {
        file,
        warnings: compiled.warnings,
        building_approaches: approachProbes.map((probe) => {
          const matches = doors.filter((door) =>
            door.point_out.every((value, axis) => value === probe.point_out[axis]),
          );
          assert.equal(matches.length, 1, "Translated approach must identify one emitted entrance");
          const door = matches[0];
          return {
            point_out: door.point_out,
            layer: door.layer_out,
            sector: door.sector_out,
            source_layer: door.layer_out,
            source_world: probe.source_world,
          };
        }),
      },
    ],
  }),
);
console.log(output);
