import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

// Author a shared contact while retaining each flight's measured slope.
// This stages a candidate; mesh and native placement review precede publication.
const [stage] = process.argv.slice(2);
assert.ok(stage);
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const review = JSON.parse(await fs.readFile(`${stage}/review.json`, "utf8"));
assert.equal(edits.length, 1);
assert.equal(edits[0].asset, "york-north-garden-wall-and-stair");
const gameplay = edits[0].gameplay;
const floors = ["building-049-walk-0", "building-050-walk-0"].map((id) => {
  const surface = gameplay.surfaces.find((surface) => surface.id === id);
  assert.ok(surface);
  return surface;
});
const planes = floors.map((floor) =>
  heightPlane(floor.polygon.map(([x, y], i) => [x, y, floor.height[i]])),
);
const difference = planes[0].map((value, i) => value - planes[1][i]);
const normSquared = difference[0] ** 2 + difference[1] ** 2;
assert.ok(normSquared > 1e-8);
const seam = floors[0].polygon.slice(0, 2).map((point) => {
  const t = -planeHeight(difference, point) / normSquared;
  assert.ok(Math.abs(t) * Math.sqrt(normSquared) < 3, "Contact adjustment exceeds review bound");
  return point.map((value, i) => value + t * difference[i]);
});
for (const [flight, corners] of [
  [0, [0, 1]],
  [1, [4, 3]],
]) {
  const floor = floors[flight];
  const clearance = gameplay.movementClearances.find((c) => c.id === `${floor.id}-seam-clearance`);
  assert.ok(clearance);
  for (const contour of [floor, clearance])
    for (const [end, corner] of corners.entries()) {
      review.changes.push({
        id: contour.id,
        corner,
        before: [...contour.polygon[corner]],
        after: [...seam[end]],
        reason: "Shared contact between distinct flight planes",
      });
      contour.polygon[corner] = [...seam[end]];
      contour.height[corner] = planeHeight(planes[flight], seam[end]);
    }
  const lift = gameplay.lifts.find((lift) => lift.surface === floor.id);
  assert.equal(lift.joins.length, 1);
  const middle = seam[0].map((value, i) => (value + seam[1][i]) / 2);
  lift.joins = [[...middle, planeHeight(planes[0], middle)]];
}
gameplay.draft.issues.push(
  "Joined garden stair contact requires mesh, moved-assembly and rendered review before publication.",
);
const output = await fs.mkdtemp("work/map-compile/york-garden-joined-seam-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(`${output}/review.json`, JSON.stringify({ ...review, stage }));
console.log(JSON.stringify({ output, seam }));
