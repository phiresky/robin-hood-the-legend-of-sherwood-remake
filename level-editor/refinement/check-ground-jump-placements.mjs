// Exercise a ground/roof jump in copied assemblies.
// Other jump overlays are isolated out; all physical asset geometry is retained.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, groupParts } from "../shared/src/level3d.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage, rotationArgument] = process.argv.slice(2);
assert.ok(stage, "Supply the staged asset gameplay directory");
const rotations =
  rotationArgument === undefined ? [0, 37, 90, 180] : rotationArgument.split(",").map(Number);
assert.ok(rotations.length && rotations.every(Number.isFinite));
const ids = ["nottingham-southeast-green-house"];
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
for (const id of ids) {
  const g = assets.get(id).gameplay;
  g.jumpPairs = g.jumpPairs.filter((p) => p.id === "jump-pair-18");
  g.jumpSegments = [];
  const zones = new Set(g.jumpPairs.flatMap((p) => p.edges.map((e) => e.zone)));
  g.jumpZones = g.jumpZones.filter((z) => zones.has(z.id));
}
const objects = saved.objects.filter((part) =>
  ids.some((id) => part.node.startsWith(`asset:${id}:`)),
);
const groups = saved.groups.filter((group) => objects.some((part) => part.group === group.id));
assert.equal(groups.length, 1);
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
  map: "Ground landing click-band assembly",
  camera: saved.camera,
  size: [2500, 2500],
  objects,
  groups,
  assetSources: saved.assetSources.filter((entry) => ids.includes(entry.id)),
  sceneAssets: [],
};
const output = await fs.mkdtemp("work/map-compile/ground-jump-placements-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "static-geometry-only-not-gameplay-parity",
      gameplayStage: stage,
      rotations,
      results,
    }),
  );
await save(false);
for (const rotation of rotations)
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
    const file = `ground-jump-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    assert.equal(
      pairs.length,
      2,
      `${rotation}/${height}: retain one independent ground/roof jump per copy`,
    );
    results.push({
      file,
      rotation,
      height,
      pairs: pairs.length,
      warnings: compiled.warnings,
    });
    await save(false);
  }
await save(true);
