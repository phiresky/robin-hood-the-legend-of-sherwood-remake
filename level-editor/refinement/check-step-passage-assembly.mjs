// Isolate the step contact and its independently placeable doorway prop.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { groupCentroid } from "../shared/src/level3d.ts";

const [stage] = process.argv.slice(2);
assert.ok(stage);
const source = await readStoredMap("library/scenes/lincoln.rhlos-map.json", "library");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
for (const edit of edits) {
  assert.equal(
    source.assetSources.find((asset) => asset.id === edit.asset)?.descriptor_sha256,
    edit.descriptorSha256,
  );
  assets.get(edit.asset).gameplay = edit.gameplay;
}
const output = await fs.mkdtemp("work/needs-review/map-compile/step-passage-assembly-");
const results = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180])
    for (const prop of [false, true]) {
      const ids = [
        "lincoln-south-wall-walk-step",
        "lincoln-south-curtain-wall-angle",
        "lincoln-southeast-curtain-wall",
        ...(prop ? ["lincoln-south-wall-cone-turret"] : []),
      ];
      const document = {
        version: 1,
        map: "Step passage assembly",
        camera: source.camera,
        size: [3000, 3000],
        groups: structuredClone(source.groups.filter((group) => ids.includes(group.id))),
        objects: source.objects.filter((object) => ids.includes(object.group)),
        sceneAssets: [],
        assetSources: source.assetSources.filter((asset) => ids.includes(asset.id)),
        terrain: createTerrainGrid([0, 0, 3000, 3000], 1000, height),
      };
      const anchor = source.groups.find((group) => group.id === ids[0]).transform;
      const angle = (rotation * Math.PI) / 180;
      const squash = Math.sin((source.camera.elevation_deg * Math.PI) / 180);
      for (const group of document.groups) {
        assert.equal(group.transform.rot_deg, 0);
        const pivot = groupCentroid(document.objects.filter((object) => object.group === group.id));
        const x = group.transform.dx - anchor.dx + pivot[0];
        const y = group.transform.dy - anchor.dy + pivot[1];
        group.transform = {
          dx: 1500 + x * Math.cos(angle) - (y * Math.sin(angle)) / squash - pivot[0],
          dy: 1500 + x * Math.sin(angle) * squash + y * Math.cos(angle) - pivot[1],
          dz: group.transform.dz + height,
          rot_deg: rotation,
        };
      }
      const compiled = compileMap(document, [0, 0, 3000, 3000], assets, { bestEffort: true });
      const passages = compiled.descriptor.asset_geometry.doors;
      assert.equal(passages.length, 1);
      const blocked =
        passages[0].passage_states?.some((state) => state.allowed_states.length === 0) ?? false;
      assert.equal(blocked, prop, "Only the doorway prop should block this isolated approach");
      const file = `step-${height}-${rotation}-${prop ? "with-prop" : "without-prop"}.level.json`;
      await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
      await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
      results.push({
        file,
        prop,
        height,
        rotation,
        warnings: compiled.warnings,
        passages,
        passage_controls: [
          {
            patches: [],
            doors: [0],
            reject_alternatives: true,
            fixed_state: prop ? "closed" : "open",
          },
        ],
      });
    }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "static-geometry-only-not-gameplay-parity",
    complete: true,
    results,
  }),
);
console.log(output);
