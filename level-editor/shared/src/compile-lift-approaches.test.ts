import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { compileLiftApproaches, compileBuildingApproaches } from "./compile-lift-approaches.ts";

function fixture(point: Point, state?: number) {
  const document: { asset_geometry: CompiledAssetGeometry } = JSON.parse(
    readFileSync(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-lift.level.json", import.meta.url),
      "utf8",
    ),
  );
  const geometry = document.asset_geometry;
  geometry.warnings = [];
  const lift = geometry.lifts![0]!;
  const door = lift.doors[0]!;
  lift.lift_type = 0;
  lift.doors = [door];
  door.point_in = point;
  door.point_out = [50, 50];
  door.point_mid = [0, point[1]];
  door.sector_in = door.sector_out = 0;
  door.layer_in = door.layer_out = 0;
  geometry.motion_data.layers = [
    [
      {
        is_lift: true,
        state_id: 0,
        flags: 0,
        polygon: {
          points: [
            [0, 0],
            [100, 0],
            [100, 100],
            [0, 100],
          ],
        },
        skeleton_segments: [],
        obstacles:
          state === undefined
            ? []
            : [
                {
                  state_id: state,
                  polygon: {
                    points: [
                      [10, 0],
                      [11, 0],
                      [11, 100],
                      [10, 100],
                    ],
                  },
                },
              ],
      },
    ],
  ];
  return { geometry, door };
}

test("exported approaches fit the actor without crossing a permanent barrier", () => {
  for (const [source, expected] of [
    [
      [2, 50],
      [7, 50],
    ],
    [
      [20, 50],
      [20, 50],
    ],
    [
      [2, 2],
      [7, 4],
    ],
  ] satisfies [Point, Point][]) {
    const { geometry, door } = fixture(source);
    compileLiftApproaches(geometry);
    assert.deepEqual(door.point_in, expected);
    assert.deepEqual(geometry.warnings, []);
  }
  const { geometry, door } = fixture([2, 50], 0);
  compileLiftApproaches(geometry);
  assert.deepEqual(door.point_in, [2, 50]);
  assert.match(geometry.warnings!.join("\n"), /no actor-sized in approach/);
});

test("switchable barriers preserve a shared approach for their open state", () => {
  for (const state of [1, 2]) {
    const { geometry, door } = fixture([2, 50], state);
    compileLiftApproaches(geometry);
    assert.deepEqual(door.point_in, [7, 50]);
    assert.equal(geometry.motion_data.layers[0]![0]!.obstacles[0]!.state_id, state);
    assert.deepEqual(geometry.warnings, []);
  }
});

test("building approaches fit outside actors without moving virtual rooms or crossing barriers", () => {
  for (const blocked of [false, true]) {
    const { geometry, door } = fixture([99, 99], blocked ? 0 : undefined);
    geometry.motion_data.layers[0]![0]!.is_lift = false;
    geometry.lifts = [];
    door.door_type = 1;
    door.point_out = [2, 50];
    door.point_mid = [0, 50];
    door.sector_in = 999;
    door.layer_in = 3;
    geometry.buildings = [{ Building: { doors: [door] } }];
    const receiver = structuredClone(geometry.sight_obstacles[0]!);
    receiver.projection_area = [0, 0];
    receiver.points = [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ].map(([x, y]) => ({ x: x!, y: y!, z_top: 0, z_bottom: 0 }));
    geometry.sight_obstacles = [receiver];
    const original = structuredClone(door);
    const motion = structuredClone(geometry.motion_data);
    compileBuildingApproaches(geometry);
    assert.deepEqual(door.point_out, blocked ? [2, 50] : [7, 50]);
    assert.deepEqual({ ...door, point_out: original.point_out }, original);
    assert.deepEqual(geometry.motion_data, motion);
    if (blocked)
      assert.match(geometry.warnings!.join("\n"), /Building 0 door 0: no actor-sized out approach/);
    else assert.deepEqual(geometry.warnings, []);
  }
});
