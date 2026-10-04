import test from "node:test";
import assert from "node:assert/strict";
import { clipSplinePolyline } from "./clip-spline-polyline.ts";

test("polyline cropping preserves direction, bend stations and disconnected fragments", () => {
  assert.deepEqual(
    clipSplinePolyline(
      [
        [-10, 0, 0],
        [30, 40, 20],
      ],
      0,
      0,
      20,
      [10],
    ),
    [
      [
        [0, 10, 5],
        [10, 20, 10],
        [20, 30, 15],
      ],
    ],
  );
  assert.deepEqual(
    clipSplinePolyline(
      [
        [30, 40, 20],
        [-10, 0, 0],
      ],
      0,
      0,
      20,
      [10],
    ),
    [
      [
        [20, 30, 15],
        [10, 20, 10],
        [0, 10, 5],
      ],
    ],
  );
  assert.deepEqual(
    clipSplinePolyline(
      [
        [0, 0, 0],
        [30, 0, 0],
        [30, 20, 0],
        [0, 20, 0],
      ],
      0,
      0,
      20,
      [],
    ),
    [
      [
        [0, 0, 0],
        [20, 0, 0],
      ],
      [
        [20, 20, 0],
        [0, 20, 0],
      ],
    ],
  );
  assert.deepEqual(
    clipSplinePolyline(
      [
        [0, 0, 0],
        [10, 0, 0],
        [10, 20, 0],
      ],
      0,
      0,
      20,
      [],
    ),
    [
      [
        [0, 0, 0],
        [10, 0, 0],
        [10, 20, 0],
      ],
    ],
  );
  assert.deepEqual(clipSplinePolyline([[25, 0, 0]], 0, 0, 20, []), []);
  assert.deepEqual(clipSplinePolyline([[20, 0, 0]], 0, 0, 20, []), [[[20, 0, 0]]]);
});
