import fs from "node:fs/promises";
import { readStoredMap, pinnedDescriptors } from "../pipeline/src/stored-map.ts";
import { compileMap } from "../app/src/map-compile.ts";

// Compile the actual saved scenes, including authored terrain and spline walls.
// Descriptors exercise native geometry; this does not replace a browser ZIP bake.
const output = await fs.mkdtemp("work/map-compile/saved-map-exports-");
const [stage] = process.argv.slice(2);
const edits = stage ? JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8")) : [];
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "static-geometry-only-not-gameplay-parity",
      ...(stage ? { gameplayStage: stage } : {}),
      complete,
      results,
    }),
  );
await report(false);
for (const file of (await fs.readdir("library/scenes"))
  .filter((file) => file.endsWith(".rhlos-map.json"))
  .sort()) {
  const map = file.replace(".rhlos-map.json", "");
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
    const compiled = compileMap(
      document,
      document.exportBounds ?? [0, 0, ...document.size],
      assets,
      {
        bestEffort: true,
      },
    );
    await fs.writeFile(`${output}/${map}.level.json`, JSON.stringify(compiled.descriptor));
    results.push({
      map,
      file: `${map}.level.json`,
      warnings: compiled.warnings,
      elapsedMs: performance.now() - start,
    });
    console.log(`${map}: compiled`);
  } catch (error) {
    results.push({ map, error: String(error) });
    console.error(`${map}: ${error}`);
  }
  await report(false);
}
const complete = results.length > 0 && results.every((result) => result.file);
await report(complete);
if (!complete) process.exitCode = 1;
