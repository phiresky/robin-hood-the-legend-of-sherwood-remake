import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fixedPolygonBoolean } from "../shared/src/fixed-polygon-boolean.ts";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { validateAssetGameplay } from "../shared/src/asset-gameplay.ts";

// Reviewed landing extensions clear only their own asset's collision on the
// landing plane. Existing floors, holes and unrelated walls remain unchanged.
const [source] = process.argv.slice(2);
assert.ok(source, "Provide a reviewed local stair seam stage");
const edits = JSON.parse(await fs.readFile(`${source}/edits.json`, "utf8"));
const review = JSON.parse(await fs.readFile(`${source}/review.json`, "utf8"));
assert.equal(edits.length, 1);
const edit = edits[0];
const index = JSON.parse(await fs.readFile("library/3d-assets/index.json", "utf8")).assets;
const entry = index.find((entry) => entry.id === edit.asset);
assert.ok(entry);
const bytes = await fs.readFile(`library/3d-assets/${entry.descriptor}`);
assert.equal(createHash("sha256").update(bytes).digest("hex"), edit.descriptorSha256);
const descriptor = JSON.parse(bytes);
const gameplay = edit.gameplay;
const liftFloors = new Set(gameplay.lifts.map((lift) => lift.surface));
const added = [];
for (const change of review.changes) {
  if (!change.surface || liftFloors.has(change.surface)) continue;
  const { before, after } = change;
  assert.ok(before && after);
  const plane = heightPlane(
    after.polygon.map(([x, y], i) => [
      x,
      y,
      Array.isArray(after.height) ? after.height[i] : after.height,
    ]),
  );
  assert.ok(
    before.polygon.every(
      (p, i) =>
        Math.abs(
          planeHeight(plane, p) - (Array.isArray(before.height) ? before.height[i] : before.height),
        ) < 1e-6,
    ),
    "A changed landing plane requires separate collision review",
  );
  const extension = fixedPolygonBoolean(
    "difference",
    [after.polygon, ...(after.holes ?? [])],
    [[before.polygon, ...(before.holes ?? [])]],
  );
  for (const [i, polygon] of extension.entries()) {
    const clearance = {
      id: `${after.id}-landing-extension-${i}`,
      node: after.node,
      polygon: polygon[0].slice(0, -1),
      height: polygon[0].slice(0, -1).map((p) => planeHeight(plane, p)),
      holes: polygon.slice(1).map((r) => r.slice(0, -1)),
    };
    gameplay.movementClearances ??= [];
    assert.ok(!gameplay.movementClearances.some((c) => c.id === clearance.id));
    gameplay.movementClearances.push(clearance);
    added.push(clearance);
  }
}
assert.ok(added.length, "No reviewed landing extensions need clearance");
validateAssetGameplay(gameplay, descriptor);
review.changes.push({ landingExtensionClearances: added });
const output = await fs.mkdtemp("work/map-compile/landing-extension-clearances-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits, null, 2));
await fs.writeFile(`${output}/review.json`, JSON.stringify(review, null, 2));
console.log(JSON.stringify({ output, asset: edit.asset, clearances: added.length }));
