import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { crossAssetJumpCompilerFixture } from "../shared/test-fixtures/asset-gameplay.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { parseLevel3D } from "../shared/src/validation.ts";
import { partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { applyAffineMatrix, sceneToGame } from "../shared/src/geometry.ts";

// Keep both sides independently authored; connection assembly uses their placed edges.
const output = await fs.mkdtemp("work/map-compile/prepared-vertical-jumps-");
console.log(output);
const results = [];
const automatic = process.argv.includes("--automatic");
const reposition = process.argv.includes("--reposition");
assert.ok(!reposition || automatic, "Repositioning requires geometric attachment rules");
for (const height of [0, 40]) {
  const { document, assets, upper, hut } = crossAssetJumpCompilerFixture();
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
      const inset = reposition ? 5 : 3;
      segment.edge.a[0] += asset === upper ? inset : -inset;
      segment.edge.b[0] += asset === upper ? inset : -inset;
      if (automatic) {
        delete segment.join;
        segment.attachment = { maxGap: 80, maxRise: 110, maxDrop: 110, minOverlap: 10 };
      }
    }
  }
  for (const rotation of [0, 37, 90, 180]) {
    const placed = roofJumpPlacement(document, rotation, height);
    if (reposition) {
      const edge = (group, asset) => {
        const part = placed.objects.find((part) => part.group === group);
        assert.ok(part);
        const matrix = partMatrix(placed.camera, placed, part);
        const segment = asset.gameplay.jumpSegments[0];
        return [segment.edge.a, segment.edge.b].map((point) => {
          const [x, y, z] = sceneToGame(
            placed.camera,
            applyAffineMatrix(matrix, gameToScene(placed.camera, ...point)),
          );
          return [x, y - z];
        });
      };
      for (const copy of [0, 1]) {
        const [a, b] = edge(`${copy}/hut-a`, hut);
        const [c, d] = edge(`${copy}/jump-upper`, upper);
        const dx = b[0] - a[0],
          dy = b[1] - a[1],
          length = Math.hypot(dx, dy);
        const group = placed.groups.find((group) => group.id === `${copy}/jump-upper`);
        group.transform.dx += (a[0] + b[0] - c[0] - d[0]) / 2 - (40 * dy) / length;
        group.transform.dy += (a[1] + b[1] - c[1] - d[1]) / 2 + (40 * dx) / length;
      }
    }
    const compiled = compileMap(placed, [0, 0, 4000, 4000], assets);
    const scene = JSON.stringify(placed);
    const reopened = parseLevel3D(JSON.parse(scene));
    assert.deepEqual(
      compileMap(reopened, [0, 0, 4000, 4000], assets).descriptor,
      compiled.descriptor,
    );
    if (
      (!automatic || reposition) &&
      compiled.descriptor.asset_geometry?.jump_line_pairs?.length !== 2
    )
      throw new Error(
        `Expected two independent vertical connections: ${rotation}/${height}: ${compiled.warnings.join("; ")}`,
      );
    let rejectedPlacements = 0;
    if (reposition) {
      for (const copy of [0, 1]) {
        for (const change of [{ dx: 1000 }, { dz: 20 }]) {
          const separated = structuredClone(placed);
          const group = separated.groups.find((group) => group.id === `${copy}/jump-upper`);
          for (const [axis, amount] of Object.entries(change)) group.transform[axis] += amount;
          const rejected = compileMap(separated, [0, 0, 4000, 4000], assets);
          assert.equal(
            rejected.descriptor.asset_geometry?.jump_line_pairs?.length,
            1,
            "Moving one upper asset out of range must leave only the other copy connected",
          );
          assert.ok(rejected.warnings.some((warning) => warning.includes("no matching edge")));
          rejectedPlacements++;
        }
      }
    }
    const file = `vertical-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, scene);
    results.push({
      file,
      rotation,
      height,
      automatic,
      reposition,
      rejected_placements: rejectedPlacements,
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
