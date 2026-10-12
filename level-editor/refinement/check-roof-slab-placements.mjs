// Exercise a slab-to-roof connection and its jump in copied assemblies.
// Other jump overlays are isolated out; all physical asset geometry is retained.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, groupParts } from "../shared/src/level3d.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { parseStoredMap, serializeStoredMap } from "../shared/src/index.ts";

const zipRoundtrip = process.argv.includes("--zip-roundtrip");
const published = process.argv.includes("--published");
const arguments_ = process.argv
  .slice(2)
  .filter((value) => !["--zip-roundtrip", "--published"].includes(value));
const [stage, rotationArgument] = published ? [undefined, ...arguments_] : arguments_;
const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
assert.ok(stage || published, "Supply a staged asset gameplay directory or --published");
const rotations =
  rotationArgument === undefined ? [0, 37, 90, 180] : rotationArgument.split(",").map(Number);
assert.ok(rotations.length && rotations.every(Number.isFinite));
const ids = [
  "nottingham-north-street-roof-edge-slab",
  "nottingham-north-stone-house",
  "nottingham-upper-green-house",
];
const saved = await readStoredMap("library/scenes/nottingham.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", saved.assetSources, saved.sceneAssets);
const edits = stage ? JSON.parse(await fs.readFile(`${stage}/edits.json`)) : [];
for (const edit of edits) {
  assert.ok(ids.includes(edit.asset));
  assert.equal(
    saved.assetSources.find((source) => source.id === edit.asset).descriptor_sha256,
    edit.descriptorSha256,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
for (const id of ids) {
  const gameplay = assets.get(id).gameplay;
  gameplay.jumpPairs = [];
  gameplay.jumpSegments = (gameplay.jumpSegments ?? []).filter((segment) =>
    ["jump-pair-32-side-0", "jump-pair-32-side-1"].includes(segment.id),
  );
  const zones = new Set(gameplay.jumpSegments.map((segment) => segment.edge.zone));
  gameplay.jumpZones = (gameplay.jumpZones ?? []).filter((zone) => zones.has(zone.id));
}
const objects = saved.objects.filter((part) =>
  ids.some((id) => part.node.startsWith(`asset:${id}:`)),
);
const groups = saved.groups.filter((group) => objects.some((part) => part.group === group.id));
assert.equal(groups.length, 3);
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
  map: "Connected roof slab jump assembly",
  camera: saved.camera,
  size: [2500, 2500],
  objects,
  groups,
  assetSources: saved.assetSources.filter((entry) => ids.includes(entry.id)),
  sceneAssets: [],
};
const output = await fs.mkdtemp("work/needs-review/map-compile/roof-slab-placements-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "static-geometry-only-not-gameplay-parity",
      zip_roundtrip: zipRoundtrip,
      rendered_pixels: false,
      gameplayStage: stage ?? "published-library",
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
    const file = `roof-slab-${rotation}-${height}.level.json`;
    let descriptor = compiled.descriptor;
    if (zipRoundtrip) {
      // Placeholder pixels test packaging and editor reopening, not rendered fidelity.
      const archive = await packageCompiledMap(compiled, {
        color: new Uint8Array(2500 * 2500 * 4),
        depth: new Uint16Array(2500 * 2500),
      });
      const files = unzipSync(archive);
      const read = (name) => {
        assert.ok(files[name], `ZIP omitted ${name}`);
        return JSON.parse(strFromU8(files[name]));
      };
      descriptor = read(`Data/Levels/${compiled.name}.level.json`);
      assert.deepEqual(descriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
      const report = read("compile-report.json");
      assert.deepEqual(report.warnings, compiled.warnings);
      const reopened = parseStoredMap(read(report.editor_document), assets);
      assert.deepEqual(serializeStoredMap(reopened, assets), serializeStoredMap(document, assets));
      assert.deepEqual(
        JSON.parse(
          JSON.stringify(
            compileMap(reopened, [0, 0, 2500, 2500], assets, { bestEffort: true }).descriptor,
          ),
        ),
        descriptor,
      );
      await fs.writeFile(`${output}/${file}.zip`, archive);
    }
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    assert.equal(
      pairs.length,
      2,
      `${rotation}/${height}: retain one independent slab jump per copy`,
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
