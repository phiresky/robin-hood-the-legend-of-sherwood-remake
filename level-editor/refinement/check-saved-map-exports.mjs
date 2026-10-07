import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";
import { savedMapBakeBounds } from "../app/tests/saved-map-bounds.mjs";
import { savedMapWallCalibration } from "../app/tests/saved-map-wall-calibration.mjs";

// Compile the actual saved scenes, including authored terrain and spline walls.
// Descriptors exercise native geometry; this does not replace a browser ZIP bake.
const arguments_ = process.argv.slice(2);
const selectedMaps = arguments_
  .filter((argument) => argument.startsWith("--map="))
  .map((argument) => argument.slice(6));
const stages = arguments_.filter((argument) => !argument.startsWith("--map="));
assert.ok(stages.length <= 1, "Provide at most one gameplay stage");
const [stage] = stages;
const files = (await fs.readdir("library/scenes"))
  .filter((file) => file.endsWith(".rhlos-map.json"))
  .sort();
for (const map of selectedMaps) {
  assert.ok(files.includes(`${map}.rhlos-map.json`), `Unknown saved map: ${map}`);
}
const output = await fs.mkdtemp("work/map-compile/saved-map-exports-");
const edits = stage ? JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8")) : [];
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      ...(selectedMaps.length ? { selectedMaps } : {}),
      ...(stage ? { gameplayStage: stage } : {}),
      complete,
      results,
    }),
  );
await report(false);
for (const file of files) {
  const map = file.replace(".rhlos-map.json", "");
  if (selectedMaps.length && !selectedMaps.includes(map)) continue;
  const start = performance.now();
  try {
    const document = await readStoredMap(`library/scenes/${file}`, "library");
    const assets = await pinnedDescriptors(
      "library",
      document.assetSources ?? [],
      document.sceneAssets,
    );
    for (const edit of edits) {
      const asset = assets.get(edit.asset);
      if (asset) asset.gameplay = edit.gameplay;
    }
    console.log(`${map}: loaded`);
    const bounds = await savedMapBakeBounds(document);
    const prepared = await savedMapWallCalibration(document, assets);
    const compiled = compileMap(document, bounds, prepared.assets, {
      bestEffort: true,
    });
    await fs.writeFile(`${output}/${map}.level.json`, JSON.stringify(compiled.descriptor));
    results.push({
      map,
      file: `${map}.level.json`,
      warnings: [...prepared.warnings, ...compiled.warnings],
      bounds,
      elapsedMs: performance.now() - start,
    });
    console.log(`${map}: compiled`);
  } catch (error) {
    results.push({ map, error: String(error), stack: error.stack, cause: error.cause });
    console.error(`${map}: ${error}`);
  }
  await report(false);
}
const complete = results.length > 0 && results.every((result) => result.file);
await report(complete);
if (!complete) process.exitCode = 1;
