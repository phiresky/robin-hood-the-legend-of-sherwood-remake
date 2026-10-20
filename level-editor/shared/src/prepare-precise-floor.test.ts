import assert from "node:assert/strict";
import test from "node:test";
import type { Point } from "./level.ts";
import { preparePreciseFloor } from "./prepare-precise-floor.ts";
import { serializePreciseMotionRing } from "./serialize-precise-motion-ring.ts";
import { removeMotionSpikes } from "./remove-motion-spikes.ts";

test("floor cleanup retains the compiled grid boundary", () => {
  const exact: Point[] = [
    [0, 0],
    [50, 0],
    [50, 90],
    [50.000008, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  const grid: Point[] = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  assert.deepEqual(preparePreciseFloor(exact, grid), removeMotionSpikes(exact));
});

test("a slit straddling a grid boundary keeps its original precision contour", () => {
  const exact: Point[] = [
    [0, 0],
    [50.499999, 0],
    [50.5, 90],
    [50.500001, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  const grid = exact.map(([x, y]): Point => [Math.round(x), Math.round(y)]);
  assert.equal(serializePreciseMotionRing(removeMotionSpikes(exact), grid), undefined);
  assert.deepEqual(preparePreciseFloor(exact, grid), exact);
});
