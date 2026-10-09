import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import {
  createTerrainGrid,
  parseProjectionAssetDescriptor,
  serializeStoredMap,
  parseStoredMap,
} from "../shared/src/index.ts";
import { insertProjectionAsset } from "../app/src/asset-commands.ts";
import { compileMap } from "../app/src/map-compile.ts";

const [id, stagedDescriptor, baseline, ...extra] = process.argv.slice(2);
assert.ok(
  id && !extra.length,
  "Provide an asset ID, optional staged descriptor and optional baseline directory",
);
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8"));
const entry = index.assets.find((asset) => asset.id === id);
assert.ok(entry, `Unknown asset ${id}`);
const bytes = await fs.readFile(stagedDescriptor ?? `library/3d-assets/${entry.descriptor}`);
const descriptor = parseProjectionAssetDescriptor(JSON.parse(bytes));
assert.equal(descriptor.id, id);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
if (!stagedDescriptor) assert.equal(digest(bytes), entry.descriptor_sha256);
const reference = {
  id,
  descriptor: `3d-assets/${entry.descriptor}`,
  model: `3d-assets/${entry.model}`,
  descriptor_sha256: digest(bytes),
  model_sha256: digest(await fs.readFile(`library/3d-assets/${entry.model}`)),
};
const assets = new Map([[id, descriptor]]);
const expectedMasks = descriptor.gameplay.masks.length * 2;
assert.ok(expectedMasks > 0);
const output = await fs.mkdtemp("work/map-compile/grounded-mask-placements-");
console.log(output);
const results = [];
const report = async () =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      complete: results.length === 8,
      masks_complete:
        results.length === 8 &&
        results.every((r) => r.mask_count === expectedMasks && !r.mask_warnings.length),
      expected_masks_per_map: expectedMasks,
      asset: reference,
      staged_descriptor: stagedDescriptor,
      baseline,
      results,
    }),
  );
await report();
for (const elevation of [0, 40])
  for (const rotation of [0, 37, 90, 180]) {
    let document = {
      version: 1,
      map: `Grounded ${id}`,
      size: [4000, 4000],
      camera: { kind: "oblique-orthographic", elevation_deg: 35 },
      sceneAssets: [],
      groups: [],
      objects: [],
      terrain: createTerrainGrid([0, 0, 4000, 4000], 1000, elevation),
    };
    for (const [x, y] of [
      [1000, 1000],
      [3000, 3000],
    ]) {
      document = insertProjectionAsset(document, descriptor, reference, [x, y, elevation]).document;
      document.groups.at(-1).transform.rot_deg = rotation;
    }
    const stored = serializeStoredMap(document, assets);
    document = parseStoredMap(stored, assets);
    const compiled = compileMap(document, [0, 0, 4000, 4000], assets, { bestEffort: true });
    const file = `grounded-masks-${rotation}-${elevation}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    await fs.writeFile(`${output}/${file}.scene.json`, JSON.stringify(stored));
    const masks = compiled.descriptor.asset_geometry.masks ?? [];
    if (baseline) {
      const before = JSON.parse(await fs.readFile(`${baseline}/${file}`, "utf8"));
      const previous = before.asset_geometry.masks ?? [];
      const remapping = previous.map((mask) => {
        const matches = masks.flatMap((next, index) =>
          isDeepStrictEqual(mask, next) ? [index] : [],
        );
        assert.equal(matches.length, 1, "each existing mask must be preserved exactly once");
        return matches[0];
      });
      const added = masks.filter((_, index) => !remapping.includes(index));
      assert.equal(added.length, 2, "one restored ground mask per independent copy");
      assert.ok(added.every((mask) => mask.layer === 0));
      const [a, b] = added;
      assert.ok(
        [0, 1].some(
          (axis) =>
            a.box_top_left[axis] + a.box_size[axis] <= b.box_top_left[axis] ||
            b.box_top_left[axis] + b.box_size[axis] <= a.box_top_left[axis],
        ),
        "independent copied mask coverage must not overlap",
      );
      for (const transition of before.asset_geometry.movement_transitions ?? [])
        for (const key of ["initial_masks", "applied_masks"])
          if (transition[key]) transition[key] = transition[key].map((index) => remapping[index]);
      const after = structuredClone(compiled.descriptor);
      for (const descriptor of [before, after]) {
        delete descriptor.asset_geometry.masks;
        delete descriptor.asset_geometry.warnings;
      }
      assert.deepEqual(
        after,
        before,
        "mask repair must preserve other gameplay and state bindings",
      );
    }
    results.push({
      file,
      map: file,
      rotation,
      elevation,
      mask_count: masks.length,
      mask_layers: masks.map((mask) => mask.layer),
      mask_warnings: compiled.warnings.filter((warning) => warning.startsWith("Mask ")),
      warnings: compiled.warnings,
    });
    await report();
    console.log(`${rotation}/${elevation}: ${masks.length}/${expectedMasks} masks`);
  }
assert.ok(
  results.every((r) => r.mask_count === expectedMasks && !r.mask_warnings.length),
  `Missing mask bindings; see ${output}/diagnostics.json`,
);
