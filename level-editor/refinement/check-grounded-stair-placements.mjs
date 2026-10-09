import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  createTerrainGrid,
  parseProjectionAssetDescriptor,
  serializeStoredMap,
  parseStoredMap,
} from "../shared/src/index.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Exercise real editor insertion using only a reusable library asset and new terrain.
// This generates inputs for exported_stairs_support_complete_sprite_actor_routes.
// Retained entrances are not proof of traversal: inspect forbidden routes as well
// as failures in the native actor-stair-sprite-route-report.json output.
const id = "leicester-church-side-tower";
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const entry = index.assets.find((asset) => asset.id === id);
assert.ok(entry);
const descriptorPath = `3d-assets/${entry.descriptor}`;
const modelPath = `3d-assets/${entry.model}`;
const descriptorBytes = await fs.readFile(`library/${descriptorPath}`);
const descriptor = parseProjectionAssetDescriptor(JSON.parse(descriptorBytes));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const reference = {
  id,
  descriptor: descriptorPath,
  model: modelPath,
  descriptor_sha256: digest(descriptorBytes),
  model_sha256: digest(await fs.readFile(`library/${modelPath}`)),
};
assert.equal(reference.descriptor_sha256, entry.descriptor_sha256);
assert.ok(descriptor.gameplay.placementGroundHeight > 0);
const assets = new Map([[id, descriptor]]);
const bounds = [0, 0, 2000, 2000];
const output = await fs.mkdtemp("work/map-compile/grounded-stair-placements-");
console.log(output);
const results = [];
const report = async () =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete: results.length === 8,
      expected_directed_routes: 64,
      snapshot_notes:
        "Fresh editor drops, two independent copies, four rotations and two terrain elevations. All 64 directed stair routes need verification; forbidden routes are not passes.",
      asset: reference,
      results,
    }),
  );
await report();
for (const elevation of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: "Grounded stair placement",
      size: [2000, 2000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      sceneAssets: [],
      groups: [],
      objects: [],
      terrain: createTerrainGrid(bounds, 500, elevation),
    };
    for (const [x, y] of [
      [500, 500],
      [1450, 1400],
    ]) {
      document = insertProjectionAsset(document, descriptor, reference, [x, y, elevation]).document;
      assert.ok(
        Math.abs(
          document.groups.at(-1).transform.dz +
            descriptor.gameplay.placementGroundHeight -
            elevation,
        ) < 1e-6,
        "new drops must ground the lower approach at the requested terrain elevation",
      );
      document.groups.at(-1).transform.rot_deg = rotation;
    }
    const stored = serializeStoredMap(document, assets);
    document = parseStoredMap(stored, assets);
    const compiled = compileMap(document, bounds, assets, { bestEffort: true });
    assert.equal(compiled.descriptor.asset_geometry.lifts?.length, 4);
    assert.equal(
      compiled.descriptor.asset_geometry.lifts.reduce((sum, lift) => sum + lift.doors.length, 0),
      8,
      `both stairs on both copies must retain their entrances: ${JSON.stringify(compiled.warnings)}`,
    );
    const file = `grounded-stairs-${rotation}-${elevation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({ file, map: file, rotation, elevation, warnings: compiled.warnings });
    await report();
  }
