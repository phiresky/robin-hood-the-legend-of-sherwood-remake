import test from "node:test";
import assert from "node:assert/strict";
import { ribbonParameterRange, type VerticalFlightRibbon } from "./vertical-jump-clearance.ts";
import type { Vec3 } from "./scene.ts";

test("clearance finds an obstruction enclosed inside a non-planar movement envelope", () => {
  const points: VerticalFlightRibbon["points"] = [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ];
  // This cube sits wholly inside the tetrahedron: checking just the boundary
  // faces or treating its four corners as one flat polygon misses the overlap.
  const planes = [0, 1, 2].flatMap((axis) => [
    (p: Vec3) => p[axis]! - 0.2,
    (p: Vec3) => 0.3 - p[axis]!,
  ]);
  const range = ribbonParameterRange(points, planes)!;
  assert.ok(Math.abs(range[0] - 0.4) < 1e-8);
  assert.ok(Math.abs(range[1] - 0.6) < 1e-8);
  assert.equal(ribbonParameterRange(points, [(p) => p[0] - 2]), undefined);
  assert.deepEqual(ribbonParameterRange(points, []), [0, 1]);
});

test("clearance retains parameter limits for planar and collapsed movement envelopes", () => {
  for (const points of [
    [
      [0, 0, 0],
      [10, 0, 0],
      [10, 10, 0],
      [0, 10, 0],
    ],
    [
      [0, 0, 0],
      [0, 0, 0],
      [0, 10, 0],
      [0, 10, 0],
    ],
  ] satisfies VerticalFlightRibbon["points"][]) {
    assert.deepEqual(ribbonParameterRange(points, [(p) => p[1] - 2, (p) => 4 - p[1]]), [0.2, 0.4]);
  }
});
