import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareOrdinaryStairPassages } from "./prepare-ordinary-stair-passages.ts";

function fixture() {
  const document: { asset_geometry: CompiledAssetGeometry } = JSON.parse(
    readFileSync(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-lift.level.json", import.meta.url),
      "utf8",
    ),
  );
  const geometry = document.asset_geometry;
  const lift = geometry.lifts![0]!;
  const door = structuredClone(lift.doors[0]!);
  Object.assign(door, {
    door_type: 0,
    point_in: [2, 50],
    point_mid: [0, 50],
    point_out: [150, 50],
    sector_in: 1,
    layer_in: 1,
    sector_out: 0,
    layer_out: 0,
    locked_npc_villain: true,
    world_endpoints: { inside: [2, 150, 100], middle: [0, 100, 50], outside: [150, 50, 0] },
    passage_states: [{ layer: 0, area: 0, allowed_states: [1] }],
  });
  geometry.doors = [door];
  lift.lift_type = 1;
  lift.motion_area_index = 1;
  lift.doors = [];
  geometry.lifts = [lift];
  const polygon: Point[] = [
    [0, 0],
    [200, 0],
    [200, 100],
    [0, 100],
  ];
  geometry.motion_data.layers = [false, true].map((is_lift) => [
    {
      is_lift,
      state_id: 0,
      flags: 0,
      skeleton_segments: [],
      obstacles: [],
      polygon: { points: polygon },
    },
  ]);
  geometry.sight_obstacles = [
    {
      points: polygon.map(([x, y]) => ({ x, y: y + 100, z_top: 100, z_bottom: 100 })),
      projection_area: [1, 1],
      opaque: false,
      solid: false,
      mouse: true,
      show_shadow_polygon: false,
      default_material: 0,
      material_indices: [],
    },
  ];
  return { geometry, door };
}

test("ordinary stair approaches gain actor clearance without changing gate permissions", () => {
  const { geometry, door } = fixture();
  const requirements = structuredClone(door.passage_states);
  prepareOrdinaryStairPassages(geometry);
  assert.ok(door.point_in[0] > 6);
  assert.deepEqual(door.point_mid, door.point_in);
  assert.equal(door.world_endpoints!.inside[2], 100);
  assert.deepEqual(door.world_endpoints!.middle, door.world_endpoints!.inside);
  assert.equal(door.locked_npc_villain, true);
  assert.deepEqual(door.passage_states, requirements);
  assert.equal(geometry.lifts![0]!.doors.length, 0);
});

test("missing receivers retain authored ordinary approaches with an explicit warning", () => {
  const { geometry, door } = fixture();
  geometry.sight_obstacles = [];
  const before = structuredClone(door);
  prepareOrdinaryStairPassages(geometry);
  assert.deepEqual(door, before);
  assert.match(geometry.warnings!.join("\n"), /lacks receiving support/);
});

test("an ordinary passage can author the stair on its outside side", () => {
  const { geometry, door } = fixture();
  [door.point_in, door.point_out] = [door.point_out, door.point_in];
  [door.sector_in, door.sector_out] = [door.sector_out, door.sector_in];
  [door.layer_in, door.layer_out] = [door.layer_out, door.layer_in];
  [door.world_endpoints!.inside, door.world_endpoints!.outside] = [
    door.world_endpoints!.outside,
    door.world_endpoints!.inside,
  ];
  prepareOrdinaryStairPassages(geometry);
  assert.ok(door.point_out[0] > 6);
  assert.deepEqual(door.point_mid, door.point_out);
  assert.equal(door.world_endpoints!.outside[2], 100);
  assert.equal(door.sector_in, 0);
  assert.equal(door.sector_out, 1);
  assert.equal(door.locked_npc_villain, true);
});
