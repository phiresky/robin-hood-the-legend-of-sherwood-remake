import test from "node:test";
import assert from "node:assert/strict";
import { physicalStairFloor } from "./physical-stair-floor.ts";
import type { Vec3 } from "./scene.ts";

const flights = (): { polygon: Vec3[] }[] => [
  {
    polygon: [
      [0, 0, 0],
      [10, 0, 10],
      [10, 20, 10],
      [0, 20, 0],
    ],
  },
  {
    polygon: [
      [10, 0, 10],
      [20, 0, 30],
      [20, 20, 30],
      [10, 20, 10],
    ],
  },
];

test("joined floor retains slope changes on projected boundary edges", () => {
  const floor = physicalStairFloor(flights());
  assert.equal(floor.patches?.length, 2);
  assert.equal(floor.heightAt([5, 10]), 5);
  assert.equal(floor.heightAt([15, 10]), 20);
  assert.deepEqual(
    floor.splitRing([
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 20],
    ]),
    [
      [0, 0],
      [10, 0],
      [20, 0],
      [20, 20],
      [10, 20],
      [0, 20],
    ],
  );
  assert.throws(() => floor.heightAt([25, 10]), /no floor support/);
});

test("joined floor rejects incompatible shared edges and overlapping flights", () => {
  const mismatch = flights();
  for (const p of mismatch[1]!.polygon) p[2] += 0.01;
  assert.throws(() => physicalStairFloor(mismatch), /disagree/);
  const overlap = flights();
  for (const p of overlap[1]!.polygon) p[0] -= 1;
  assert.throws(() => physicalStairFloor(overlap), /disagree/);
});
