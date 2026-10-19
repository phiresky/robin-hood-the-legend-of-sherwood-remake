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
const rowStride = 1600;
document.size = [1600, rowStride * scenes.length];
document.terrain = createTerrainGrid([0, 0, ...document.size], 400, 0);
// Preserve all 1200 units of each input's flat support. A separate 400-unit
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
for (const [index, scene] of scenes.entries()) {
  assert.deepEqual(scene.camera, document.camera);
  assert.deepEqual(scene.size, [1600, 1200]);
  const report = JSON.parse(await fs.readFile(`${directories[index]}/diagnostics.json`));
  assert.equal(report.complete, true, "Finish placement checks before combining fixtures");
  expectedDoors += report.results.find((row) => row.file === "37-0.level.json").building_approaches
    .length;
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
        building_approaches: doors.map((door) => {
          const heights = terrainHeights.filter((height, row) => {
            const worldY = door.point_out[1] + height;
            return worldY >= row * rowStride && worldY < row * rowStride + 1200;
          });
          assert.equal(heights.length, 1, "Entrance must identify one gallery row");
          const terrainHeight = heights[0];
          const delta = door.point_out.map((value, axis) => value - door.point_mid[axis]);
          const length = Math.hypot(...delta);
          assert.ok(length > 0, "Entrance needs a distinct outdoor approach");
          return {
            point_out: door.point_out,
            layer: door.layer_out,
            sector: door.sector_out,
            source_layer: 0,
            source_world: [
              door.point_out[0] + (delta[0] * 32) / length,
              door.point_out[1] + (delta[1] * 32) / length + terrainHeight,
              terrainHeight,
            ],
          };
        }),
      },
    ],
  }),
);
console.log(output);
