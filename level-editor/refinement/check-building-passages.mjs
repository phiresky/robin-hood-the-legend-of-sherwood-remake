import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { interiorPassageBarrierCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { serializeStoredMap, parseStoredMap } from "../shared/src/index.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const options = process.argv.slice(2);
assert.ok(
  options.every((option) => option === "--trap"),
  "only --trap is supported",
);
const trap = options.includes("--trap");
const output = await fs.mkdtemp(`work/needs-review/map-compile/building-${trap ? "trap-" : ""}passages-`);
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "editable-building-passage-zips-awaiting-native-traversal",
      trap,
      results,
    }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180])
  for (const height of [0, 40]) {
    const { document, assets, hut } = interiorPassageBarrierCompilerFixture();
    if (trap)
      for (const room of hut.gameplay.interiors) for (const door of room.doors) door.type = 2;
    // Leave room for one animation step before closure at every direction.
    // A barrier appearing over the actor tests crushing instead of suspension.
    for (const point of hut.gameplay.movementTransitions[0].applied[0].polygon) point[1] += 4;
    document.size = [3000, 2000];
    document.groups.forEach((group, index) => {
      group.transform = { dx: 900 + index * 700, dy: 900, dz: height, rot_deg: rotation };
    });
    const bounds = [0, 0, 3000, 2000];
    const compiled = compileMap(document, bounds, assets);
    assert.ok(
      !compiled.warnings.some((warning) => warning.includes("clipped to the export frame")),
    );
    assert.equal(compiled.descriptor.asset_geometry.buildings.length, 2);
    for (const { Building } of compiled.descriptor.asset_geometry.buildings) {
      assert.equal(Building.doors.length, 2);
      assert.ok(Building.doors.every((door) => door.door_type === (trap ? 2 : 1)));
      assert.deepEqual(
        Building.doors[0].passage_states.map((state) => state.allowed_states),
        [[1]],
      );
      assert.equal(Building.doors[1].passage_states, undefined);
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
    const file = `building-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.zip`, bytes);
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({ file, rotation, height, warnings: compiled.warnings });
    await report(false);
  }
await report(true);
