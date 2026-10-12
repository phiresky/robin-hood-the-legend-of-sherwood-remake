// Check an external asset connection in moved, rotated and elevated assemblies.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { groupCentroid, groupParts, partMatrix } from "../shared/src/level3d.ts";
import { gameToScene } from "../shared/src/scene.ts";
import { sceneToGame } from "../shared/src/geometry.ts";
import { roofJumpPlacement } from "../shared/test-fixtures/roof-jump-placements.ts";
import { createTerrainGrid } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const diagnosticPrecision = process.argv.includes("--diagnostic-precision");
const includeInternal = process.argv.includes("--include-internal");
const internalPair = process.argv.includes("--internal-pair");
assert.ok(!internalPair || (!includeInternal && !diagnosticPrecision));
assert.ok(
  !includeInternal || !diagnosticPrecision,
  "Precision diagnostic only supports isolated external pairs",
);
const terrainArguments = process.argv
  .slice(2)
  .filter((argument) => argument.startsWith("--terrain-offset="));
assert.ok(terrainArguments.length <= 1, "Supply at most one terrain offset");
const terrainOffset = terrainArguments.length
  ? Number(terrainArguments[0].slice("--terrain-offset=".length))
  : 0;
assert.ok(Number.isFinite(terrainOffset));
const rotationArguments = process.argv
  .slice(2)
  .filter((argument) => argument.startsWith("--rotations="));
assert.ok(rotationArguments.length <= 1, "Supply at most one rotation list");
const rotations = rotationArguments.length
  ? rotationArguments[0].slice("--rotations=".length).split(",").map(Number)
  : [0, 37, 90, 180];
assert.ok(
  rotations.length &&
    rotations.every((angle) => Number.isFinite(angle) && angle >= 0 && angle < 360),
);
assert.equal(new Set(rotations).size, rotations.length, "Rotations must be unique");
const [map, pair, assetList, stage] = process.argv
  .slice(2)
  .filter(
    (argument) =>
      argument !== "--diagnostic-precision" &&
      argument !== "--include-internal" &&
      argument !== "--internal-pair" &&
      !argument.startsWith("--rotations=") &&
      !argument.startsWith("--terrain-offset="),
  );
assert.ok(
  map && pair && assetList,
  "Supply map, pair prefix, comma-separated asset IDs and optional stage",
);
const ids = assetList.split(",");
assert.ok(!internalPair || ids.length === 1, "An internal pair belongs to one asset");
const saved = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", saved.assetSources, saved.sceneAssets);
const edits = stage ? JSON.parse(await fs.readFile(`${stage}/edits.json`)) : [];
for (const edit of edits) {
  assert.ok(ids.includes(edit.asset));
  const source = saved.assetSources.find((source) => source.id === edit.asset);
  assert.equal(source.descriptor_sha256, edit.descriptorSha256);
  assert.equal(source.model_sha256, edit.modelSha256);
  assets.get(edit.asset).gameplay = edit.gameplay;
}
for (const id of ids) {
  const gameplay = assets.get(id).gameplay;
  if (internalPair) {
    gameplay.jumpPairs = gameplay.jumpPairs.filter((entry) => entry.id === pair);
    assert.equal(gameplay.jumpPairs.length, 1, `${id}: expected one internal pair`);
  } else if (!includeInternal) gameplay.jumpPairs = [];
  gameplay.jumpSegments = (gameplay.jumpSegments ?? []).filter(
    (segment) => !internalPair && [pair + "-side-0", pair + "-side-1"].includes(segment.id),
  );
  assert.equal(gameplay.jumpSegments.length, internalPair ? 0 : 1, `${id}: connection endpoints`);
  const zones = new Set(gameplay.jumpSegments.map((segment) => segment.edge.zone));
  for (const pair of gameplay.jumpPairs ?? []) for (const edge of pair.edges) zones.add(edge.zone);
  gameplay.jumpZones = gameplay.jumpZones.filter((zone) => zones.has(zone.id));
}
const objects = saved.objects.filter((part) =>
  ids.some((id) => part.node.startsWith(`asset:${id}:`)),
);
const groups = saved.groups.filter((group) => objects.some((part) => part.group === group.id));
assert.equal(groups.length, ids.length);
const centers = groups.map((group) => {
  const [x, y] = groupCentroid(groupParts(saved, group.id));
  return [x + group.transform.dx, y + group.transform.dy];
});
for (const group of groups) {
  group.transform.dx += 600 - centers.reduce((sum, p) => sum + p[0] / centers.length, 0);
  group.transform.dy += 900 - centers.reduce((sum, p) => sum + p[1] / centers.length, 0);
}
const source = {
  version: 1,
  map: `${map} ${pair} assembly`,
  camera: saved.camera,
  size: [2500, 2500],
  objects,
  groups,
  assetSources: saved.assetSources.filter((entry) => ids.includes(entry.id)),
  sceneAssets: [],
};
const output = await fs.mkdtemp("work/needs-review/map-compile/connected-jump-placements-");
console.log(output);
const results = [];
const save = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      complete,
      scope: "static-geometry-only-not-gameplay-parity",
      stage,
      terrainOffset,
      includeInternal,
      internalPair,
      results,
    }),
  );
