import test from "node:test";
import assert from "node:assert/strict";
import { createJumpWalkingClearance, type JumpWalkArea } from "./jump-walking-clearance.ts";
import type { JumpEdge } from "./jump-clearance.ts";

const edges: [JumpEdge, JumpEdge] = [
  { zone: "left", a: [90, 90, 0], b: [90, 10, 0] },
  { zone: "right", a: [140, 10, 0], b: [140, 90, 0] },
];
const areas: JumpWalkArea[] = [
  {
    plane: [0, 0, 0],
    polygon: [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ],
    blockers: [],
  },
  {
    plane: [0, 0, 0],
    polygon: [
      [130, 0],
      [230, 0],
      [230, 100],
      [130, 100],
    ],
    blockers: [],
  },
];
const bands = new Map(
  edges.map((e) => [e.zone, { plane: [0, 0, 0] as [number, number, number], depth: 6 }]),
);

test("movement-only exclusions trim an approach even with a clear flight", () => {
  assert.deepEqual(createJumpWalkingClearance(areas, bands)(edges), []);
  const blocked = structuredClone(areas);
  blocked[0]!.blockers.push([
    [80, 48],
    [100, 48],
    [100, 52],
    [80, 52],
  ]);
  const intervals = createJumpWalkingClearance(blocked, bands)(edges);
  assert.equal(intervals.length, 1);
  assert.deepEqual(intervals, [[0.4375, 0.5625]]);
  assert.ok(intervals[0]![0] > 0 && intervals[0]![1] < 1);
});

test("coplanar alternative areas cannot bridge a blocked seam or substitute a different height", () => {
  const split = structuredClone(areas);
  split[0]!.polygon = [
    [0, 0],
    [100, 0],
    [100, 48],
    [0, 48],
  ];
  split.push({
    plane: [0, 0, 0],
    polygon: [
      [0, 52],
      [100, 52],
      [100, 100],
      [0, 100],
    ],
    blockers: [],
  });
  const intervals = createJumpWalkingClearance(split, bands)(edges);
  assert.equal(intervals.length, 1);
  assert.ok(intervals[0]![0] < 0.5 && intervals[0]![1] > 0.5);
  split[0]!.plane[2] = 10;
  split[2]!.plane[2] = 10;
  assert.deepEqual(createJumpWalkingClearance(split, bands)(edges), [[0, 1]]);
});

test("the whole inward approach needs clearance, even when the takeoff line is clear", () => {
  const blocked = structuredClone(areas);
  blocked[0]!.blockers.push([[80, 0], [81, 0], [81, 100], [80, 100]]);
  assert.deepEqual(createJumpWalkingClearance(blocked, bands)(edges), [[0, 1]]);
});
