import assert from "node:assert/strict";
import test from "node:test";
import { removeMotionSpikes } from "./remove-motion-spikes.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import type { Point } from "./level.ts";

test("a numeric seam slit cannot divide an otherwise traversable floor", () => {
  const floor: Point[] = [
    [0, 0],
    [50, 0],
    [50, 90],
    [50.000008, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  const snapshot = structuredClone(floor);
  assert.deepEqual(prepareCorridorStates([], floor)([25, 50], [75, 50]), []);
  for (let start = 0; start < floor.length; start++) {
    const rotated = [...floor.slice(start), ...floor.slice(0, start)];
    for (const ring of [rotated, [...rotated].reverse()]) {
      const clean = removeMotionSpikes(ring);
      assert.equal(clean.length, 5);
      assert.deepEqual(prepareCorridorStates([], clean)([25, 50], [75, 50]), [0]);
    }
  }
  assert.deepEqual(floor, snapshot);
});

test("real gaps, fine corners and ordinary fractional geometry remain unchanged", () => {
  const contours: Point[][] = [
    [
      [0, 0],
      [50, 0],
      [50, 90],
      [50.001, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ],
    [
      [0.1, 0.1],
      [10.2, 0.1],
      [10.200005, 0.100005],
      [10.2, 10.3],
      [0.1, 10.3],
    ],
    [
      [0, 0],
      [0.00001, 0],
      [0, 0.00001],
    ],
  ];
  for (const contour of contours) assert.deepEqual(removeMotionSpikes(contour), contour);
  assert.deepEqual(prepareCorridorStates([], contours[0])([25, 50], [75, 50]), []);
});

test("a joined roof retains its outer coordinates when the shared seam backtracks", () => {
  const floor: Point[] = [
    [141, 1447],
    [103, 1403],
    [177.57866382598877, 1355.2696552276611],
    [228.4146203994751, 1414.6605424880981],
    [177.57867050170898, 1355.2696514129639],
    [178, 1355],
    [199, 1341],
    [249, 1414],
    [233, 1420],
    [188, 1437],
  ];
  assert.deepEqual(removeMotionSpikes(floor), [...floor.slice(0, 3), ...floor.slice(5)]);
});
