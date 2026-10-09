import test from "node:test";
import assert from "node:assert/strict";
import { takeoffReceiverRegions } from "./takeoff-receiver-regions.ts";
import { planeHeight } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
type Receiver = Parameters<typeof takeoffReceiverRegions>[2][number];

const source = { zone: "roof", a: [0, 80, 80], b: [0, 180, 80] } as const;
const rectangle = (low: number, high: number): Point[] => [
  [low, 0],
  [high, 0],
  [high, 100],
  [low, 100],
];
const receiver: Receiver = { polygon: rectangle(-5, 5), plane: [0, 0, 80] };

for (const inside of [false, true])
  test(`takeoff ${inside ? "drops at" : "retains its plane beyond"} a receiving boundary ${inside ? "inside" : "outside"} the motion area`, () => {
    const regions = takeoffReceiverRegions(
      { ...source, a: [...source.a], b: [...source.b] },
      receiver.plane,
      [receiver],
      rectangle(inside ? -20 : -4, 10),
    );
    let area = 0;
    for (const region of regions)
      for (const polygon of region.polygons) {
        const ring = polygon[0]!;
        const x = ring.reduce((sum, p) => sum + p[0], 0) / ring.length;
        assert.equal(region.plane[2], inside && x < -5 ? 0 : 80);
        area +=
          Math.abs(
            ring.reduce((sum, p, i) => {
              const q = ring[(i + 1) % ring.length]!;
              return sum + p[0] * q[1] - p[1] * q[0];
            }, 0),
          ) / 2;
      }
    assert.ok(Math.abs(area - 1500) < 1e-7);
  });

test("leaving the motion area retains the last crossed receiver, not the initial plane", () => {
  const slope: Receiver = { polygon: rectangle(-10, -5), plane: [1, 0, 85] };
  const regions = takeoffReceiverRegions(
    { ...source, a: [...source.a], b: [...source.b] },
    receiver.plane,
    [receiver, slope],
    rectangle(-9, 10),
  );
  let beyondSlope = false;
  for (const region of regions)
    for (const polygon of region.polygons) {
      const ring = polygon[0]!;
      const point: Point = [0, 1].map(
        (axis) => ring.reduce((sum, p) => sum + p[axis]!, 0) / ring.length,
      ) as Point;
      assert.ok(
        Math.abs(planeHeight(region.plane, point) - (point[0] < -5 ? 85 + point[0] : 80)) < 1e-7,
      );
      beyondSlope ||= point[0] < -10;
    }
  assert.ok(beyondSlope);
});

test("a subpixel motion-area margin follows native elevation-boundary side probes", () => {
  for (const margin of [0.00005, 0.0002]) {
    const regions = takeoffReceiverRegions(
      { ...source, a: [...source.a], b: [...source.b] },
      receiver.plane,
      [receiver],
      rectangle(-5 - margin, 10),
    );
    const beyond = regions.filter((region) => {
      const ring = region.polygons[0]![0]!;
      return ring.reduce((sum, point) => sum + point[0], 0) / ring.length < -6;
    });
    assert.ok(beyond.length);
    assert.ok(beyond.every((region) => region.plane[2] === (margin < 0.0001 ? 80 : 0)));
  }
});
