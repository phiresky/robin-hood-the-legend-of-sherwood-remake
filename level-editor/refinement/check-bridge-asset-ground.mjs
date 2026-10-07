// Reuse rotated bridge fixtures with ordinary asset-owned ground instead of
// editor terrain. No terrain generator participates in the resulting compile.
import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { terrainGameplay } from "../shared/src/authored-terrain.ts";
import { compileMap } from "../app/src/map-compile.ts";

const stage = process.argv[2];
assert.ok(stage, "Provide a bridge fixture stage");
const manifest = JSON.parse(await fs.readFile(`${stage}/diagnostics.json`));
assert.equal(manifest.complete, true);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`));
const output = await fs.mkdtemp("work/map-compile/bridge-asset-ground-");
console.log(JSON.stringify({ output }));
const results = [];
const areasForMaterial = (compiled, material) =>
  new Set(
    compiled.descriptor.asset_geometry.sight_obstacles
      .filter(
        (obstacle) =>
          obstacle.default_material === material && Array.isArray(obstacle.projection_area),
      )
      .map((obstacle) => JSON.stringify(obstacle.projection_area)),
  );
for (const row of manifest.results) {
  const document = JSON.parse(await fs.readFile(`${stage}/${row.file}.scene.json`));
  const assets = new Map();
  for (const reference of document.assetSources) {
    const bytes = await fs.readFile(`library/${reference.descriptor}`);
    assert.equal(createHash("sha256").update(bytes).digest("hex"), reference.descriptor_sha256);
    const descriptor = JSON.parse(bytes);
    const edit = edits.find((edit) => edit.asset === descriptor.id);
    assert.ok(edit);
    descriptor.gameplay = edit.gameplay;
    assets.set(descriptor.id, descriptor);
  }
  const ground = terrainGameplay(document);
  assert.ok(ground);
  ground.id = "fixture-ground";
  for (const surface of ground.gameplay.surfaces) surface.acceptsNavigationJoins = true;
  delete document.terrain;
  document.sceneAssets.push({
    resources: [],
    id: ground.id,
    role: "ground",
    model: "generated",
    descriptor: "generated.json",
    model_sha256: "0".repeat(64),
    descriptor_sha256: "0".repeat(64),
  });
  assets.set(ground.id, ground);
  const compile = (scene = document, definitions = assets) =>
    compileMap(scene, [0, 0, 1000, 1000], definitions, { bestEffort: false });
  const compiled = compile();
  const groundAreas = areasForMaterial(compiled, 3);
  const shared = [...areasForMaterial(compiled, 1)].filter((area) => groundAreas.has(area));
  assert.equal(shared.length, 1, "Asset ground and bridge must share navigation");
  for (const delta of [-1, 1]) {
    const shifted = structuredClone(ground);
    for (const surface of shifted.gameplay.surfaces)
      surface.height =
        typeof surface.height === "number"
          ? surface.height + delta
          : surface.height.map((z) => z + delta);
    const definitions = new Map(assets).set(ground.id, shifted);
    const rejected = compile(document, definitions);
    const banks = areasForMaterial(rejected, 3);
    assert.ok(
      [...areasForMaterial(rejected, 1)].every((area) => !banks.has(area)),
      "Mismatched asset-ground height must not connect",
    );
  }
  if (row.banksDisconnectedWithoutBridge) {
    const removed = compile({ ...document, objects: [], groups: [], assetSources: [] });
    assert.equal(
      removed.descriptor.asset_geometry.motion_data.layers.flat().length,
      2,
      "Asset banks alone must remain disconnected",
    );
  }
  await fs.writeFile(`${output}/${row.file}`, JSON.stringify(compiled.descriptor));
  const [sector, layer] = JSON.parse(shared[0]);
  results.push({
    ...row,
    sector,
    layer,
    mismatchedLandingsRejected: 2,
    groundSource: "asset-local",
  });
  console.log(JSON.stringify({ file: row.file, exports: results.length }));
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    complete: true,
    scope: "static-geometry-only-not-gameplay-parity",
    results,
  }),
);
console.log(JSON.stringify({ output, exports: results.length }));
