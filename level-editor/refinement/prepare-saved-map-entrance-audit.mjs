// Prepare doorway-start probes from completed map exports, without changing gameplay.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [source] = process.argv.slice(2);
assert.ok(source, "Supply a completed saved-map export directory");
const manifest = JSON.parse(await fs.readFile(path.join(source, "diagnostics.json"), "utf8"));
assert.equal(manifest.complete, true, "Wait for the export batch to finish");
const output = await fs.mkdtemp("work/needs-review/map-compile/saved-map-entrances-");
const results = [];
const withoutEntrances = [];
for (const entry of manifest.results) {
  assert.ok(!entry.error, `Failed export: ${entry.map}`);
  const bytes = await fs.readFile(path.join(source, entry.file));
  const descriptor = JSON.parse(bytes);
  const doors = (descriptor.asset_geometry.buildings ?? [])
    .flatMap(({ Building }) => Building.doors)
    .filter((door) => door.door_type === 1);
  if (!doors.length) {
    withoutEntrances.push(entry.map);
    continue;
  }
  const building_approaches = doors.map((door, index) => ({
    id: `${entry.map}/building-door-${index}`,
    point_out: door.point_out,
    sector: door.sector_out,
    layer: door.layer_out,
  }));
  await fs.writeFile(path.join(output, entry.file), bytes);
  results.push({ map: entry.map, file: entry.file, building_approaches });
}
assert.ok(results.length, "Export batch has no building entrances");
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    scope: "doorway-start-round-trips-not-terrain-approaches",
    source: path.resolve(source),
    complete: true,
    withoutEntrances,
    results,
  }),
);
console.log(
  JSON.stringify({
    output,
    maps: results.length,
    entrances: results.reduce((n, r) => n + r.building_approaches.length, 0),
    withoutEntrances,
  }),
);
