import fs from "node:fs/promises";
import { crossAssetJumpCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Keep both sides independently authored; connection assembly uses their placed edges.
const output = await fs.mkdtemp("work/map-compile/prepared-vertical-jumps-");
console.log(output);
const results = [];
for (const height of [0, 40]) {
  const { document, assets } = crossAssetJumpCompilerFixture();
  for (const asset of assets.values()) {
    for (const segment of asset.gameplay?.jumpSegments ?? []) segment.long = false;
  }
  for (const group of document.groups) group.transform.dz += height;
  const compiled = compileMap(document, [0, 0, 2000, 2000], assets);
  if (compiled.descriptor.asset_geometry?.jump_line_pairs?.length !== 1)
    throw new Error("Expected one authored vertical connection between the separate assets");
  const file = `vertical-${height}.level.json`;
  await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
  await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(document));
  results.push({
    file,
    height,
    approach_depth: 4,
    pairs: compiled.descriptor.asset_geometry?.jump_line_pairs?.length ?? 0,
    warnings: compiled.warnings,
  });
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({ scope: "prepared-vertical-jumps-awaiting-native-traversal", complete: true, results }),
);
