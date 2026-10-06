import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";

// Author a shared contact while retaining separate flight slopes and landings.
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
// Nearly parallel flights amplify a tiny height discrepancy into a large
// footprint extension. Fit the short upper flight through the lower contact
// and its existing upper landing midpoint instead.
const top = floors[1].polygon.slice(0, 3);
const topHeight = floors[1].height[0];
const topMiddle = top[0].map((value, i) => (value + top[2][i]) / 2);
const previousUpperPlane = [...planes[1]];
planes[1] = heightPlane([
  ...floors[0].polygon.slice(0, 2).map((point) => [...point, planeHeight(planes[0], point)]),
  [...topMiddle, topHeight],
]);
const maximumHeightAdjustment = Math.max(
  ...floors[1].polygon.map((point) =>
    Math.abs(planeHeight(planes[1], point) - planeHeight(previousUpperPlane, point)),
  ),
);
assert.ok(maximumHeightAdjustment < 0.1, "Upper flight height fit exceeds review bound");
review.changes.push({
  id: floors[1].id,
  previousUpperPlane,
  fittedPlane: planes[1],
  maximumHeightAdjustment,
  reason: "Fit upper flight to shared seam and landing",
});
for (const before of top) {
  const plane = planes[1];
  const t = (topHeight - planeHeight(plane, before)) / (plane[0] ** 2 + plane[1] ** 2);
  const after = before.map((value, i) => value + t * plane[i]);
  assert.ok(Math.hypot(...after.map((value, i) => value - before[i])) < 0.5);
  for (const contour of [...gameplay.surfaces, ...gameplay.movementClearances]) {
    for (const [i, point] of contour.polygon.entries()) {
      if (Math.hypot(...point.map((value, axis) => value - before[axis])) > 1e-6) continue;
      contour.polygon[i] = [...after];
      review.changes.push({
        id: contour.id,
        corner: i,
        before: [...before],
        after: [...after],
        reason: "Retain upper landing contact after fitting joined flight",
      });
    }
  }
}
for (const contour of [
  floors[1],
  gameplay.movementClearances.find((c) => c.id === `${floors[1].id}-seam-clearance`),
]) {
  contour.height = contour.polygon.map((point) => planeHeight(planes[1], point));
}
for (const points of [
  floors[1].projectionMaterials.planePoints,
  floors[1].projectionMaterials.footprint,
]) {
  for (const point of points) point[2] = planeHeight(planes[1], point);
}
const upperDoor = gameplay.lifts.find((lift) => lift.surface === floors[1].id).doors[0];
upperDoor.inside[2] = planeHeight(planes[1], upperDoor.inside);
const plane = planes[1];
const shift =
  (upperDoor.middle[2] - planeHeight(plane, upperDoor.middle)) / (plane[0] ** 2 + plane[1] ** 2);
upperDoor.middle[0] += shift * plane[0];
upperDoor.middle[1] += shift * plane[1];
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
