// Isolate the prepared ordinary passages while retaining all physical geometry.
import fs from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";

const [directory] = process.argv.slice(2);
assert.ok(directory, "Supply a completed two-copy, two-elevation folded-floor export");
const manifest = JSON.parse(await fs.readFile(path.join(directory, "diagnostics.json")));
assert.equal(manifest.complete, true);
assert.equal(manifest.results.length, 2);
const output = await fs.mkdtemp("work/needs-review/map-compile/folded-floor-passages-");
const results = [];
for (const entry of manifest.results) {
  const descriptor = JSON.parse(await fs.readFile(path.join(directory, entry.file)));
  const geometry = descriptor.asset_geometry;
  const doors = geometry.doors;
  assert.equal(
    new Set(doors.map((door) => `${door.sector_out}/${door.sector_in}`)).size,
    2,
    "Two independent floor pairs; preserve every passage across each pair",
  );
  assert.ok(doors.every((door) => door.world_endpoints));
  geometry.jump_line_pairs = [];
  geometry.jump_zones = [];
  await fs.writeFile(path.join(output, entry.file), JSON.stringify(descriptor));
  results.push({
    file: entry.file,
    rotation: entry.rotation,
    height: entry.height,
    probes: doors.map((door) => [door.point_out, door.point_in]),
    probe_layers: doors.map((door) => [door.layer_out, door.layer_in]),
    world_probes: doors.map((door) => [door.world_endpoints.outside, door.world_endpoints.inside]),
    world_floors: doors.map((door) =>
      geometry.sight_obstacles
        .filter(
          (obstacle) =>
            Array.isArray(obstacle.projection_area) &&
            ((obstacle.projection_area[0] === door.sector_out &&
              obstacle.projection_area[1] === door.layer_out) ||
              (obstacle.projection_area[0] === door.sector_in &&
                obstacle.projection_area[1] === door.layer_in)),
        )
        .map((obstacle) => obstacle.points.map((point) => [point.x, point.y])),
    ),
  });
}
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({
    complete: true,
    foldedFloor: true,
    expectedWalks: results.reduce((sum, entry) => sum + entry.probes.length * 2, 0),
    scope:
      "prepared-folded-passage-traversal-with-explicit-floor-selection-not-jump-overlays-or-rendering",
    source: directory,
    results,
  }),
);
console.log(output);
