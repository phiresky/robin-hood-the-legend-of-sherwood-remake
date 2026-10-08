import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { crossAssetJumpCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { parseLevel3D } from "../shared/src/validation.ts";

// Keep both sides independently authored; connection assembly uses their placed edges.
const output = await fs.mkdtemp("work/map-compile/prepared-vertical-jumps-");
console.log(output);
const results = [];
const automatic = process.argv.includes("--automatic");
for (const height of [0, 40]) {
  const { document, assets, upper } = crossAssetJumpCompilerFixture();
  // Align the landing in map space; asset coordinates include elevation in Y.
  for (const surface of upper.gameplay.surfaces)
    for (const point of surface.polygon) point[1] += 100;
  for (const zone of upper.gameplay.jumpZones) {
    zone.anchor[1] += 100;
    for (const point of zone.polygon) point[1] += 100;
  }
  for (const segment of upper.gameplay.jumpSegments) {
    segment.edge.a[1] += 100;
    segment.edge.b[1] += 100;
  }
  for (const asset of assets.values()) {
    for (const segment of asset.gameplay?.jumpSegments ?? []) {
      segment.long = false;
      [segment.edge.a, segment.edge.b] = [segment.edge.b, segment.edge.a];
      // Authored takeoff/landing lines reserve the native six-unit half-width.
      segment.edge.a[0] += asset === upper ? 3 : -3;
      segment.edge.b[0] += asset === upper ? 3 : -3;
      if (automatic) {
        delete segment.join;
        segment.attachment = { maxGap: 80, maxRise: 110, maxDrop: 110, minOverlap: 10 };
      }
    }
  }
  for (const rotation of [0, 37, 90, 180]) {
    const placed = roofJumpPlacement(document, rotation, height);
    const compiled = compileMap(placed, [0, 0, 4000, 4000], assets);
    const scene = JSON.stringify(placed);
    const reopened = parseLevel3D(JSON.parse(scene));
    assert.deepEqual(
      compileMap(reopened, [0, 0, 4000, 4000], assets).descriptor,
      compiled.descriptor,
    );
    if (!automatic && compiled.descriptor.asset_geometry?.jump_line_pairs?.length !== 2)
      throw new Error("Expected two independent authored vertical connections");
    const file = `vertical-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, scene);
    results.push({
      file,
      rotation,
      height,
      automatic,
      editor_roundtrip: true,
      approach_depth: 4,
      pairs: compiled.descriptor.asset_geometry?.jump_line_pairs?.length ?? 0,
      warnings: compiled.warnings,
    });
  }
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "prepared-vertical-jumps-awaiting-native-traversal",
    complete: true,
    results,
  }),
);
assert.ok(
  results.some((result) => result.pairs > 0),
  "No native traversal cases were exported",
);
