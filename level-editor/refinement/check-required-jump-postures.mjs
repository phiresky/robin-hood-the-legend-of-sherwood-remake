// Isolate helper-required vertical pairs for the native posture-aware harness.
// Preserve physical geometry; record original line and zone numbers for comparison.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [directory] = process.argv.slice(2);
assert.ok(directory, "Supply a completed saved-map export directory");
const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/map-compile/required-jump-postures-");
const results = [];
for (const result of manifest.results) {
  const descriptor = JSON.parse(await fs.readFile(path.join(directory, result.file)));
  const geometry = descriptor.asset_geometry;
  const originalPairs = geometry.jump_line_pairs ?? [];
  const selected = originalPairs.flatMap((pair, index) => {
    if (pair.jump_long) return [];
    const lines = [pair.line1, pair.line2];
    const helpers = lines.map((line) => geometry.jump_zones[line.jump_zone_index].helper_needed);
    if (!helpers.some(Boolean)) return [];
    const heights = lines.map((line) => line.point_a[2]);
    assert.ok(Math.abs(heights[0] - heights[1]) >= 60);
    // A line references the destination zone, where its helper rule is stored.
    assert.equal(helpers[0], heights[1] > heights[0]);
    assert.equal(helpers[1], heights[0] > heights[1]);
    return [{ pair, index }];
  });
  if (!selected.length) continue;
  geometry.jump_line_pairs = selected.map(({ pair }) => pair);
  const originalZones = [
    ...new Set(
      selected.flatMap(({ pair }) => [pair.line1.jump_zone_index, pair.line2.jump_zone_index]),
    ),
  ];
  geometry.jump_zones = originalZones.map((index) => geometry.jump_zones[index]);
  for (const { pair } of selected)
    for (const line of [pair.line1, pair.line2])
      line.jump_zone_index = originalZones.indexOf(line.jump_zone_index);
  await fs.writeFile(path.join(output, result.file), JSON.stringify(descriptor));
  results.push({
    file: result.file,
    approach_depth: result.approach_depth ?? 8,
    original_lines: selected.flatMap(({ index }) => [index * 2, index * 2 + 1]),
    original_zones: originalZones,
  });
}
assert.ok(results.length, "No helper-required vertical pairs in these exports");
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    complete: true,
    scope: "selected-helper-jump-pairs-not-full-map-parity",
    source: directory,
    results,
  }),
);
console.log(output);
