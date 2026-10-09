import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import { prepareDirectStairApproaches } from "./prepare-direct-stair-approaches.ts";
import { compileLiftApproaches } from "./compile-lift-approaches.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";

function fixture() {
  const { asset_geometry: geometry }: { asset_geometry: CompiledAssetGeometry } = JSON.parse(
    readFileSync(
      new URL("../../../crates/robin_engine/tests/fixtures/asset-lift.level.json", import.meta.url),
      "utf8",
    ),
  );
  const lift = geometry.lifts![0]!;
  lift.motion_area_index = 0;
  lift.lift_type = 1;
  const area = geometry.motion_data.layers[0]![0]!;
  area.obstacles = [];
  area.polygon.points = [
    [0, 0],
    [12, 0],
    [12, 100],
    [0, 100],
  ];
  geometry.motion_data.layers = [[area]];
  lift.doors.forEach((door, i) => {
    door.sector_in = door.sector_out = 0;
    door.layer_in = door.layer_out = 0;
    door.point_in = i === 0 ? [2, 5] : [10, 95];
    door.point_mid = i === 0 ? [2, -5] : [10, 105];
    door.point_out = [...door.point_mid];
  });
  return { geometry, lift, area };
}

test("narrow unobstructed stairs prepare mutually direct-reachable inner approaches", () => {
  for (const reverse of [false, true]) {
    const { geometry, lift, area } = fixture();
    if (reverse) area.polygon.points.reverse();
    const motion = structuredClone(geometry.motion_data);
    const outer = lift.doors.map((d) => [d.point_mid, d.point_out]);
    assert.equal(prepareDirectStairApproaches(geometry).size, 2);
    assert.deepEqual(
      lift.doors.map((d) => d.point_in),
      [
        [6, 5],
        [6, 95],
      ],
    );
    const allowed = prepareCorridorStates([], area.polygon.points);
    for (const a of lift.doors)
      for (const b of lift.doors) assert.deepEqual(allowed(a.point_in, b.point_in), [0]);
    assert.deepEqual(geometry.motion_data, motion);
    assert.deepEqual(
      lift.doors.map((d) => [d.point_mid, d.point_out]),
      outer,
    );
    geometry.warnings = [];
    compileLiftApproaches(geometry);
    assert.ok(!geometry.warnings.some((w) => /no actor-sized in approach/.test(w)));
  }
});

test("narrow route preparation excludes blockers, concavity and partial endpoint recovery", () => {
  for (const kind of ["blocked", "concave", "remote", "wide", "no-space", "ladder"] as const) {
    const { geometry, lift, area } = fixture();
    if (kind === "blocked")
      area.obstacles.push({
        state_id: 1,
        polygon: {
          points: [
            [0, 40],
            [12, 40],
            [12, 50],
            [0, 50],
          ],
        },
      });
    if (kind === "concave") area.polygon.points.splice(2, 0, [8, 50]);
    if (kind === "remote") lift.doors[1]!.point_in = [500, 500];
    if (kind === "wide") {
      area.polygon.points = area.polygon.points.map(([x, y]): Point => [x * 3, y]);
      lift.doors.forEach((door) => {
        door.point_in[0] = 12;
      });
    }
    if (kind === "no-space")
      area.polygon.points = area.polygon.points.map(([x, y]): Point => [x * 0.5, y]);
    if (kind === "ladder") lift.lift_type = 2;
    const before = structuredClone(geometry);
    assert.equal(prepareDirectStairApproaches(geometry).size, 0, kind);
    assert.deepEqual(geometry, before, kind);
  }
});

test("convex stairs recover external endpoints using full-box clearance when available", () => {
  const { geometry, lift, area } = fixture();
  area.polygon.points = area.polygon.points.map(([x, y]): Point => [x * 3, y]);
  lift.doors[0]!.point_in = [-2, 5];
  lift.doors[1]!.point_in = [38, 95];
  const motion = structuredClone(geometry.motion_data);
  const compiled = structuredClone(geometry);
  assert.equal(
    prepareDirectStairApproaches(geometry).size,
    0,
    "full-box routes need no inset exception",
  );
  assert.deepEqual(
    lift.doors.map((door) => door.point_in),
    [
      [7, 5],
      [29, 95],
    ],
  );
  assert.deepEqual(geometry.motion_data, motion);
  compileLiftApproaches(compiled);
  assert.deepEqual(
    compiled.lifts![0]!.doors.map((door) => door.point_in),
    [
      [7, 5],
      [29, 95],
    ],
  );
  assert.ok(!compiled.warnings?.some((warning) => /no actor-sized in approach/.test(warning)));
  assert.ok(compiled.warnings?.some((warning) => /no actor-sized out approach/.test(warning)));
});

test("a narrow projected entrance can turn inward instead of extending past its floor", () => {
  const { geometry, lift, area } = fixture();
  area.polygon.points = [
    [470, 368],
    [524, 366],
    [485, 372],
    [399, 379],
  ];
  lift.doors[0]!.point_mid = [461, 371];
  lift.doors[0]!.point_in = [464, 371];
  lift.doors[1]!.point_mid = [483, 371];
  lift.doors[1]!.point_in = [484, 370];
  assert.equal(prepareDirectStairApproaches(geometry).size, 2);
  const allowed = prepareCorridorStates([], area.polygon.points);
  for (const a of lift.doors)
    for (const b of lift.doors) assert.deepEqual(allowed(a.point_in, b.point_in), [0]);
  assert.ok(lift.doors[1]!.point_in[0] < lift.doors[1]!.point_mid[0]);
});
