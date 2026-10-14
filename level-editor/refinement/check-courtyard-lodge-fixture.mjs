// Assemble the independently verified lodges into one browser/native fixture.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [east, west, stage] = process.argv.slice(2);
assert.ok(east && west && stage, "Supply two placement audit directories and gameplay stage");
const document = JSON.parse(await fs.readFile(`${east}/37-0.level.json.scene.json`));
const other = JSON.parse(await fs.readFile(`${west}/37-0.level.json.scene.json`));
assert.deepEqual(document.camera, other.camera);
document.size = [1600, 2400];
document.terrain = createTerrainGrid([0, 0, ...document.size], 400, 90.00101);
for (const group of other.groups) {
  group.id = `west/${group.id}`;
  group.transform.dy += 1200;
}
for (const object of other.objects) {
  object.id = `west/${object.id}`;
  if (object.group) object.group = `west/${object.group}`;
}
document.groups.push(...other.groups);
document.objects.push(...other.objects);
document.assetSources.push(...other.assetSources);
document.sceneAssets.push(...other.sceneAssets);
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
assert.equal(doors.length, 4);
assert.ok(
  doors.every((door) => !door.passage_states?.some((state) => !state.allowed_states.length)),
);
const output = await fs.mkdtemp("work/needs-review/map-compile/courtyard-lodge-fixture-");
const file = "lodges.level.json";
await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
await fs.writeFile(`${output}/scene.json`, JSON.stringify(document));
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "copied-rotated-lodges-on-authored-terrain-not-full-map-parity",
    stage,
    results: [
      {
        file,
        warnings: compiled.warnings,
        building_approaches: doors.map((door) => {
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
              door.point_out[1] + (delta[1] * 32) / length + 90.00101,
              90.00101,
            ],
          };
        }),
      },
    ],
  }),
);
console.log(output);
