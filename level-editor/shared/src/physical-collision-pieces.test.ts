import test from "node:test";
import assert from "node:assert/strict";
import clipping from "polygon-clipping";
import { physicalCollisionPieces } from "./physical-collision-pieces.ts";
import { compilePhysicalStairArea } from "./compile-physical-stair.ts";
import type { Point } from "./level.ts";

const strip: Point[] = [
  [257.1864776169649, 1286.4635876471746],
  [276.92972871616627, 1292.5339584812298],
  [276.89479870429943, 1292.571673991555],
  [276.7848401403829, 1292.5375951677054],
  [276.81959049275, 1292.5000948125062],
  [257.18676192694625, 1286.4636757617282],
];

test("thin collision that crosses itself at runtime precision retains its entire footprint", () => {
  const pieces = physicalCollisionPieces(strip);
  assert.equal(pieces.length, 4);
  assert.deepEqual(clipping.xor([strip], clipping.union(pieces.map((ring) => [ring]))), []);
  for (const piece of pieces) {
    assert.equal(piece.length, 3);
    const [a, b, c] = piece.map(([x, y]): Point => [Math.fround(x), Math.fround(y)]);
    assert.ok(
      Math.abs((b![0] - a![0]) * (c![1] - a![1]) - (b![1] - a![1]) * (c![0] - a![0])) > 1e-9,
    );
  }
  const compiled = compilePhysicalStairArea({
    surfaces: [
      {
        polygon: [
          [250, 1280, 0],
          [280, 1280, 0],
          [280, 1300, 0],
          [250, 1300, 0],
        ],
        holes: [],
      },
    ],
    doors: [],
    obstacles: [{ stateId: 2, polygon: strip.map(([x, y]) => [x, y, 0]) }],
  });
  assert.equal(compiled.navigation.obstacles.length, 4);
  for (const [index, obstacle] of compiled.navigation.obstacles.entries()) {
    assert.equal(obstacle.motion_obstacle, index);
    assert.equal(compiled.area.obstacles[index]!.state_id, 2);
  }
});

test("valid thin collision stays unchanged and invalid authored collision is rejected", () => {
  const thin: Point[] = [
    [0, 0],
    [20, 0],
    [20, 0.001],
    [0, 0.001],
  ];
  assert.deepEqual(physicalCollisionPieces(thin), [thin]);
  assert.throws(
    () =>
      physicalCollisionPieces([
        [0, 0],
        [20, 20],
        [0, 20],
        [20, 0],
      ]),
    /not simple/,
  );
});
