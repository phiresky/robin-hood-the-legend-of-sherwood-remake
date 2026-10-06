import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Stage reviewed waypoints on the narrow flight; outside approaches stay local
// to their platforms and still require normal placed connectivity checks.
const [source] = process.argv.slice(2);
assert.ok(source, "Provide the scaffold ladder seam stage");
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
assert.equal(edit.asset, "york-bridge-square-scaffolded-corner-house");
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === edit.asset);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes);
const gameplay = edit.gameplay;
const lift = gameplay.lifts.find((lift) => lift.id === "building-133-lift");
assert.equal(lift.doors.length, 2);
const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
assert.ok(Array.isArray(floor.height));
const seams = lift.doors.map((door) => {
  const points = floor.polygon.filter((_, i) => Math.abs(floor.height[i] - door.outside[2]) < 1e-5);
  assert.equal(points.length, 2, "Each endpoint needs one complete level seam");
  return points[0].map((value, axis) => (value + points[1][axis]) / 2);
});
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
const low = lift.doors[0].outside[2],
  high = lift.doors[1].outside[2];
assert.ok(high > low);
for (const [index, door] of lift.doors.entries()) {
  const before = structuredClone(door);
  const t = (door.inside[2] - low) / (high - low);
  assert.ok(t > 0 && t < 1);
  door.middle = [...seams[index], door.outside[2]];
  door.inside = [
    ...seams[0].map((value, axis) => value + t * (seams[1][axis] - value)),
    door.inside[2],
  ];
  for (const key of ["inside", "middle"])
    assert.ok(Math.hypot(...door[key].map((value, axis) => value - before[key][axis])) < 2);
  review.changes.push({ centeredDoor: door.id, before, after: structuredClone(door) });
}
gameplay.draft ??= { issues: [] };
gameplay.draft.issues.push(
  "Scaffold ladder seam corrections retain mesh discrepancies up to 0.711 game units on the flight and 0.537 on the upper landing. Rendered actor integration remains unverified.",
);
validateAssetGameplay(gameplay, descriptor);
const output = await fs.mkdtemp("work/map-compile/york-scaffold-ladder-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review, null, 2));
console.log(JSON.stringify({ output }));