await save(false);
for (const rotation of rotations)
  for (const height of [0, 40]) {
    const document = roofJumpPlacement(source, rotation, height);
    document.terrain = createTerrainGrid([0, 0, 2500, 2500], 2500, height + terrainOffset);
    const compiled = compileMap(
      document,
      [0, 0, 2500, 2500],
      new Map(ids.map((id) => [id, assets.get(id)])),
      { bestEffort: true },
    );
    const file = `connection-${rotation}-${height}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    let diagnosticEndpoints;
    if (diagnosticPrecision) {
      const segments = document.groups.map((group) => {
        const parts = groupParts(document, group.id);
        const id = parts[0].node.split(":")[1];
        const segment = assets.get(id).gameplay.jumpSegments[0];
        const part = parts.find((part) => part.node === `asset:${id}:${segment.node}`);
        assert.ok(part);
        const matrix = partMatrix(document.camera, document, part);
        return [segment.edge.a, segment.edge.b].map((point) => {
          const local = gameToScene(document.camera, ...point);
          const world = sceneToGame(
            document.camera,
            [0, 1, 2].map(
              (row) =>
                matrix[row] * local[0] +
                matrix[4 + row] * local[1] +
                matrix[8 + row] * local[2] +
                matrix[12 + row],
            ),
          );
          return [world[0], world[1] - world[2], world[2]];
        });
      });
      const used = new Set();
      const match = (line) => {
        const candidates = segments
          .map((points, index) => ({
            points,
            index,
            distance: points.reduce(
              (sum, point, endpoint) =>
                sum +
                point.reduce(
                  (sum, value, axis) =>
                    sum + (value - [line.point_a, line.point_b][endpoint][axis]) ** 2,
                  0,
                ),
              0,
            ),
          }))
          .filter((candidate) => !used.has(candidate.index))
          .sort((a, b) => a.distance - b.distance);
        assert.ok(
          candidates[0] && candidates[0].distance < 25,
          "Diagnostic requires an untrimmed matching asset edge",
        );
        used.add(candidates[0].index);
        return candidates[0].points;
      };
      diagnosticEndpoints = compiled.descriptor.asset_geometry.jump_line_pairs.flatMap((pair) => {
        const first = match(pair.line1),
          second = match(pair.line2);
        // Keep the native equal-span rule, removing only integer snapping.
        second[0] = [
          second[1][0] + first[1][0] - first[0][0],
          second[1][1] + first[1][1] - first[0][1],
          second[0][2],
        ];
        return [first, second];
      });
    }
    results.push({
      file,
      rotation,
      height,
      pairs: compiled.descriptor.asset_geometry.jump_line_pairs?.length ?? 0,
      warnings: compiled.warnings,
      diagnostic_jump_endpoints: diagnosticEndpoints,
    });
    await save(false);
  }
await save(true);
