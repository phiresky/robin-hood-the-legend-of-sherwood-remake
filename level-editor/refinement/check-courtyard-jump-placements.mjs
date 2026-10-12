// Exercise the two independent wall assets as a moved and copied assembly.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, groupParts } from "../shared/src/level3d.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply the staged asset gameplay directory");
const ids = ["nottingham-castle-west-courtyard-wall", "nottingham-southwest-curtain-wall-north"];
const saved = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", saved.assetSources, saved.sceneAssets);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
for (const edit of edits) {
  assert.ok(ids.includes(edit.asset));
  assert.equal(
    saved.assetSources.find((source) => source.id === edit.asset).descriptor_sha256,
    edit.descriptorSha256,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const objects = saved.objects.filter((part) =>
  ids.some((id) => part.node.startsWith(`asset:${id}:`)),
);
const groups = saved.groups.filter((group) => objects.some((part) => part.group === group.id));
assert.equal(groups.length, 2);
const centers = groups.map((group) => {
  const [x, y] = groupCentroid(groupParts(saved, group.id));
  return [x + group.transform.dx, y + group.transform.dy];
});
for (const group of groups) {
  group.transform.dx += 600 - centers.reduce((sum, p) => sum + p[0] / centers.length, 0);
  group.transform.dy += 900 - centers.reduce((sum, p) => sum + p[1] / centers.length, 0);
}
const source = {
  version: 1,
  map: "Courtyard wall jump assembly",
  camera: saved.camera,
  size: [2500, 2500],
  objects,
  groups,
  assetSources: saved.assetSources.filter((entry) => ids.includes(entry.id)),
  sceneAssets: [],
};
const output = await fs.mkdtemp("work/needs-review/map-compile/courtyard-jump-placements-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "static-geometry-only-not-gameplay-parity",
      gameplayStage: stage,
      results,
    }),
  );
await save(false);
for (const rotation of [0, 37, 90, 180])
  for (const height of [0, 40]) {
    const document = roofJumpPlacement(source, rotation, height);
    document.terrain = createTerrainGrid([0, 0, 2500, 2500], 2500, height);
    const compiled = compileMap(
      document,
      [0, 0, 2500, 2500],
      new Map(ids.map((id) => [id, assets.get(id)])),
      { bestEffort: true },
    );
    const pairs = compiled.descriptor.asset_geometry.jump_line_pairs ?? [];
    const zones = compiled.descriptor.asset_geometry.jump_zones;
    const helperPairs = pairs.filter(
      (pair) =>
        !pair.jump_long &&
        [pair.line1, pair.line2].some((line) => zones[line.jump_zone_index].helper_needed),
    );
    const file = `courtyard-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    assert.equal(
      helperPairs.length,
      2,
      `${rotation}/${height}: retain one independent helper connection per copy`,
    );
    results.push({
      file,
      rotation,
      height,
      pairs: pairs.length,
      helperPairs: helperPairs.length,
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
