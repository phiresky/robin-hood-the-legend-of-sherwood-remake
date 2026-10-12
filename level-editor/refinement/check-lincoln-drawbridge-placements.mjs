// Move independent asset assemblies together; retain native lock and traversal probes.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid } from "../shared/src/level3d.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage, "Supply staged drawbridge and plateau definitions");
const source = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const ids = [
  "lincoln-south-gate-drawbridge",
  "lincoln-castle-hill-inner-bailey-plateau",
  "lincoln-south-gatehouse",
];
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
for (const edit of JSON.parse(await fs.readFile(`${stage}/edits.json`))) {
  assert.ok(ids.includes(edit.asset));
  assert.equal(
    source.assetSources.find((entry) => entry.id === edit.asset)?.descriptor_sha256,
    edit.descriptorSha256,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const anchor = source.groups.find((group) => group.id === ids[0]).transform;
const output = await fs.mkdtemp("work/needs-review/map-compile/lincoln-drawbridge-placements-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      gameplayStage: stage,
      results,
    }),
  );
await report(false);
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const size = [7000, 5000];
    const document = {
      version: 1,
      map: "Drawbridge placement",
      camera: source.camera,
      size,
      objects: [],
      groups: [],
      sceneAssets: source.sceneAssets.filter((entry) => ids.includes(entry.id)),
      assetSources: source.assetSources.filter((entry) => ids.includes(entry.id)),
      terrain: createTerrainGrid([0, 0, ...size], 1000, height),
    };
    const angle = (rotation * Math.PI) / 180;
    const squash = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
    const rotate = ([x, y]) => [
      x * Math.cos(angle) - (y * Math.sin(angle)) / squash,
      x * Math.sin(angle) * squash + y * Math.cos(angle),
    ];
    for (const copy of [0, 1])
      for (const id of ids) {
        const group = structuredClone(source.groups.find((entry) => entry.id === id));
        assert.ok(group && group.transform.rot_deg === 0);
        const objects = source.objects.filter((object) => object.group === id);
        const pivot = groupCentroid(objects);
        const position = rotate([
          group.transform.dx - anchor.dx + pivot[0],
          group.transform.dy - anchor.dy + pivot[1],
        ]);
        group.id = `copy${copy}/${id}`;
        group.transform = {
          dx: 1800 + copy * 3000 + position[0] - pivot[0],
          dy: 2300 + position[1] - pivot[1],
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
    const compiled = compileMap(document, [0, 0, ...size], assets, { bestEffort: true });
    const transitions = compiled.descriptor.asset_geometry.movement_transitions;
    const passage_controls = [0, 1].map((copy) => {
      const prefix = `copy${copy}/${ids[0]}/`;
      const patches = transitions.flatMap((transition, index) =>
        [transition.id, ...(transition.aliases ?? [])].some((id) => id.startsWith(prefix))
          ? [{ index, id: transition.id }]
          : [],
      );
      assert.equal(patches.length, 2, compiled.warnings.join("\n"));
      const links = patches.flatMap((patch) => transitions[patch.index].door_links?.indices ?? []);
      assert.equal(links.length, 6, compiled.warnings.join("\n"));
      return { patches, doors: links, reject_alternatives: true };
    });
    assert.equal(new Set(passage_controls.flatMap((probe) => probe.doors)).size, 12);
    const file = `drawbridge-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({ file, height, rotation, warnings: compiled.warnings, passage_controls });
    await report(false);
    console.log(`${height}/${rotation}: twelve passages and independent controls retained`);
  }
await report(true);
