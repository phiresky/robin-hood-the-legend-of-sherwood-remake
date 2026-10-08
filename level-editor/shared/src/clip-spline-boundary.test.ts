import test from "node:test";
import assert from "node:assert/strict";
import { clipSplineBoundary } from "./clip-spline-boundary.ts";
import type { Vec3 } from "./scene.ts";

test("cropping retains spatial bends hidden by the clipping projection", () => {
  const points: Vec3[] = [
    [0, 0, 0],
    [10, 0, 4],
    [20, 0, 0],
    [30, 0, 0],
    [30, 20, 0],
    [0, 20, 0],
  ];
  for (const input of [points, [...points].reverse()]) {
    const result = clipSplineBoundary(input, 0, 5, 25, [15]);
    assert.equal(result.length, 1);
    assert.ok(result[0]!.some((p) => p[0] === 10 && p[1] === 0 && p[2] === 4));
    assert.ok(result[0]!.some((p) => p[0] === 15 && p[1] === 0 && p[2] === 2));
    assert.ok(result[0]!.some((p) => p[0] === 20 && p[1] === 0 && p[2] === 0));
  }
});

test("closed contour crops preserve separate islands and interpolated heights", () => {
  const xy = [
    [0, 0],
    [30, 0],
    [30, 30],
    [0, 30],
    [0, 20],
    [25, 20],
    [25, 10],
    [0, 10],
  ];
  for (const axis of [0, 1]) {
    const points = xy.map(([x, y]): Vec3 =>
      axis === 0 ? [x!, y!, 2 * x! + y!] : [y!, x!, 2 * x! + y!],
    );
    const before = structuredClone(points);
    for (const input of [points, [...points].reverse()]) {
      const result = clipSplineBoundary(input, axis, 5, 20, [10, 15]);
      assert.equal(result.length, 2);
      for (const contour of result) {
        assert.ok(contour.every((p) => p[axis]! >= 5 && p[axis]! <= 20));
        assert.ok(
          contour.every((p) => p[1 - axis]! <= 10) || contour.every((p) => p[1 - axis]! >= 20),
        );
        assert.ok(contour.some((p) => p[axis] === 10));
        assert.ok(contour.some((p) => p[axis] === 15));
        assert.ok(contour.every((p) => Math.abs(p[2] - 2 * p[axis]! - p[1 - axis]!) < 1e-8));
      }
    }
    assert.deepEqual(points, before);
  }
});

test("vertical and wholly trimmed contours do not invent connecting boundaries", () => {
  const points: Vec3[] = [
    [0, 4, 0],
    [30, 4, 0],
    [30, 4, 30],
    [0, 4, 30],
    [0, 4, 20],
    [25, 4, 20],
    [25, 4, 10],
    [0, 4, 10],
  ];
  const result = clipSplineBoundary(points, 0, 5, 20, []);
  assert.equal(result.length, 2);
  assert.ok(result.flat().every((point) => point[1] === 4));
  assert.deepEqual(clipSplineBoundary(points, 0, 31, 40, []), []);
  assert.deepEqual(clipSplineBoundary(points, 0, 30, 40, []), []);
});
