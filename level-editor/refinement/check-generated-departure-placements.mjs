// Surface-only authoring regression: no recovered jump lines or landing zones.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { surfaceJumpCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { groupCentroid, groupParts } from "../shared/src/level3d.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { parseStoredMap, serializeStoredMap } from "../shared/src/index.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const riseArgument = process.argv.slice(2);
assert.ok(riseArgument.length <= 1 && (!riseArgument.length || riseArgument[0].startsWith("--rise=")));
const rise = riseArgument.length ? Number(riseArgument[0].slice("--rise=".length)) : 0;
assert.ok(Number.isFinite(rise) && rise >= 0 && rise <= 40);
const { document: source, assets } = surfaceJumpCompilerFixture();
for (const asset of assets.values())
  for (const surface of asset.gameplay?.surfaces ?? [])
    if (surface.jump) {
      surface.height = 100 + (surface.id === "east" ? rise : 0);
      surface.jump.inset = 17;
      surface.jump.maxGap = 100;
    }
// Place physically adjacent platforms independently of their elevation.
for (const group of source.groups) {
  group.transform.dx = 0;
  group.transform.dy = 0;
}
const centers = source.groups.map((group) => {
  const center = groupCentroid(groupParts(source, group.id));
  return [center[0] + group.transform.dx, center[1] + group.transform.dy];
});
const center = [0, 1].map((axis) =>
  centers.reduce((sum, point) => sum + point[axis], 0) / centers.length);
for (const group of source.groups) {
  group.transform.dx += 600 - center[0];
  group.transform.dy += 900 - center[1];
}
const output = await fs.mkdtemp("work/needs-review/map-compile/generated-departure-placements-");
console.log(output);
const results = [];
const report = (complete) => fs.writeFile(`${output}/diagnostics.json`, JSON.stringify({
  scope: "surface-only-generated-departures-zip-roundtrip-awaiting-native-traversal",
  rise, complete, results,
}));
await report(false);
for (const rotation of [0, 37, 90, 180]) {
  for (const height of [0, 40]) {
    const document = roofJumpPlacement(source, rotation, height);
    const compiled = compileMap(document, [0, 0, 2000, 2000], assets, { bestEffort: true });
    const pairs = compiled.descriptor.asset_geometry?.jump_line_pairs ?? [];
    assert.equal(pairs.length, 2, `${rotation}/${height}: both copied connections must survive`);
    assert.ok(!compiled.warnings.some((warning) =>
      warning.includes("takeoff animation lacks receiving coverage")),
    `${rotation}/${height}: departure must remain supported`);
    const zip = await packageCompiledMap(compiled, {
      color: new Uint8Array(2000 * 2000 * 4), depth: new Uint16Array(2000 * 2000),
    });
    const files = unzipSync(zip);
    const read = (path) => {
      assert.ok(files[path], `Missing archive entry ${path}`);
      return JSON.parse(strFromU8(files[path]));
    };
    const descriptor = read(`Data/Levels/${compiled.name}.level.json`);
    assert.deepEqual(descriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
    const compilationReport = read("compile-report.json");
    const reopened = parseStoredMap(read(compilationReport.editor_document), assets);
    assert.deepEqual(serializeStoredMap(reopened, assets), serializeStoredMap(document, assets));
    assert.deepEqual(JSON.parse(JSON.stringify(
      compileMap(reopened, [0, 0, 2000, 2000], assets, { bestEffort: true }).descriptor,
    )), descriptor);
    const file = `generated-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.zip`, zip);
    results.push({ file, rotation, height, pairs: pairs.length, approach_depth: 6,
      warnings: compiled.warnings });
    await report(false);
  }
}
await report(true);
