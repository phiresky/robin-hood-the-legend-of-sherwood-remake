import test from "node:test";
import assert from "node:assert/strict";
import { splineLightReceivers } from "./spline-light-receivers.ts";
import type { AssetLightRegion, AssetWalkableSurface } from "./asset-gameplay.ts";

test("automatic spline light probes stay inside quantized contours and receiving triangles", () => {
  const light: AssetLightRegion = {
    id: "light",
    node: "$root",
    ambiences: 1,
    polygon: [
      [0.3, 10.3, 10],
      [10.3, 10.3, 10],
      [0.3, 20.3, 10],
    ],
  };
  const receiver: AssetWalkableSurface = {
    id: "surface",
    node: "$root",
    height: 10,
    polygon: [
      [5, 10],
      [15, 10],
      [5, 20],
    ],
  };
  const probes = splineLightReceivers(light, [receiver], [0.3, 0.3]);
  assert.equal(probes.length, 1);
  const [a, b] = probes[0]!;
  assert.ok(a[0] > 5 && a[0] < 10.3);
  assert.ok(a[1] - a[2] > 0.3);
  assert.ok(a[0] + a[1] - a[2] < 10.6);
  assert.equal(a[0], b[0]);
  assert.equal(a[1] - a[2], b[1] - b[2]);
  assert.ok(a[2] < 10 && b[2] > 10);
  const reversed = { ...receiver, polygon: [...receiver.polygon].reverse() };
  assert.deepEqual(splineLightReceivers(light, [reversed], [0.3, 0.3]), probes);
  const distant = {
    ...receiver,
    polygon: receiver.polygon.map(([x, y]): [number, number] => [x + 100, y]),
  };
  assert.deepEqual(splineLightReceivers(light, [distant], [0.3, 0.3]), []);
});
