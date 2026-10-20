// Check exported door geometry in each authored lock state, without editing a map.
// Loading separate snapshots does not verify a mission's runtime state trigger.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [source] = process.argv.slice(2);
assert.ok(source, "Supply completed placement diagnostics");
const manifest = JSON.parse(await fs.readFile(path.join(source, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/needs-review/map-compile/authored-door-states-");
let changed = 0;
for (const state of ["initial", "after"]) {
  const directory = path.join(output, state);
  await fs.mkdir(directory);
  const results = [];
  for (const entry of manifest.results) {
    assert.ok(!entry.error);
    const bytes = await fs.readFile(path.join(source, entry.file));
    const descriptor = JSON.parse(bytes);
    const doors = descriptor.asset_geometry.buildings
      .flatMap((building) => building.Building.doors)
      .filter((door) => door.door_type === 1 || door.door_type === 2)
      .sort((a, b) => a.door_type - b.door_type);
    assert.ok(doors.length);
    for (const door of doors) {
      assert.equal(door.active, true);
      assert.equal(door.unlockable, false, "Lockpicking requires a separate actor audit");
      if (state === "after") {
        for (const field of [
          "locked_pc",
          "unlockable",
          "locked_npc_villain",
          "locked_npc_civilian",
        ]) {
          const value = door[`${field}_after_patch`];
          assert.equal(typeof value, "boolean");
          if (door[field] !== value) changed++;
          door[field] = value;
        }
        assert.equal(door.unlockable, false);
      }
    }
    await fs.writeFile(
      path.join(directory, entry.file),
      state === "initial" ? bytes : JSON.stringify(descriptor),
    );
    results.push({
      map: entry.map ?? entry.file,
      file: entry.file,
      building_approaches: doors.map((door, index) => ({
        id: `${entry.file}/door-${index}`,
        door_type: door.door_type,
        point_out: door.point_out,
        sector: door.sector_out,
        layer: door.layer_out,
        expected_authorized: !door.locked_pc,
      })),
    });
  }
  await fs.writeFile(
    path.join(directory, "diagnostics.json"),
    JSON.stringify({
      scope: "authored-lock-state-snapshots-not-runtime-trigger-verification",
      source: path.resolve(source),
      state,
      complete: true,
      results,
    }),
  );
}
assert.ok(changed > 0, "No authored lock changes to test");
console.log(JSON.stringify({ output, changed }));
