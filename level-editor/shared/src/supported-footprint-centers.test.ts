import test from "node:test";
import assert from "node:assert/strict";
import clipping, { type Polygon, type MultiPolygon } from "polygon-clipping";
import { supportedFootprintCenters } from "./supported-footprint-centers.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";

const box = (left: number, top: number, right: number, bottom: number): Polygon => [
  [
    [left, top],
    [right, top],
    [right, bottom],
    [left, bottom],
  ],
];
const contains = (regions: MultiPolygon, point: Point) =>
  regions.some(
    (polygon) =>
      pointInGameplayPolygon(point, polygon[0]!, true) &&
      polygon.slice(1).every((hole) => !pointInGameplayPolygon(point, hole, true)),
  );

test("a short stair receives footprint support from both joined landings", () => {
  const stair = box(0, 0, 4, 20);
  const low = box(-20, 0, 0, 20),
    high = box(4, 0, 24, 20);
  assert.deepEqual(supportedFootprintCenters([stair], [stair], [5, 2]), []);
  const joined = supportedFootprintCenters([stair], [stair, low, high], [5, 2]);
  assert.ok(contains(joined, [2, 10]));
  assert.ok(contains(joined, [0.1, 3]));
  assert.ok(contains(joined, [3.9, 17]));
  assert.ok(!contains(joined, [2, 1]));
  assert.ok(!contains(joined, [-1, 10]), "centers must remain on the stair");
  for (const supports of [
    [stair, low],
    [stair, high],
    [stair, low, box(5, 0, 25, 20)],
  ])
    assert.deepEqual(supportedFootprintCenters([stair], supports, [5, 2]), []);
});

test("holes inside the footprint stay unsupported even when all corners have support", () => {
  const floor = box(-20, -20, 20, 20);
  floor.push(box(1, -0.5, 2, 0.5)[0]!);
  const result = supportedFootprintCenters([box(-10, -10, 10, 10)], [floor], [5, 2]);
  assert.ok(!contains(result, [0, 0]));
  assert.ok(contains(result, [-7, 0]));
  assert.ok(contains(result, [0, 4]));
});

test("support domains agree with whole-box clipping on concave and disconnected floors", () => {
  const supports: Polygon[] = [
    [
      [
        [-10, -10],
        [10, -10],
        [10, -2],
        [2, -2],
        [2, 10],
        [-10, 10],
      ],
    ],
    box(20, -10, 30, 10),
  ];
  const result = supportedFootprintCenters([box(-20, -20, 40, 20)], supports, [3, 2]);
  const joined = clipping.union(supports[0]!, ...supports.slice(1));
  let checked = 0;
  for (let x = -15.25; x <= 35; x += 0.5)
    for (let y = -15.25; y <= 15; y += 0.5) {
      const expected = clipping.difference(box(x - 3, y - 2, x + 3, y + 2), joined).length === 0;
      assert.equal(contains(result, [x, y]), expected, `support at ${x},${y}`);
      checked++;
    }
  assert.ok(checked > 6000);
});

test("support extents must be positive and finite", () => {
  for (const half of [
    [0, 2],
    [-1, 2],
    [1, Infinity],
  ] satisfies Point[])
    assert.throws(() => supportedFootprintCenters([], [], half), /half extents/);
});
