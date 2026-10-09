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
  for (const kind of ["solid", "distance"] as const) {
    const { geometry, door, stairs, landing } = fixture();
    if (kind === "solid") stairs.obstacles.push(structuredClone(landing.obstacles[0]!));
    else landing.obstacles[0]!.polygon.points = rectangle(40, 40, 60, 70);
    prepareStairPassages(geometry);
    assert.deepEqual(door.point_mid, [50, 50]);
    assert.ok(
      compileLiftPassageStates(geometry)[0]![0]!.some((r) => r.allowed_states.length === 0),
    );
  }
});

test("non-stair lifts and switchable barriers retain their authored handoff", () => {
  for (const kind of ["lift", "state"] as const) {
    const { geometry, door, lift, landing } = fixture();
    if (kind === "lift") lift.lift_type = 2;
    else landing.obstacles[0]!.state_id = 1;
    prepareStairPassages(geometry);
    assert.deepEqual(door.point_mid, [50, 50]);
    if (kind === "state")
      assert.deepEqual(compileLiftPassageStates(geometry)[0]![0]![0]!.allowed_states, [2]);
  }
});
