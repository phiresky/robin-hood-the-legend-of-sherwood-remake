import fs from "node:fs/promises";
import assert from "node:assert/strict";
import { heightPlane, planeHeight } from "../shared/src/gameplay-plane.ts";
import { pointInGameplayPolygon } from "../shared/src/navigation-anchor.ts";

// Authoring candidate only: independent receiver and actor checks precede publication.
const [stage] = process.argv.slice(2);
assert.ok(stage, "Usage: stage-local-passage-seams.mjs STAGE");
const edits = JSON.parse(await fs.readFile(`${stage}/edits.json`, "utf8"));
const review = JSON.parse(await fs.readFile(`${stage}/review.json`, "utf8"));
const changes = [];
const cross = (a, b) => a[0] * b[1] - a[1] * b[0];
const sub = (a, b) => [a[0] - b[0], a[1] - b[1]];
for (const edit of edits) {
  const gameplay = edit.gameplay;
  for (const lift of gameplay.lifts ?? []) {
    if (lift.type !== 1) continue;
    const floor = gameplay.surfaces.find((surface) => surface.id === lift.surface);
    assert.ok(floor && Array.isArray(floor.height));
    const plane = heightPlane(floor.polygon.map((p, i) => [...p, floor.height[i]]));
    const onFloor = (point) =>
      Math.abs(planeHeight(plane, point) - point[2]) < 1e-4 &&
      pointInGameplayPolygon(point, floor.polygon);
    for (const door of gameplay.doors) {
      const inside = onFloor(door.inside),
        outside = onFloor(door.outside);
      if (inside === outside) continue;
      const start = outside ? door.outside : door.inside;
      const end = outside ? door.inside : door.outside;
      const direction = sub(end, start);
      const hits = [];
      for (let i = 0; i < floor.polygon.length; i++) {
        const a = floor.polygon[i],
          b = floor.polygon[(i + 1) % floor.polygon.length];
        const edge = sub(b, a),
          delta = sub(a, start);
        const denominator = cross(direction, edge);
        if (Math.abs(denominator) < 1e-9) continue;
        const t = cross(delta, edge) / denominator;
        const u = cross(delta, direction) / denominator;
        if (t < 0 || t > 1 || u < 0 || u > 1) continue;
        const point = start.slice(0, 2).map((v, axis) => v + direction[axis] * t);
        if (!hits.some((hit) => Math.hypot(...sub(hit, point)) < 1e-6)) hits.push(point);
      }
      assert.equal(hits.length, 1, `${edit.asset}/${door.id}: ambiguous stair boundary crossing`);
      const point = hits[0];
      const middle = [...point, planeHeight(plane, point)];
      const shift = Math.hypot(...middle.map((v, i) => v - door.middle[i]));
      assert.ok(
        shift <= 15,
        `${edit.asset}/${door.id}: midpoint shift ${shift} needs manual review`,
      );
      changes.push({
        asset: edit.asset,
        door: door.id,
        lift: lift.id,
        before: door.middle,
        after: middle,
        shift,
        note: "Passage midpoint on the stair boundary; receiving floor and node frame require independent placement checks",
      });
      door.middle = middle;
    }
  }
}
assert.ok(changes.length, "No ordinary passage stair crossings found");
const output = await fs.mkdtemp("work/map-compile/local-passage-seams-");
await fs.writeFile(`${output}/edits.json`, JSON.stringify(edits));
await fs.writeFile(
  `${output}/review.json`,
  JSON.stringify({ ...review, changes: [...review.changes, ...changes] }),
);
await fs.writeFile(
  `${output}/passage-review.json`,
  JSON.stringify({ input: stage, changes }, null, 2),
);
console.log(JSON.stringify({ output, changes }));
