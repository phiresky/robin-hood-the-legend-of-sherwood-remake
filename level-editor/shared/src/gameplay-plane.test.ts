import assert from "node:assert/strict";
import test from "node:test";
import { heightPlane, planeHeight, projectionPlaneAnchors } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

test("generated receiver anchors retain sloped heights after clipping and float conversion", () => {
  for (const shift of [0, 1400, 24000]) {
    const points: Point[] = [
      [shift + 700, 810],
      [shift + 710, 815],
      [shift + 720, 820.000001],
      [shift + 730, 850],
      [shift + 680, 850],
    ];
    const plane: [number, number, number] = [0.7, -0.4, 57 - shift * 0.7];
    const anchors = projectionPlaneAnchors(points, plane);
    const rounded = anchors.map(([x, y, z]): Vec3 => [
      Math.fround(x),
      Math.fround(y),
      Math.fround(z),
    ]);
    const runtime = heightPlane(rounded);
    for (const [x, y] of points) {
      const z = planeHeight(plane, [x, y]);
      assert.ok(Math.abs(planeHeight(runtime, [x, y + z]) - z) < 0.001);
    }
    assert.deepEqual(projectionPlaneAnchors([...points].reverse(), plane), anchors);
  }
});
