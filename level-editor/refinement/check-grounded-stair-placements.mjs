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
const overrides = process.argv.slice(2).filter((argument) => argument.startsWith("--descriptor="));
assert.ok(overrides.length <= 1, "Provide at most one staged descriptor");
const stagedDescriptor = overrides[0]?.slice("--descriptor=".length);
const requireMasks = process.argv.includes("--require-masks");
const paddingOptions = process.argv
  .slice(2)
  .filter((argument) => argument.startsWith("--padding="));
assert.ok(paddingOptions.length <= 1, "Provide at most one padding value");
const padding = paddingOptions.length ? Number(paddingOptions[0].slice("--padding=".length)) : 0;
assert.ok(
  Number.isInteger(padding) && padding >= 0 && padding <= 4096,
  "Padding must be an integer from 0 to 4096",
);
const rotationOptions = process.argv
  .slice(2)
  .filter((argument) => argument.startsWith("--rotations="));
assert.ok(rotationOptions.length <= 1, "Provide at most one rotation list");
const rotations = rotationOptions.length
  ? rotationOptions[0].slice("--rotations=".length).split(",").map(Number)
  : [0, 37, 90, 180];
assert.ok(
  rotations.length > 0 &&
    new Set(rotations).size === rotations.length &&
    rotations.every((value) => Number.isFinite(value) && value >= 0 && value < 360),
  "Rotations must be distinct finite angles from 0 to less than 360",
);
assert.ok(
  process.argv
    .slice(2)
    .every(
      (argument) =>
        argument.startsWith("--descriptor=") ||
        argument.startsWith("--padding=") ||
        argument.startsWith("--rotations=") ||
        argument === "--require-masks",
    ),
  "Unknown verification option",
);
const descriptorBytes = await fs.readFile(stagedDescriptor ?? `library/${descriptorPath}`);
const descriptor = parseProjectionAssetDescriptor(JSON.parse(descriptorBytes));
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const reference = {
  id,
  descriptor: descriptorPath,
  model: modelPath,
  descriptor_sha256: digest(descriptorBytes),
  model_sha256: digest(await fs.readFile(`library/${modelPath}`)),
};
if (!stagedDescriptor) assert.equal(reference.descriptor_sha256, entry.descriptor_sha256);
assert.equal(descriptor.id, id);
assert.ok(descriptor.gameplay.placementGroundHeight > 0);
const assets = new Map([[id, descriptor]]);
const bounds = [0, 0, 2000 + padding * 2, 2000 + padding * 2];
const output = await fs.mkdtemp("work/map-compile/grounded-stair-placements-");
console.log(output);
const results = [];
const report = async () =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete: results.length === rotations.length * 2,
      expected_directed_routes: rotations.length * 16,
      rotations,
      padding,
      expected_masks_per_map: requireMasks ? 2 : undefined,
      snapshot_notes: `Fresh editor drops, two independent copies, ${rotations.length} rotations and two terrain elevations. All ${rotations.length * 16} directed stair routes need verification; forbidden routes are not passes.`,
      asset: reference,
      staged_descriptor: stagedDescriptor,
      results,
    }),
  );
await report();
for (const elevation of [0, 40])
  for (const rotation of rotations) {
    let document = {
      version: 1,
      map: "Grounded stair placement",
      size: [bounds[2], bounds[3]],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      sceneAssets: [],
      groups: [],
      objects: [],
      terrain: createTerrainGrid(bounds, 500, elevation),
    };
    for (const [x, y] of [
      [500 + padding, 500 + padding],
      [1450 + padding, 1400 + padding],
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
    if (requireMasks) {
      assert.equal(
        compiled.descriptor.asset_geometry.masks?.length,
        2,
        "each independent tower must retain its mask",
      );
      assert.ok(
        compiled.descriptor.asset_geometry.masks.every((mask) => mask.layer === 0),
        "grounded masks must bind the new terrain layer",
      );
      const [first, second] = compiled.descriptor.asset_geometry.masks;
      assert.ok(first.obstacle_indices.length > 0 && second.obstacle_indices.length > 0);
      assert.ok(
        !first.obstacle_indices.some((index) => second.obstacle_indices.includes(index)),
        "copied masks must retain independent obstacle ownership",
      );
      assert.ok(
        !compiled.warnings.some((warning) => warning.startsWith("Mask ")),
        JSON.stringify(compiled.warnings),
      );
    }
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
