// Attach native closed/open/reset probes to existing, unchanged exports.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";

const [input, suffix] = process.argv.slice(2);
assert.ok(input && suffix, "Supply a completed export and asset/control ID suffix");
const manifest = JSON.parse(await fs.readFile(path.join(input, "diagnostics.json")));
assert.equal(manifest.complete, true);
const output = await fs.mkdtemp("work/needs-review/map-compile/passage-control-audit-");
for (const row of manifest.results) {
  const bytes = await fs.readFile(path.join(input, row.file));
  const geometry = JSON.parse(bytes).asset_geometry;
  const buildingDoorCount = (geometry.buildings ?? []).reduce(
    (count, building) => count + Object.values(building)[0].doors.length,
    0,
  );
  row.passage_controls = geometry.movement_transitions.flatMap((transition, index) => {
    if (!transition.id.endsWith(`/${suffix}`)) return [];
    const doors = transition.door_links?.indices;
    assert.ok(doors?.length, `Control ${transition.id} has no linked passages`);
    assert.ok(doors.every((index) => geometry.doors[index - buildingDoorCount]));
    const unlocks =
      transition.door_links.mode === "trigger-transition"
        ? doors.filter((index) => geometry.doors[index - buildingDoorCount].unlockable)
        : [];
    return [
      {
        patches: [{ index, id: transition.id }],
        doors,
        reject_alternatives: true,
        ...(unlocks.length ? { unlock_before_open: unlocks } : {}),
      },
    ];
  });
  assert.ok(row.passage_controls.length, `No matching controls in ${row.file}`);
  await fs.writeFile(path.join(output, row.file), bytes);
}
await fs.writeFile(
  path.join(output, "diagnostics.json"),
  JSON.stringify({ ...manifest, sourceDiagnostics: input, controlSuffix: suffix }),
);
console.log(output);
