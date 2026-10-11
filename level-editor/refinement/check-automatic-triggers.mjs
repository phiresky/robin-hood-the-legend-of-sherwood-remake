import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { automaticTriggerCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { serializeStoredMap, parseStoredMap } from "../shared/src/index.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const output = await fs.mkdtemp("work/map-compile/automatic-triggers-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "editable-asset-occupancy-triggers",
      results,
    }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180]) {
  for (const elevation of [0, 40]) {
    const { document, assets } = automaticTriggerCompilerFixture();
    for (const group of document.groups)
      group.transform = { dx: 900, dy: 900, dz: elevation, rot_deg: rotation };
    const groups = structuredClone(document.groups);
    const objects = structuredClone(document.objects.filter((object) => object.group));
    document.groups.push(
      ...groups.map((group) => ({
        ...group,
        id: `copy/${group.id}`,
        transform: { ...group.transform, dx: group.transform.dx + 700 },
      })),
    );
    document.objects.push(
      ...objects.map((object) => ({
        ...object,
        id: `copy/${object.id}`,
        group: `copy/${object.group}`,
      })),
    );
    document.size = [3000, 2000];
    document.map = `Occupancy triggers ${rotation} ${elevation}`;
    const bounds = [0, 0, 3000, 2000];
    const compiled = compileMap(document, bounds, assets);
    const transitions = compiled.descriptor.asset_geometry.movement_transitions;
    assert.equal(transitions.length, 2);
    for (const transition of transitions) {
      assert.equal(transition.apply_polygon.points.length, 4);
      assert.equal(transition.initial_sight.length, 1);
      assert.equal(transition.applied_sight.length, 1);
    }
    const bytes = await packageCompiledMap(compiled, {
      color: new Uint8Array(3000 * 2000 * 4),
      depth: new Uint16Array(3000 * 2000),
    });
    const files = unzipSync(bytes);
    const read = (path) => {
      assert.ok(files[path]);
      return JSON.parse(strFromU8(files[path]));
    };
    const descriptor = read(`Data/Levels/${compiled.name}.level.json`);
    assert.deepEqual(descriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
    const compileReport = read("compile-report.json");
    assert.deepEqual(compileReport.warnings, compiled.warnings);
    const stored = read(compileReport.editor_document);
    const reopened = parseStoredMap(stored, assets);
    assert.deepEqual(serializeStoredMap(reopened, assets), serializeStoredMap(document, assets));
    assert.deepEqual(
      JSON.parse(JSON.stringify(compileMap(reopened, bounds, assets).descriptor)),
      descriptor,
    );
    const file = `trigger-${rotation}-${elevation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.zip`, bytes);
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({ file, rotation, elevation, warnings: compiled.warnings });
    await report(false);
  }
}
await report(true);
