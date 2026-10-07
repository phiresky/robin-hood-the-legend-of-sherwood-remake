import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid } from "../shared/src/level3d.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Provide the reviewed doorstep stage");
const source = await readStoredMap("library/scenes/york.rhlos-map.json", "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
const ids = ["york-west-town-lane-stone-tower", "york-west-town-lane-narrow-timber-house"];
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
for (const id of ids) {
  const edit = edits.find((edit) => edit.asset === id);
  assert.equal(
    edit?.descriptorSha256,
    source.assetSources.find((entry) => entry.id === id)?.descriptor_sha256,
  );
  assets.get(id).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/map-compile/receiver-floor-placements-");
console.log(output);
const results = [],
  rejected = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "compiler-door-and-terrain-connections-not-native-traversal",
      complete,
      gameplayStage: stage,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const size = [3000, 3000];
    const document = {
      version: 1,
      map: "receiver-floors",
      camera: source.camera,
      size,
      objects: [],
      groups: [],
      sceneAssets: source.sceneAssets.filter((entry) => ids.includes(entry.id)),
      assetSources: source.assetSources.filter((entry) => ids.includes(entry.id)),
      terrain: createTerrainGrid([0, 0, ...size], 1000, 90.00101 + height),
    };
    const radians = (rotation * Math.PI) / 180,
      sinT = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
    const rotate = ([x, y]) => [
      x * Math.cos(radians) - (y * Math.sin(radians)) / sinT,
      x * Math.sin(radians) * sinT + y * Math.cos(radians),
    ];
    for (const [index, id] of ids.entries())
      for (const copy of [0, 1]) {
        const group = structuredClone(source.groups.find((group) => group.id === id));
        assert.ok(group && group.transform.rot_deg === 0);
        const objects = source.objects.filter((object) => object.group === id);
        const pivot = groupCentroid(objects),
          rotated = rotate(pivot);
        group.id = `copy${copy}/${id}`;
        group.transform = {
          dx: 800 + copy * 1400 - pivot[0] + rotated[0],
          dy: 800 + index * 1400 - pivot[1] + rotated[1],
          dz: group.transform.dz + height,
          rot_deg: rotation,
        };
        document.groups.push(group);
        document.objects.push(
          ...objects.map((object) => ({
            ...structuredClone(object),
            id: `copy${copy}/${object.id}`,
            group: group.id,
          })),
        );
      }
    const compile = (document) =>
      compileMap(document, [0, 0, ...size], assets, { bestEffort: true });
    const compiled = compile(document);
    const unbound = (result) =>
      result.warnings.filter(
        (warning) =>
          warning.startsWith("Navigation region ") && warning.includes("-receiver-physical-floor:"),
      );
    assert.equal(
      compiled.descriptor.asset_geometry.buildings.length,
      4,
      compiled.warnings.join("\n"),
    );
    // Each four-sided doorstep exposes one lower edge to independently authored ground.
    assert.equal(unbound(compiled).length, 12, unbound(compiled).join("\n"));
    for (const group of document.groups)
      assert.equal(
        unbound(compiled).filter((warning) => warning.includes(`Navigation region ${group.id}/`))
          .length,
        3,
        `${group.id}: expected exactly one terrain connection`,
      );
    const file = `receiver-floors-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ map: file, file, warnings: compiled.warnings });
    for (const kind of ["missing", "raised"]) {
      const changed = structuredClone(document);
      if (kind === "missing") delete changed.terrain;
      else for (const vertex of changed.terrain.vertices) vertex.position[2] += 20;
      const invalid = compile(changed);
      assert.equal(unbound(invalid).length, 16, `${file}: ${kind} ground still connects`);
      for (const group of document.groups)
        assert.equal(
          unbound(invalid).filter((warning) => warning.includes(`Navigation region ${group.id}/`))
            .length,
          4,
          `${group.id}: disconnected terrain still matches a socket`,
        );
      rejected.push({ file, kind });
    }
    await report(false);
  }
await fs.writeFile(`${output}/rejected.json`, JSON.stringify(rejected));
await report(true);
console.log(
  JSON.stringify({
    output,
    exports: results.length,
    placements: results.length * 4,
    rejected: rejected.length * 4,
  }),
);
