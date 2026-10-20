// Preserve click coordinates and selection mode when comparing compiler revisions.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [baseline, replacement, selectedLines] = process.argv.slice(2);
assert.ok(baseline && replacement, "Supply baseline probe directory and replacement descriptor");
const lines = selectedLines?.split(",").map(Number);
assert.ok(!lines || lines.every((line) => Number.isInteger(line) && line >= 0));
const manifest = JSON.parse(await fs.readFile(path.join(baseline, "diagnostics.json")));
assert.equal(manifest.complete, true);
assert.equal(manifest.supportedFloorClicks, true);
assert.equal(manifest.results.length, 1, "Replay one descriptor at a time");
const entry = manifest.results[0];
const oldMap = JSON.parse(await fs.readFile(path.join(baseline, entry.file)));
const bytes = await fs.readFile(replacement);
const newMap = JSON.parse(bytes);
const probes = entry.click_probes.filter((probe) => !lines || lines.includes(probe.line));
assert.ok(probes.length, "No selected probes");
for (const probe of probes) {
  const pair = Math.floor(probe.line / 2);
  assert.deepEqual(
    newMap.asset_geometry.jump_line_pairs[pair],
    oldMap.asset_geometry.jump_line_pairs[pair],
    `Jump line ${probe.line} changed identity; review mapping before replay`,
  );
}
const output = await fs.mkdtemp("work/needs-review/map-compile/replayed-jump-clicks-");
const file = path.basename(replacement);
await fs.writeFile(path.join(output, file), bytes);
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    complete: true,
    supportedFloorClicks: true,
    scope: "unchanged-click-coordinates-and-selection-modes",
    baseline,
    replacement,
    results: [{ file, click_probes: probes }],
  }),
);
console.log(output);
