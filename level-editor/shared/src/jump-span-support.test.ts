import test from "node:test";
import assert from "node:assert/strict";
import type { Point } from "./level.ts";
import {
  jumpSpanInsideReceiver,
  jumpSpanHasFootprint,
  jumpFlightObstacles,
} from "./jump-span-support.ts";

const rectangle = (x: number, y: number, w: number, h: number): Point[] => [
  [x, y],
  [x + w, y],
  [x + w, y + h],
  [x, y + h],
];

test("receiver span rejects a notch missed by endpoints and midpoint", () => {
  const boundary: Point[] = [
    [0, 0],
    [10, 0],
    [10, 10],
    [4, 10],
    [4, 2],
    [2, 2],
    [2, 10],
    [0, 10],
  ];
  assert.equal(jumpSpanInsideReceiver([1, 5], [9, 5], boundary), false);
  assert.equal(jumpSpanInsideReceiver([1, 1], [9, 1], boundary), true);
  assert.equal(jumpSpanInsideReceiver([2, 2], [4, 2], boundary), true);
  assert.equal(jumpSpanInsideReceiver([1, 5], [1, 5], boundary), true);
});

test("whole footprint sweep rejects intermediate gaps and interior holes", () => {
  const whole = rectangle(0, 0, 30, 20);
  assert.equal(jumpSpanHasFootprint([6, 10], [24, 10], [[whole]], [6, 3]), true);
  const hole = rectangle(11, 9, 1, 2);
  assert.equal(jumpSpanHasFootprint([6, 10], [24, 10], [[whole, hole]], [6, 3]), false);
  const gap = [[rectangle(0, 0, 12, 20)], [rectangle(13, 0, 17, 20)]];
  assert.equal(jumpSpanHasFootprint([6, 10], [24, 10], gap, [6, 3]), false);
  assert.equal(jumpSpanHasFootprint([5, 10], [24, 10], [[whole]], [6, 3]), false);
});

test("flight checks include every solid state without removing inactive receiving planes", () => {
  const receiver = {
    points: [],
    projection_area: [0, 0],
    opaque: false,
    solid: false,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  };
  const geometry = {
    sight_obstacles: [receiver, receiver, { ...receiver, initial_active: false }, receiver],
    movement_transitions: [
      {
        id: "roof",
        waypoint: [0, 0] as Point,
        sector: 0,
        layer: 0,
        active: true,
        definitive: false,
        apply_polygon: { points: [] },
        no_apply_polygon: { points: [] },
        motion_changes: [],
        initial_sight: [0],
        applied_sight: [1],
      },
    ],
  };
  const before = structuredClone(geometry);
  const obstacles = jumpFlightObstacles(geometry);
  assert.equal(obstacles.length, 4);
  assert.equal(obstacles[0]!.initial_active, true);
  assert.equal(obstacles[1]!.initial_active, true);
  assert.equal(obstacles[2]!.initial_active, false);
  assert.deepEqual(obstacles[2]!.projection_area, [0, 0]);
  assert.deepEqual(geometry, before);
});
