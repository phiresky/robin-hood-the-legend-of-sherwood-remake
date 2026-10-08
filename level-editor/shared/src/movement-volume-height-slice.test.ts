import test from "node:test";
import assert from "node:assert/strict";
import { movementVolumeHeightSlice } from "./movement-volume-height-slice.ts";
import type { HeightPlane } from "./gameplay-plane.ts";

test("rounded sloped underside contact blocks without closing real underpasses", () => {
  const footprint: [number, number][] = [
    [100, 190],
    [200, 190],
    [200, 210],
    [100, 210],
  ];
  const floor: HeightPlane = [0.1, 0, -10];
  const top: HeightPlane = [0.1, 0, 30];
  assert.equal(movementVolumeHeightSlice(footprint, floor, [0.1000004, 0, -10], top).length, 4);
  assert.equal(movementVolumeHeightSlice(footprint, floor, [0.1, 0, -9.99], top).length, 0);
  assert.equal(movementVolumeHeightSlice(footprint, floor, [0, 0, -20], floor).length, 0);
});
