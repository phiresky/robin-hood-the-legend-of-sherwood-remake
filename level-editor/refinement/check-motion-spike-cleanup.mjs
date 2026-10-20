// Stage contour cleanup and rebuild the graph without modifying source assets.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { preparePreciseFloor } from "../shared/src/prepare-precise-floor.ts";
import { compileNavigationGraph } from "../shared/src/compile-navigation-graph.ts";

const inputs = process.argv.slice(2);
assert.ok(inputs.length, "Supply exported level JSON files");
const output = await fs.mkdtemp("work/needs-review/map-compile/motion-spike-cleanup-");
console.log(output);
const results = [];
const report = (complete) =>
  fs.writeFile(
    `${output}/diagnostics.json`,
    JSON.stringify({
      scope: "diagnostic-floor-cleanup-and-graph-rebuild-not-full-compiler-export",
      complete,
      results,
    }),
  );
await report(false);
for (const input of inputs) {
  const bytes = await fs.readFile(input);
  const descriptor = JSON.parse(bytes);
  const motion = descriptor.asset_geometry.motion_data;
  const changes = [];
  for (const [layer, areas] of motion.layers.entries())
    for (const [area, entry] of areas.entries()) {
      if (!entry.precise_polygon) continue;
      const before = entry.precise_polygon;
      const after = preparePreciseFloor(before, entry.polygon.points);
      assert.ok(after, `Existing contour cannot reproduce its grid: ${input}/${layer}/${area}`);
      if (JSON.stringify(before) === JSON.stringify(after)) continue;
      changes.push({ layer, area, before, after });
      entry.precise_polygon = after;
    }
  if (changes.length) motion.graph_bytes = compileNavigationGraph(motion.layers);
  const file = path.basename(input);
  assert.ok(!results.some((entry) => entry.file === file), `Duplicate output name: ${file}`);
  await fs.writeFile(`${output}/${file}`, JSON.stringify(descriptor));
  results.push({
    file,
    input,
    sourceSha256: createHash("sha256").update(bytes).digest("hex"),
    changes,
  });
  await report(false);
  console.log(`${file}: ${changes.length} changed contours`);
}
await report(true);
