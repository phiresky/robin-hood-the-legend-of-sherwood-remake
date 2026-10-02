import test from "node:test";
import assert from "node:assert/strict";
import {
  declaredDoorOwners,
  doorOwnershipFootprint,
  recoverDoorStateOwner,
  unownedInteriorEntrances,
} from "./recover-door-owner.ts";
import type { SightObstacle, Point } from "@rle/shared";

test("inferred rooms cannot assign distant entrances to the first doorway's asset", () => {
  const obstacle: SightObstacle = {
    points: [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ].map(([x, y]) => ({ x: x!, y: y!, z_bottom: 0, z_top: 100 })),
    projection_area: null,
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  };
  const entrances = [
    { door: 4, point: [20, 20] as Point, height: 0 },
    { door: 8, point: [1020, 20] as Point, height: 0 },
  ];
  assert.deepEqual(unownedInteriorEntrances(entrances, [obstacle]), [{ door: 8, distance: 920 }]);
  const annex = { ...obstacle, points: obstacle.points.map((p) => ({ ...p, x: p.x + 1000 })) };
  assert.deepEqual(unownedInteriorEntrances(entrances, [obstacle, annex]), []);
  // A floor beneath an unrelated entrance is not evidence of a doorway owner.
  assert.deepEqual(
    unownedInteriorEntrances(entrances, [
      obstacle,
      { ...annex, points: annex.points.map((p) => ({ ...p, z_top: 0 })) },
    ]),
    [{ door: 8, distance: 920 }],
  );
  assert.deepEqual(unownedInteriorEntrances(entrances, []), [
    { door: 4, distance: null },
    { door: 8, distance: null },
  ]);
  assert.deepEqual(
    unownedInteriorEntrances([{ door: 2, point: [124, 50], height: 0 }], [obstacle]),
    [],
  );
  assert.equal(
    unownedInteriorEntrances([{ door: 2, point: [125, 50], height: 0 }], [obstacle]).length,
    1,
  );
});

test("declared door ownership requires unique endpoints and one pinned frame", () => {
  const frame = { asset: "gate", node: "arch" };
  const entries = [{ doors: [1, 2], owner: "gate", node: "arch", reason: "Gate passage" }];
  const frames = (asset: string, node: string) =>
    asset === frame.asset && node === frame.node ? [frame] : [];
  assert.deepEqual(
    [...declaredDoorOwners(entries, 3, frames)],
    [
      [1, frame],
      [2, frame],
    ],
  );
  assert.throws(() => declaredDoorOwners([...entries, ...entries], 3, frames), /duplicate/);
  assert.throws(() => declaredDoorOwners(entries, 2, frames), /Invalid/);
  assert.throws(
    () => declaredDoorOwners([{ ...entries[0]!, node: "missing" }], 3, frames),
    /pinned/,
  );
  assert.throws(() => declaredDoorOwners(entries, 3, () => [frame, frame]), /pinned/);
  assert.throws(() => declaredDoorOwners([{ ...entries[0]!, reason: "" }], 3, frames), /rationale/);
});

test("door ownership excludes supporting terrain and preserves disconnected sloped slices", () => {
  const outline: Point[] = [
    [0, 0],
    [6, 0],
    [6, 6],
    [4, 6],
    [4, 2],
    [2, 2],
    [2, 6],
    [0, 6],
  ];
  const obstacle: SightObstacle = {
    points: outline.map(([x, y]) => ({ x, y, z_bottom: 0, z_top: y })),
    projection_area: null,
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  };
  const slices = doorOwnershipFootprint(obstacle, 3);
  assert.equal(slices.length, 2);
  assert.ok(slices.every((ring) => ring.every((point) => point[1] > 3)));
  assert.deepEqual(doorOwnershipFootprint(obstacle, 6), []);
  obstacle.points.forEach((point) => {
    point.z_top = 3;
  });
  assert.deepEqual(doorOwnershipFootprint(obstacle, 3), []);
  obstacle.points.forEach((point) => {
    point.z_bottom = 30;
    point.z_top = 40;
  });
  assert.deepEqual(doorOwnershipFootprint(obstacle, 3), []);
});

test("door ownership follows all linked state geometry, independent of source indices", () => {
  const owner = { asset: "gate", node: "closed" };
  const owners = new Map([
    [80, [owner]],
    [12, [{ asset: "gate", node: "open" }]],
  ]);
  const patch = { door_indices: [7, 8], old_sight_obstacles: [80], new_sight_obstacles: [12] };
  assert.equal(recoverDoorStateOwner([8], [patch], owners), owner);
  assert.equal(recoverDoorStateOwner([3], [patch], owners), undefined);
  assert.equal(
    recoverDoorStateOwner([8], [{ ...patch, new_sight_obstacles: [13] }], owners),
    undefined,
  );
  owners.set(12, [{ asset: "other", node: "open" }]);
  assert.equal(recoverDoorStateOwner([8], [patch], owners), undefined);
  owners.set(12, [owner, owner]);
  assert.equal(recoverDoorStateOwner([8], [patch], owners), undefined);
});

test("conflicting linked patches and geometry-free permission changes do not invent owners", () => {
  const owners = new Map([
    [1, [{ asset: "first" }]],
    [2, [{ asset: "second" }]],
  ]);
  const patch = { door_indices: [0], old_sight_obstacles: [1], new_sight_obstacles: [] };
  assert.equal(
    recoverDoorStateOwner([0], [patch, { ...patch, old_sight_obstacles: [2] }], owners),
    undefined,
  );
  assert.equal(
    recoverDoorStateOwner([0], [{ ...patch, old_sight_obstacles: [] }], owners),
    undefined,
  );
});
