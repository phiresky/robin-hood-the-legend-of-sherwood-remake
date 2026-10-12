// Check independent neighbour edits without changing the intact assembly.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import { pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [placements] = process.argv.slice(2);
assert.ok(placements, "Supply the completed drawbridge placement audit directory");
const input = JSON.parse(await fs.readFile(`${placements}/diagnostics.json`));
assert.equal(input.complete, true);
const edits = JSON.parse(await fs.readFile(`${input.gameplayStage}/edits.json`));
const output = await fs.mkdtemp("work/needs-review/map-compile/lincoln-drawbridge-disconnections-");
console.log(output);
const results = [];
const writeReport = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete,
      placements,
      results,
    }),
  );
await writeReport(false);
for (const entry of input.results) {
  const source = JSON.parse(await fs.readFile(`${placements}/${entry.file}.scene.json`));
  const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
  for (const edit of edits) {
    assert.equal(
      source.assetSources.find((asset) => asset.id === edit.asset)?.descriptor_sha256,
      edit.descriptorSha256,
    );
    assets.get(edit.asset).gameplay = edit.gameplay;
  }
  for (const change of ["missing", "raised"]) {
    const document = structuredClone(source);
    const plateau = "copy0/lincoln-castle-hill-inner-bailey-plateau";
    if (change === "missing") {
      document.groups = document.groups.filter((group) => group.id !== plateau);
      document.objects = document.objects.filter((object) => object.group !== plateau);
    } else document.groups.find((group) => group.id === plateau).transform.dz += 40;
    const compiled = compileMap(document, [0, 0, ...document.size], assets, { bestEffort: true });
    const transitions = compiled.descriptor.asset_geometry.movement_transitions;
    const controls = [0, 1].map((copy) => {
      const prefix = `copy${copy}/lincoln-south-gate-drawbridge/`;
      const patches = transitions.flatMap((transition, index) =>
        [transition.id, ...(transition.aliases ?? [])].some((id) => id.startsWith(prefix))
          ? [{ index, id: transition.id }]
          : [],
      );
      return {
        patches,
        doors: patches.flatMap((patch) => transitions[patch.index].door_links?.indices ?? []),
        reject_alternatives: true,
      };
    });
    assert.equal(controls[1].doors.length, 6, "Untouched assembly must retain all passages");
    assert.equal(controls[0].doors.length, 3, "Only the three supported deck passages remain");
    for (const passage of [0, 2, 3]) {
      assert.ok(
        compiled.warnings.some(
          (warning) =>
            warning.startsWith("Door copy0/") &&
            warning.includes(`/passage-${passage}/door-0: omitted because`) &&
            warning.includes("unblocked walkable surface"),
        ),
        `Unsupported plateau passage ${passage} must be rejected`,
      );
    }
    const file = `${change}-${entry.file}`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    results.push({
      file,
      change,
      rotation: entry.rotation,
      height: entry.height,
      warnings: compiled.warnings,
      changed_passages: controls[0].doors,
      passage_controls: [controls[1]],
    });
    await writeReport(false);
    console.log(`${file}: changed=${controls[0].doors.length}, intact=${controls[1].doors.length}`);
  }
}
await writeReport(true);
