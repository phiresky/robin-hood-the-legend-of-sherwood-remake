import test from "node:test";
import assert from "node:assert/strict";
import polygonClipping, { type MultiPolygon, type Pair } from "polygon-clipping";
import captured from "../test-fixtures/near-coincident-collision-difference.json" with { type: "json" };
import { subtractMovementCollision } from "./subtract-movement-collision.ts";

const geometry = (input: number[][][][]): MultiPolygon =>
  input.map((polygon) =>
    polygon.map((ring) =>
      ring.map((point): Pair => {
        const [x, y] = point;
        assert.equal(point.length, 2);
        assert.ok(x !== undefined && y !== undefined);
        return [x, y];
      }),
    ),
  );

function containsRing(ring: Pair[], x: number, y: number) {
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const a = ring[i]!,
      b = ring[j]!;
    if (a[1] > y !== b[1] > y && x < ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]) + a[0])
      inside = !inside;
  }
  return inside;
}
const contains = (shape: MultiPolygon, x: number, y: number) =>
  shape.some(
    (polygon) =>
      containsRing(polygon[0]!, x, y) && !polygon.slice(1).some((hole) => containsRing(hole, x, y)),
  );

test("ordinary collision subtraction preserves existing output", () => {
  const floor: MultiPolygon = [
    [
      [
        [0, 0],
        [100, 0],
        [100, 100],
        [0, 100],
        [0, 0],
      ],
    ],
  ];
  const wall: MultiPolygon = [
    [
      [
        [30, 20],
        [40, 20],
        [40, 80],
        [30, 80],
        [30, 20],
      ],
    ],
  ];
  assert.deepEqual(subtractMovementCollision(floor, wall), polygonClipping.difference(floor, wall));
});

test("near-coincident collision cuts retain free space and existing holes", () => {
  const [rawSubject, rawCuts] = captured;
  assert.ok(rawSubject && rawCuts);
  const subject = geometry(rawSubject),
    cuts = geometry(rawCuts);
  const result = subtractMovementCollision(subject, cuts);
  let retained = 0,
    removed = 0,
    holes = 0;
  for (let x = 325.03125; x < 380; x += 0.125) {
    for (let y = 425.046875; y < 470; y += 0.125) {
      const wasFree = contains(subject, x, y);
      const cut = contains(cuts, x, y);
      assert.equal(contains(result, x, y), wasFree && !cut, `collision occupancy at ${x}, ${y}`);
      if (wasFree && !cut) retained++;
      if (wasFree && cut) removed++;
      if (subject.some((polygon) => polygon.slice(1).some((hole) => containsRing(hole, x, y))))
        holes++;
    }
  }
  assert.ok(retained > 100 && removed > 0 && holes > 0, "probe set must exercise all three cases");
});
