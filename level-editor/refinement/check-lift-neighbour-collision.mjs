import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { transformedObstacle } from "../shared/src/level3d.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Diagnostic subsets identify collisions that disconnect an otherwise usable
// asset pair. They are not substitutes for full-scene or actor route checks.
const [stage, map] = process.argv.slice(2);
assert.ok(stage && map, "Provide staged asset edits and a saved map name");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const source = await readStoredMap(`library/scenes/${map}.rhlos-map.json`, "library");
const assets = await pinnedDescriptors("library", source.assetSources, source.sceneAssets);
for (const edit of edits) assets.get(edit.asset).gameplay = edit.gameplay;
const primary = edits[0].asset;
const baseline = edits.map((edit) => edit.asset);
const bounds = (points) => [
  Math.min(...points.map((point) => point.x)),
  Math.min(...points.map((point) => point.y)),
  Math.max(...points.map((point) => point.x)),
  Math.max(...points.map((point) => point.y)),
];
const boxes = new Map();
for (const group of source.groups) {
  const points = source.objects
    .filter((object) => object.group === group.id && object.obstacle)
    .flatMap((object) => transformedObstacle(source, object).points);
  if (points.length) boxes.set(group.id, bounds(points));
}
const box = boxes.get(primary);
assert.ok(box, "Primary asset needs placed collision geometry");
const nearby = [...boxes]
  .filter(
    ([id, b]) =>
      !baseline.includes(id) &&
      b[0] < box[2] + 50 &&
      b[2] > box[0] - 50 &&
      b[1] < box[3] + 50 &&
      b[3] > box[1] - 50,
  )
  .map(([id]) => id);
const frame = [
  Math.max(0, box[0] - 200),
  Math.max(0, box[1] - 500),
  Math.min(source.size[0], box[2] + 200),
  Math.min(source.size[1], box[3] + 200),
];
const output = await fs.mkdtemp("work/map-compile/lift-neighbour-collision-");
const results = [];
console.log(JSON.stringify({ output, primary, nearby }));
for (const included of [[], ...nearby.map((id) => [id]), nearby]) {
  const ids = new Set([...baseline, ...included]);
  const document = {
    ...source,
    groups: source.groups.filter((group) => ids.has(group.id)),
    objects: source.objects.filter((object) => ids.has(object.group)),
  };
  let result;
  try {
    const compiled = compileMap(document, frame, assets, { bestEffort: true });
    const warnings = compiled.warnings.filter(
      (warning) => warning.startsWith("Lift ") && warning.includes(primary),
    );
    const file = `subset-${results.length}.level.json`;
    await fs.writeFile(`${output}/${file}`, JSON.stringify(compiled.descriptor));
    result = {
      included,
      file,
      totalLifts: compiled.descriptor.asset_geometry.lifts?.length ?? 0,
      warnings,
    };
  } catch (error) {
    result = { included, error: String(error) };
  }
  results.push(result);
  console.log(JSON.stringify(result));
  await fs.writeFile(`${output}/results.json`, JSON.stringify(results));
}
await fs.writeFile(
  `${output}/diagnostics.json`,
  JSON.stringify({
    scope: "collision-subsets-only-not-full-map-parity",
    complete: !results.some((result) => result.error),
    results: results
      .filter((result) => result.file)
      .map((result) => ({ ...result, map: result.file })),
  }),
);
// Actor checks can inspect the successfully compiled subsets independently.
// Failed subsets remain explicit in the parent report and are never certified.
await fs.mkdir(`${output}/compiled-subsets`);
await fs.writeFile(
  `${output}/compiled-subsets/diagnostics.json`,
  JSON.stringify({
    scope: "successfully-compiled-collision-subsets-only",
    complete: true,
    excluded: results.filter((result) => result.error),
    results: results
      .filter((result) => result.file)
      .map((result) => ({
        ...result,
        file: `../${result.file}`,
        map: result.file,
      })),
  }),
);
