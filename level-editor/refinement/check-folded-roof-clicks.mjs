// Probe real floor positions separately from perpendicular edge-offset audits.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";

const [directory] = process.argv.slice(2);
assert.ok(directory, "Supply a completed folded roof/slab assembly export");
const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/map-compile/folded-supported-clicks-");
const results = [];
for (const entry of manifest.results) {
  const bytes = await fs.readFile(path.join(directory, entry.file));
  const geometry = JSON.parse(bytes).asset_geometry;
  const pairs = geometry.jump_line_pairs
    .map((pair, index) => ({ pair, index }))
    .filter(({ pair }) => {
      const zone = geometry.jump_zones[pair.line2.jump_zone_index];
      return geometry.doors.some(
        (door) => door.sector_out === zone.sector && door.layer_out === zone.layer,
      );
    });
  assert.equal(pairs.length, 2, "Two independent slab jumps with ordinary approach passages");
  const click_probes = pairs.flatMap(({ pair, index }) => {
    const zone = geometry.jump_zones[pair.line2.jump_zone_index];
    const passages = geometry.doors.filter(
      (door) => door.sector_out === zone.sector && door.layer_out === zone.layer,
    );
    assert.ok(passages.length, "Slab must have a prepared ordinary approach passage");
    const a = pair.line2.point_a;
    const b = pair.line2.point_b;
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const length = Math.hypot(dx, dy);
    assert.ok(length > 0);
    const roof = [(a[0] + b[0]) / 2 + (4 * dy) / length, (a[1] + b[1]) / 2 - (4 * dx) / length];
    return passages.flatMap((door) => [
      { line: index * 2, start: door.point_out, goal: roof },
      // The slab's projection is covered by the adjoining roof after folding.
      // Exercise the actual hidden-floor selection path for the return click.
      { line: index * 2 + 1, start: roof, goal: door.point_out, hidden: true },
    ]);
  });
  await fs.writeFile(path.join(output, entry.file), bytes);
  results.push({ ...entry, click_probes });
}
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    complete: true,
    supportedFloorClicks: true,
    source: directory,
    scope: "explicit-passage-to-roof-clicks-not-perpendicular-approach-coverage-or-rendering",
    results,
  }),
);
console.log(output);
