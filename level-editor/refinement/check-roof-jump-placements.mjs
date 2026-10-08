import fs from "node:fs/promises";
import { parseLevel3D, parseProjectionAssetDescriptor } from "../shared/src/validation.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { compileMap } from "../app/src/map-compile.ts";

const input = JSON.parse(
  await fs.readFile("shared/test-fixtures/complete-roof-jumps.json", "utf8"),
);
const source = parseLevel3D(input.document);
const asset = parseProjectionAssetDescriptor(input.asset);
const approachDepth = Math.min(
  ...asset.gameplay.surfaces.filter((s) => s.jump).map((s) => s.jump.landingDepth),
);
const output = await fs.mkdtemp("work/map-compile/prepared-roof-placements-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "prepared-roof-jump-placements-awaiting-native-traversal",
      complete,
      results,
    }),
  );
await report(false);
for (const rotation of [0, 37, 90, 180]) {
  for (const height of [0, 40]) {
    const document = roofJumpPlacement(source, rotation, height);
    const compiled = compileMap(document, [0, 0, 2000, 2000], new Map([[asset.id, asset]]), {
      bestEffort: true,
    });
    const file = `roofs-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
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
