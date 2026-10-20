import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { parseLevel3D, parseProjectionAssetDescriptor } from "../shared/src/validation.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";
import { parseStoredMap, serializeStoredMap } from "../shared/src/index.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);
const zipRoundtrip = process.argv.includes("--zip-roundtrip");
const insetArguments = process.argv.slice(2).filter((argument) => argument.startsWith("--inset="));
assert.ok(insetArguments.length <= 1);
const diagnosticInset = insetArguments.length ? Number(insetArguments[0].slice(8)) : undefined;
assert.ok(
  diagnosticInset === undefined || (Number.isFinite(diagnosticInset) && diagnosticInset >= 0),
  "Diagnostic inset must be a nonnegative finite number",
);

const input = JSON.parse(
  await fs.readFile("shared/test-fixtures/complete-roof-jumps.json", "utf8"),
);
const source = parseLevel3D(input.document);
const asset = parseProjectionAssetDescriptor(input.asset);
if (diagnosticInset !== undefined)
  for (const surface of asset.gameplay.surfaces)
    if (surface.jump) surface.jump.inset = diagnosticInset;
const crouched = process.argv.includes("--crouched");
if (crouched) for (const surface of asset.gameplay.surfaces) surface.forceCrouched = true;
const approachDepth = Math.min(
  ...asset.gameplay.surfaces.filter((s) => s.jump).map((s) => s.jump.landingDepth),
);
const output = await fs.mkdtemp(
  `work/needs-review/map-compile/prepared-${crouched ? "crouched-" : ""}roof-placements-`,
);
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "prepared-roof-jump-placements-awaiting-native-traversal",
      crouched,
      zip_roundtrip: zipRoundtrip,
      diagnostic_inset: diagnosticInset,
      complete,
      results,
    }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180]) {
  for (const height of [0, 40]) {
    const document = roofJumpPlacement(source, rotation, height);
    const assets = new Map([[asset.id, asset]]);
    const bounds = [0, 0, 2000, 2000];
    const compiled = compileMap(document, bounds, assets, {
      bestEffort: true,
    });
    // The 90-degree layout is intentionally too narrow for this actor footprint.
    if (rotation !== 90 && diagnosticInset === undefined)
      assert.ok(
        compiled.descriptor.asset_geometry?.jump_line_pairs?.length > 0,
        `Roof traversal fixture ${rotation}/${height} exported no jump connections`,
      );
    const file = `roofs-${rotation}-${height}.level.json`;
    let descriptor = compiled.descriptor;
    if (zipRoundtrip) {
      const bytes = await packageCompiledMap(compiled, {
        color: new Uint8Array(2000 * 2000 * 4),
        depth: new Uint16Array(2000 * 2000),
      });
      const files = unzipSync(bytes);
      const read = (path) => {
        assert.ok(files[path]);
        return JSON.parse(strFromU8(files[path]));
      };
      descriptor = read(`Data/Levels/${compiled.name}.level.json`);
      assert.deepEqual(descriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
      const report = read("compile-report.json");
      assert.deepEqual(report.warnings, compiled.warnings);
      const reopened = parseStoredMap(read(report.editor_document), assets);
      assert.deepEqual(serializeStoredMap(reopened, assets), serializeStoredMap(document, assets));
      assert.deepEqual(
        JSON.parse(
          JSON.stringify(compileMap(reopened, bounds, assets, { bestEffort: true }).descriptor),
        ),
        descriptor,
      );
      await fs.writeFile(`${output}/${file}.zip`, bytes);
    }
    await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
    results.push({
      file,
      rotation,
      height,
      approach_depth: approachDepth,
      pairs: compiled.descriptor.asset_geometry?.jump_line_pairs?.length ?? 0,
      warnings: compiled.warnings,
    });
    await report(false);
  }
}
await report(true);
