import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareStairPassages } from "./prepare-stair-passages.ts";
import { compileLiftPassageStates } from "./compile-passage-states.ts";

const rectangle = (left: number, top: number, right: number, bottom: number): Point[] => [
  [left, top],
  [right, top],
  [right, bottom],
  [left, bottom],
];
function fixture() {
  const { asset_geometry: geometry }: { asset_geometry: CompiledAssetGeometry } = JSON.parse(
    readFileSync(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-lift.level.json", import.meta.url),
      "utf8",
    ),
  );
  const lift = geometry.lifts![0]!;
  lift.lift_type = 1;
  const door = lift.doors[0]!;
  lift.doors = [door];
  door.point_mid = [50, 50];
  door.point_out = [50, 40];
  door.point_in = [50, 60];
  door.sector_out = 0;
  door.sector_in = 2;
  door.layer_out = 0;
  door.layer_in = 1;
  const area = (is_lift: boolean) => ({
    is_lift,
    state_id: 0,
    flags: 0,
    skeleton_segments: [],
    polygon: { points: rectangle(0, 0, 100, 100) },
    obstacles: [] as CompiledAssetGeometry["motion_data"]["layers"][number][number]["obstacles"],
  });
  const landing = area(false),
    stairs = area(true);
  landing.obstacles.push({ state_id: 0, polygon: { points: rectangle(40, 50, 60, 70) } });
  geometry.motion_data.layers = [[landing], [stairs]];
  return { geometry, door, lift, landing, stairs };
}

test("stair handoff clears an opening without changing endpoints or collision", () => {
  const { geometry, door } = fixture();
  const before = structuredClone(geometry.motion_data);
  assert.deepEqual(compileLiftPassageStates(geometry)[0]![0]![0]!.allowed_states, []);
  prepareStairPassages(geometry);
  assert.deepEqual(door.point_mid, [50, 47]);
  assert.deepEqual(door.point_out, [50, 40]);
  assert.deepEqual(door.point_in, [50, 60]);
  assert.deepEqual(geometry.motion_data, before);
  assert.deepEqual(compileLiftPassageStates(geometry)[0]![0], []);
  const once = structuredClone(geometry);
  prepareStairPassages(geometry);
  assert.deepEqual(geometry, once);
});

test("handoff cannot bypass a solid shared by both floors or extend a long passage", () => {
  for (const type of [1, 2] as const)
    for (const kind of ["solid", "distance"] as const) {
      const { geometry, door, lift, stairs, landing } = fixture();
      lift.lift_type = type;
      door.door_type = 5;
      if (kind === "solid") stairs.obstacles.push(structuredClone(landing.obstacles[0]!));
      else landing.obstacles[0]!.polygon.points = rectangle(40, 40, 60, 70);
      prepareStairPassages(geometry);
      assert.deepEqual(door.point_mid, [50, 50]);
      assert.ok(
        compileLiftPassageStates(geometry)[0]![0]!.some((r) => r.allowed_states.length === 0),
      );
    }
});

test("low ladder handoffs clear landing holes without changing collision", () => {
  const { geometry, door, lift } = fixture();
  lift.lift_type = 2;
  door.door_type = 5;
  const before = structuredClone(geometry.motion_data);
  prepareStairPassages(geometry);
  assert.deepEqual(door.point_mid, [50, 47]);
  assert.deepEqual(geometry.motion_data, before);
  assert.deepEqual(compileLiftPassageStates(geometry)[0]![0], []);
});

test("walls, high ladders and switchable barriers retain their authored handoff", () => {
  for (const kind of ["wall", "high-ladder", "state", "ladder-state"] as const) {
    const { geometry, door, lift, landing } = fixture();
    if (kind === "wall") lift.lift_type = 3;
    else if (kind === "high-ladder") {
      lift.lift_type = 2;
      door.door_type = 4;
    } else {
      landing.obstacles[0]!.state_id = 1;
      if (kind === "ladder-state") {
        lift.lift_type = 2;
        door.door_type = 5;
      }
    }
    prepareStairPassages(geometry);
    assert.deepEqual(door.point_mid, [50, 50]);
    if (kind === "state" || kind === "ladder-state")
      assert.deepEqual(compileLiftPassageStates(geometry)[0]![0]![0]!.allowed_states, [2]);
  }
});

test("high ladder exits climb over their own solid but keep other barriers", () => {
  for (const kind of ["climbed", "edge", "foreign", "switchable"] as const) {
    const { geometry, door, lift, landing } = fixture();
    lift.lift_type = 2;
    door.door_type = 4;
    // The authored clamber starts inside the parapet it climbs over; after
    // rotation and rounding it may instead lie just outside its edge.
    landing.obstacles[0]!.polygon.points = rectangle(40, kind === "edge" ? 51 : 46, 60, 70);
    if (kind === "foreign")
      landing.obstacles.push({ state_id: 0, polygon: { points: rectangle(45, 38, 55, 43) } });
    if (kind === "switchable")
      landing.obstacles.push({ state_id: 1, polygon: { points: rectangle(45, 38, 55, 43) } });
    door.sector_in = 1 + landing.obstacles.length;
    const states = compileLiftPassageStates(geometry)[0]![0]!;
    if (kind === "climbed" || kind === "edge") assert.deepEqual(states, []);
    else
      assert.deepEqual(
        states.map((requirement) => requirement.allowed_states),
        [kind === "foreign" ? [] : [2]],
      );
    // Low ladder handoffs keep the ordinary two-sided permanent check.
    door.door_type = 5;
    assert.deepEqual(compileLiftPassageStates(geometry)[0]![0]![0]!.allowed_states, []);
  }
});
