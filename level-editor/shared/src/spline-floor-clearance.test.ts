import test from "node:test";
import assert from "node:assert/strict";
import { splineFloorClearance } from "./spline-floor-clearance.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

test("clearance follows the receiving triangle while retaining its authored footprint", () => {
  const source: Vec3[] = [
    [0, 0, 0],
    [10, 0, 0],
    [0, 10, 0],
  ];
  const placed: Vec3[] = [
    [100, 200, 40],
    [112, 202, 43],
    [98, 212, 41],
  ];
  const clearance: Vec3[] = [
    [2, 2, 0],
    [8, 2, 0],
    [8, 8, 0],
    [2, 8, 0],
  ];
  const before = structuredClone({ source, placed, clearance });
  const result = splineFloorClearance(clearance, source, placed);
  assert.ok(result.length >= 3);
  const plane = heightPlane(placed);
  for (const point of result) {
    const [x, y] = point.source;
    assert.ok(x >= 2 && y >= 2 && x + y <= 10 + 1e-10);
    assert.ok(
      Math.abs(planeHeight(plane, [point.placed[0], point.placed[1]]) - point.placed[2]) < 1e-10,
    );
  }
  assert.ok(result.some(({ source: [x, y] }) => x === 2 && y === 2));
  assert.deepEqual({ source, placed, clearance }, before);
  const reversed = splineFloorClearance(clearance, [...source].reverse(), [...placed].reverse());
  assert.equal(reversed.length, result.length);
  assert.deepEqual(
    splineFloorClearance(
      clearance.map(([x, y]) => [x, y, 0.01]),
      source,
      placed,
    ),
    [],
    "A clearance on another source floor must not remove this floor's collision",
  );
  assert.deepEqual(
    splineFloorClearance(
      clearance.map(([x, y, z]) => [x + 20, y, z]),
      source,
      placed,
    ),
    [],
  );
});
