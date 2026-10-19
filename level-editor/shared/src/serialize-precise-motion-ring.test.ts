import test from "node:test";
import assert from "node:assert/strict";
import { serializePreciseMotionRing } from "./serialize-precise-motion-ring.ts";
import type { Point } from "./level.ts";

test("a narrow clipped corner keeps its geometry with a reproducible grid contour", () => {
  const exact: Point[] = [
    [488.8108050793753, 1470.6926004001627],
    [495.012160184201, 1475.3751474188282],
    [493.548828125, 1474.4505081176758],
    [491.2265625, 1472.7727737426758],
    [489.0654296875, 1470.9339065551758],
  ];
  const grid: Point[] = [
    [489, 1471],
    [494, 1474],
    [491, 1473],
  ];
  assert.deepEqual(serializePreciseMotionRing(exact, grid), [
    ...exact.slice(3),
    ...exact.slice(0, 3),
  ]);
  assert.equal(
    serializePreciseMotionRing(exact, [
      [0, 0],
      [10, 0],
      [0, 10],
    ]),
    undefined,
  );
});

test("ordinary precision contours keep their existing order", () => {
  const exact: Point[] = [
    [0.1, 0.1],
    [10.1, 0.1],
    [0.1, 10.1],
  ];
  assert.deepEqual(
    serializePreciseMotionRing(exact, [
      [0, 0],
      [10, 0],
      [0, 10],
    ]),
    exact,
  );
});
