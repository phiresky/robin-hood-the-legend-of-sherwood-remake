import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";

const [staged, mode] = process.argv.slice(2);
assert.ok(staged, "Provide a staged edits directory");
assert.ok(mode === undefined || mode === "--published", "Unknown verification mode");
const edits = JSON.parse(await fs.readFile(`${staged}/edits.json`, "utf8"));
const review = JSON.parse(await fs.readFile(`${staged}/review.json`, "utf8"));
const externalDoors = review.changes
  .filter((change) => change.landing === "external placement receiver")
  .map((change) => change.door);
assert.equal(edits.length, 1, "This fixture places one complete asset independently");
const edit = edits[0];
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === edit.asset);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(hash(bytes), entry.descriptor_sha256);
const descriptor = JSON.parse(bytes);
if (mode === "--published") assert.deepEqual(descriptor.gameplay, edit.gameplay);
else {
  assert.equal(entry.descriptor_sha256, edit.descriptorSha256);
  descriptor.gameplay = edit.gameplay;
}
const reference = {
  id: entry.id,
  descriptor: `3d-assets/${entry.descriptor}`,
  descriptor_sha256: entry.descriptor_sha256,
  model: `3d-assets/${entry.model}`,
  model_sha256: hash(await fs.readFile(`library/3d-assets/${entry.model}`)),
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
};
const output = await fs.mkdtemp("work/map-compile/local-stair-placements-");
const results = [];
const gaps = [];
for (const height of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    const empty = {
      version: 1,
      map: edit.asset,
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      size: [4000, 4000],
      objects: [],
      groups: [],
      sceneAssets: [],
      assetSources: [],
      terrain: createTerrainGrid([0, 0, 4000, 4000], 1000, height),
    };
    const { document } = insertProjectionAsset(empty, descriptor, reference, [1700, 1800, height]);
    document.groups[0].transform.rot_deg = rotation;
    const compiled = compileMap(document, [0, 0, 4000, 4000], new Map([[entry.id, descriptor]]), {
      bestEffort: true,
    });
    const file = `${entry.id}-${height}-${rotation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    const geometry = compiled.descriptor.asset_geometry;
    results.push({
      file,
      map: file,
      lifts: geometry.lifts.length,
      physical: geometry.lifts.filter((lift) => lift.physical_navigation).length,
      controls: geometry.movement_transitions?.length ?? 0,
      warnings: compiled.warnings,
    });
    if (externalDoors.length) {
      const raised = structuredClone(document);
      raised.groups[0].transform.dz += 20;
      const disconnected = compileMap(
        raised,
        [0, 0, 4000, 4000],
        new Map([[entry.id, descriptor]]),
        { bestEffort: true },
      );
      for (const door of externalDoors)
        assert.ok(
          disconnected.warnings.some(
            (warning) => warning.includes(door) && warning.includes("traversal omitted"),
          ),
          `${file}: raised external entrance must reject`,
        );
      assert.ok(disconnected.descriptor.asset_geometry.lifts.length < geometry.lifts.length);
      gaps.push({
        height,
        rotation,
        raisedBy: 20,
        lifts: disconnected.descriptor.asset_geometry.lifts.length,
        warnings: disconnected.warnings,
      });
    }
  }
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "static-geometry-only-not-gameplay-parity", complete: true, results }),
);
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/external-gap-checks.json`, JSON.stringify(gaps));
console.log(
  JSON.stringify({ output, placements: results.map(({ warnings, ...result }) => result) }, null, 2),
);
