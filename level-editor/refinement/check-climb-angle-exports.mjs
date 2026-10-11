import assert from "node:assert/strict";
import { mkdtemp, writeFile } from "node:fs/promises";
import { changingClimbCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Run from level-editor. Native traversal and barrier-state tests consume this
// manifest through ROBIN_ASSET_MAP_DIAGNOSTICS and ROBIN_CLIMB_RHS.
const args = process.argv.slice(2);
const selectedAngle = args[0]?.match(/^--angle=(\d+)$/)?.[1];
assert.ok(
  args.length === 0 ||
    (args.length === 1 && (args[0] === "--narrow" || (selectedAngle && +selectedAngle < 360))),
);
const angles = selectedAngle
  ? [+selectedAngle]
  : args.length
    ? [5, 6, 7, 8, 172, 173, 174, 175]
    : Array.from({ length: 360 }, (_, index) => index);
const output = await mkdtemp("work/map-compile/climb-angle-routes-");
console.log(output);
const results = [];
const report = (complete) =>
  writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      expected_directed_routes: angles.length * 6,
      scope: "editor-angle-probes-not-zip-round-trip",
      results,
    }),
  );
await report(false);
for (const [type, high] of [
  [2, 4],
  [3, 4],
  [3, 6],
]) {
  for (const degrees of angles) {
    const dz = 40;
    const { document, assets } = changingClimbCompilerFixture(type, high);
    document.groups[0].transform = { dx: 1100, dy: 1100, dz, rot_deg: degrees };
    const compiled = compileMap(document, [0, 0, 3000, 3000], assets);
    const file = `climb-${type}-${high}-${degrees}-${dz}.level.json`;
    await writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await writeFile(
      `${output}/${file}.scene.json`,
      JSON.stringify({ document, assets: [...assets] }),
    );
    results.push({ file, type, high, degrees, dz, warnings: compiled.warnings });
  }
  await report(false);
  console.log(`Compiled climb type ${type}, high direction ${high}`);
}
await report(true);
