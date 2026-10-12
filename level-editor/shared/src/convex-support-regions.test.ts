import test from "node:test";
import assert from "node:assert/strict";
import clipping from "polygon-clipping";
import type { Point } from "./level.ts";
import { convexSupportRegions } from "./convex-support-regions.ts";
import { stairBoundaryPlanes, stairCenterRegion } from "./stair-center-region.ts";
import { footprintBoundaryHull } from "./supported-footprint-centers.ts";

test("convex support deduplication is independent of ring start and winding", () => {
  const boundary: Point[] = [
    [2467.482940492016, 343.1274004758509],
    [2483.8064079045876, 400.57228977840407],
    [2451.119885747444, 409.8602102052063],
    [2434.796418334873, 352.4153209026531],
  ];
  const expected = footprintBoundaryHull(boundary);
  for (const ring of [boundary, [...boundary].reverse()]) {
    for (let start = 0; start < ring.length; start++) {
      const placed = [...ring.slice(start), ...ring.slice(0, start)];
      const regions = convexSupportRegions(placed);
      assert.equal(regions.length, 1);
      assert.deepEqual(footprintBoundaryHull(regions[0]!), expected);
    }
  }
});

test("concave support has usable convex regions even when its common kernel is empty", () => {
  const boundary: Point[] = [
    [0, 0],
    [30, 0],
    [30, 30],
    [20, 30],
    [20, 10],
    [10, 10],
    [10, 30],
    [0, 30],
  ];
  assert.deepEqual(stairCenterRegion(boundary, [0, 0]), []);
  for (const offset of [0, 20000.125]) {
    const placed = boundary.map(([x, y]): Point => [x + offset, y + offset]);
    const regions = convexSupportRegions(placed);
    assert.ok(regions.length >= 3);
    for (const region of regions) {
      assert.equal(clipping.difference([region], [placed]).length, 0);
      assert.ok(
        stairBoundaryPlanes(region).every(([a, b, c]) =>
          region.every(([x, y]) => a * x + b * y + c >= -1e-6),
        ),
      );
    }
    const union = clipping.union(regions.map((ring) => [ring]));
    assert.equal(
      clipping.xor(union, [placed]).length,
      0,
      "candidate regions must retain complete support coverage",
    );
  }
});
