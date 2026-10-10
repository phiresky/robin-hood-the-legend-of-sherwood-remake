import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import {
  createTerrainGrid,
  parseProjectionAssetDescriptor,
  serializeStoredMap,
  parseStoredMap,
} from "../shared/src/index.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap, packageCompiledMap } from "../app/src/map-compile.ts";

const { unzipSync, strFromU8 } = createRequire(new URL("../app/package.json", import.meta.url))(
  "fflate",
);

// Exercise real editor insertion using only a reusable library asset and new terrain.
// This generates inputs for exported_stairs_support_complete_sprite_actor_routes.
// Retained entrances are not proof of traversal: inspect forbidden routes as well
// as failures in the native actor-stair-sprite-route-report.json output.
const assetOptions = process.argv.slice(2).filter((argument) => argument.startsWith("--asset="));
assert.ok(assetOptions.length <= 1, "Provide at most one asset ID");
const id = assetOptions[0]?.slice("--asset=".length) ?? "leicester-church-side-tower";
const { stdout } = await promisify(execFile)("uv", [
  "run",
  "--script",
  fileURLToPath(new URL("./read_asset_index_entry.py", import.meta.url)),
  "library/3d-assets/index.json",
  id,
]);
const entry = JSON.parse(stdout);
assert.equal(entry.id, id);
const descriptorPath = `3d-assets/${entry.descriptor}`;
const modelPath = `3d-assets/${entry.model}`;
const overrides = process.argv.slice(2).filter((argument) => argument.startsWith("--descriptor="));
assert.ok(overrides.length <= 1, "Provide at most one staged descriptor");
const stagedDescriptor = overrides[0]?.slice("--descriptor=".length);
const requireMasks = process.argv.includes("--require-masks");
const zipRoundtrip = process.argv.includes("--zip-roundtrip");
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
        argument.startsWith("--asset=") ||
        argument.startsWith("--descriptor=") ||
        argument.startsWith("--padding=") ||
        argument.startsWith("--rotations=") ||
        argument === "--require-masks" ||
        argument === "--zip-roundtrip",
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
  resources: descriptor.resources ?? [],
  ...(entry.model_scene ? { model_scene: entry.model_scene } : {}),
};
if (!stagedDescriptor) assert.equal(reference.descriptor_sha256, entry.descriptor_sha256);
assert.equal(descriptor.id, id);
const lifts = descriptor.gameplay.lifts ?? [];
const routesPerCopy = lifts
  .filter((lift) => lift.type === 1)
  .reduce((count, lift) => count + lift.doors.length * (lift.doors.length - 1), 0);
assert.ok(routesPerCopy > 0, "The asset must contain at least one connected stair");
const expectedRoutes = rotations.length * 2 * 2 * routesPerCopy;
assert.ok(
  !requireMasks || id === "leicester-church-side-tower",
  "The ground-mask ownership check currently describes the church-side tower only",
);
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
      expected_directed_routes: expectedRoutes,
      rotations,
      padding,
      zip_roundtrip: zipRoundtrip,
      raster_scope: zipRoundtrip ? "blank diagnostic images, not rendering coverage" : undefined,
      expected_masks_per_map: requireMasks ? 2 : undefined,
      snapshot_notes: `Fresh editor drops of ${id}, two independent copies, ${rotations.length} rotations and two terrain elevations. All ${expectedRoutes} directed stair routes need verification; forbidden routes are not passes.`,
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
      if (descriptor.gameplay.placementGroundHeight !== undefined)
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
    assert.equal(
      compiled.descriptor.asset_geometry.lifts?.length,
      lifts.length * 2,
      `both copies must retain every authored lift: ${JSON.stringify(compiled.warnings)}`,
    );
    assert.equal(
      compiled.descriptor.asset_geometry.lifts.reduce((sum, lift) => sum + lift.doors.length, 0),
      lifts.reduce((count, lift) => count + lift.doors.length, 0) * 2,
      `both copies must retain their entrances: ${JSON.stringify(compiled.warnings)}`,
    );
    const file = `grounded-stairs-${rotation}-${elevation}.level.json`;
    let runtimeDescriptor = compiled.descriptor;
    let packagedPreciseLifts;
    if (zipRoundtrip) {
      // Diagnostic rasters exercise packaging only; native traversal consumes
      // the descriptor extracted from the ZIP, not the pre-package object.
      const bytes = await packageCompiledMap(compiled, {
        color: new Uint8Array(bounds[2] * bounds[3] * 4),
        depth: new Uint16Array(bounds[2] * bounds[3]),
      });
      const files = unzipSync(bytes);
      const readJson = (path) => {
        assert.ok(files[path], `Missing ZIP entry ${path}`);
        return JSON.parse(strFromU8(files[path]));
      };
      runtimeDescriptor = readJson(`Data/Levels/${compiled.name}.level.json`);
      assert.deepEqual(runtimeDescriptor, JSON.parse(JSON.stringify(compiled.descriptor)));
      const report = readJson("compile-report.json");
      assert.deepEqual(report.warnings, compiled.warnings);
      const reopened = parseStoredMap(readJson(report.editor_document), assets);
      assert.deepEqual(serializeStoredMap(reopened, assets), stored);
      const recompiled = compileMap(reopened, bounds, assets, { bestEffort: true });
      assert.deepEqual(JSON.parse(JSON.stringify(recompiled.descriptor)), runtimeDescriptor);
      assert.deepEqual(recompiled.warnings, compiled.warnings);
      packagedPreciseLifts = runtimeDescriptor.asset_geometry.motion_data.layers
        .flat()
        .filter((area) => area.is_lift && area.precise_polygon?.length).length;
      await fs.writeFile(`${output}/${file}.zip`, bytes);
    }
    await fs.writeFile(`${output}/${file}`, JSON.stringify(runtimeDescriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    results.push({
      file,
      map: file,
      rotation,
      elevation,
      packaged_precise_lifts: packagedPreciseLifts,
      warnings: compiled.warnings,
    });
    await report();
  }
