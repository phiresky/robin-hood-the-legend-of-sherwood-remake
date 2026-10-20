import test from "node:test";
import assert from "node:assert/strict";
import clipping from "polygon-clipping";
import { climbEntryClearance } from "./climb-entry-clearance.ts";
import { stairCenterRegion } from "./stair-center-region.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { compilePhysicalStairArea } from "./compile-physical-stair.ts";

const floor: Point[] = [
  [1615, 1802],
  [1625, 1689],
  [1634, 1702],
  [1625, 1816],
];
const entries: Point[] = [
  [1622, 1793],
  [1627, 1719],
];

test("narrow rotated climbs reserve complete entry footprints without extending supported centers", () => {
  const prepared = climbEntryClearance(floor, floor, entries);
  assert.ok(prepared);
  for (const [x, y] of entries)
    for (const dx of [-6, 6])
      for (const dy of [-3, 3]) assert.ok(pointInGameplayPolygon([x + dx, y + dy], prepared));
  const centers = stairCenterRegion(prepared, [6, 3]);
  assert.ok(entries.every((point) => pointInGameplayPolygon(point, centers)));
  assert.deepEqual(clipping.difference([centers], [floor]), []);
  assert.equal(climbEntryClearance(prepared, floor, entries), undefined);
});

test("climb envelopes cannot manufacture support for missing or concave floor", () => {
  assert.equal(climbEntryClearance(floor, floor, [[1600, 1800]]), undefined);
  const concave: Point[] = [
    [0, 0],
    [30, 0],
    [30, 30],
    [20, 30],
    [20, 10],
    [0, 10],
  ];
  assert.equal(
    climbEntryClearance(concave, concave, [
      [25, 25],
      [5, 5],
    ]),
    undefined,
  );
  assert.equal(climbEntryClearance(floor, floor, []), undefined);
});

test("an enlarged envelope cannot retain existing unsupported root positions", () => {
  const oversized: Point[] = [
    [1500, 1600],
    [1700, 1600],
    [1700, 1900],
    [1500, 1900],
  ];
  // One entry footprint protrudes from the oversized boundary, forcing a candidate.
  const narrowFloor: Point[] = [
    [1499, 1650],
    [1701, 1650],
    [1701, 1850],
    [1499, 1850],
  ];
  assert.equal(climbEntryClearance(oversized, narrowFloor, [[1501, 1700]]), undefined);
});

test("static and changing barriers keep their collision boundaries and state identities", () => {
  const polygon = floor.map(([x, y]): Vec3 => [x, y, 0]);
  const doors = entries.map(([x, y]) => ({
    inside: [x, y, 0] as Vec3,
    middle: [x, y, 0] as Vec3,
    outside: [x, y, 0] as Vec3,
  }));
  const obstacle: Vec3[] = [
    [1619, 1750, 0],
    [1630, 1750, 0],
    [1630, 1752, 0],
    [1619, 1752, 0],
  ];
  const open = compilePhysicalStairArea({
    prepareClimbingClearance: true,
    surfaces: [{ polygon, holes: [] }],
    obstacles: [],
    doors,
  });
  assert.ok(open.area.precise_polygon, "the unobstructed rotated floor needs an entry envelope");
  for (const stateId of [0, 1]) {
    const input = {
      surfaces: [{ polygon, holes: [] }],
      obstacles: [{ stateId, polygon: obstacle }],
      doors,
    };
    const blocked = compilePhysicalStairArea({ ...input, prepareClimbingClearance: true });
    const unchanged = compilePhysicalStairArea(input);
    assert.deepEqual(blocked.area, unchanged.area);
    assert.ok(blocked.area.obstacles.length > 0);
    assert.ok(blocked.area.obstacles.every((obstacle) => obstacle.state_id === stateId));
  }
});
