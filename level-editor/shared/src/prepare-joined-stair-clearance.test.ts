import test from "node:test";
import assert from "node:assert/strict";
import type { CompiledAssetGeometry } from "./asset-gameplay.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { prepareJoinedStairClearance } from "./prepare-joined-stair-clearance.ts";
import { supportedFootprintCenters } from "./supported-footprint-centers.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";

const rectangle = (left: number, top: number, right: number, bottom: number): Point[] => [
  [left, top],
  [right, top],
  [right, bottom],
  [left, bottom],
];
function fixture(): CompiledAssetGeometry {
  const world: { inside: Vec3; middle: Vec3; outside: Vec3 }[] = [
    { inside: [101, 110, 2], middle: [100, 110, 0], outside: [90, 110, 0] },
    { inside: [103, 110, 6], middle: [104, 110, 8], outside: [114, 110, 8] },
  ];
  const project = ([x, y, z]: Vec3): Point => [x, y - z];
  const floor = rectangle(100, 100, 104, 120);
  const polygons = [
    rectangle(80, 100, 100, 120),
    floor.map(([x, y]): Point => [x, y - 2 * (x - 100)]),
    rectangle(104, 92, 124, 112),
  ];
  return {
    doors: [],
    motion_data: {
      graph_bytes: [],
      layers: polygons.map((polygon, index) => [
        {
          is_lift: index === 1,
          state_id: 0,
          flags: 0,
          skeleton_segments: [],
          obstacles: [],
          polygon: { points: polygon },
        },
      ]),
    },
    sight_obstacles: [0, 2].map((index) => ({
      points: polygons[index]!.map(([x, y]) => ({
        x,
        y: y + (index === 2 ? 8 : 0),
        z_top: index === 2 ? 8 : 0,
        z_bottom: index === 2 ? 8 : 0,
      })),
      projection_area: [index, index],
      opaque: false,
      solid: true,
      mouse: true,
      show_shadow_polygon: false,
      default_material: 0,
      material_indices: [],
    })),
    lifts: [
      {
        motion_area_index: 1,
        lift_type: 1,
        direction: 0,
        physical_navigation: { plane: [2, 0, -200], boundary: floor, obstacles: [], doors: world },
        doors: world.map((points, index) => ({
          door_type: index === 0 ? 5 : 4,
          active: true,
          locked_pc: false,
          unlockable: false,
          locked_npc_villain: true,
          locked_npc_civilian: false,
          locked_pc_after_patch: false,
          unlockable_after_patch: false,
          locked_npc_villain_after_patch: false,
          locked_npc_civilian_after_patch: false,
          door_sector: { points: [] },
          point_in: project(points.inside),
          point_out: project(points.outside),
          point_mid: project(points.middle),
          sector_in: 1,
          layer_in: 1,
          sector_out: index === 0 ? 0 : 2,
          layer_out: index === 0 ? 0 : 2,
        })),
      },
    ],
  };
}

test("short stairs gain static clearance from height-connected landings", () => {
  const geometry = fixture();
  const doors = structuredClone(geometry.lifts![0]!.doors);
  assert.equal(prepareJoinedStairClearance(geometry), 1);
  const polygon = geometry.motion_data.layers[1]![0]!.precise_polygon!;
  assert.ok(polygon);
  const centers = supportedFootprintCenters([[polygon]], [[polygon]], [5, 2]);
  assert.ok(centers.length);
  for (const door of doors)
    assert.ok(centers.some((region) => pointInGameplayPolygon(door.point_in, region[0]!)));
  assert.ok(geometry.motion_data.graph_bytes!.length > 0);
  assert.deepEqual(
    geometry.lifts![0]!.doors,
    doors,
    "permissions and door semantics remain unchanged",
  );
});

test("missing, raised or displaced landings cannot supply stair clearance", () => {
  for (const change of ["missing", "raised", "raised-anchor", "gap"] as const) {
    const geometry = fixture();
    if (change === "missing") geometry.sight_obstacles.pop();
    if (change === "raised" || change === "raised-anchor")
      for (const point of geometry.sight_obstacles[1]!.points) {
        point.z_top += 20;
        point.z_bottom += 20;
      }
    if (change === "raised-anchor") {
      geometry.lifts![0]!.physical_navigation!.doors[1]!.outside[2] += 20;
      geometry.lifts![0]!.doors[1]!.point_out[1] -= 20;
      for (const point of geometry.motion_data.layers[2]![0]!.polygon.points) point[1] -= 20;
    }
    if (change === "gap") for (const point of geometry.sight_obstacles[1]!.points) point.x += 1;
    const before = structuredClone(geometry.motion_data);
    assert.equal(prepareJoinedStairClearance(geometry), 0, change);
    assert.deepEqual(geometry.motion_data, before);
  }
});

test("landing collision in any state is excluded from unconditional support", () => {
  for (const state of [0, 1, 2]) {
    const geometry = fixture();
    geometry.motion_data.layers[2]![0]!.obstacles.push({
      state_id: state,
      polygon: { points: rectangle(104, 92, 111, 112) },
    });
    assert.equal(prepareJoinedStairClearance(geometry), 0, `state ${state}`);
  }
});

test("recovering a short stair does not narrow existing native walking coverage", () => {
  const geometry = fixture();
  geometry.motion_data.layers[1]![0]!.polygon.points = rectangle(90, 90, 120, 120);
  const before = structuredClone(geometry.motion_data);
  assert.equal(prepareJoinedStairClearance(geometry), 0);
  assert.deepEqual(geometry.motion_data, before);
});

test("a state-dependent landing cannot supply unconditional stair support", () => {
  const geometry = fixture();
  geometry.motion_data.layers[2]![0]!.state_id = 1;
  assert.equal(prepareJoinedStairClearance(geometry), 0);
  geometry.motion_data.layers[2]![0]!.state_id = 0;
  for (const state of ["initial_sight", "applied_sight"] as const) {
    geometry.movement_transitions = [
      {
        id: "landing",
        waypoint: [0, 0],
        sector: 0,
        layer: 0,
        active: true,
        definitive: false,
        apply_polygon: { points: [] },
        no_apply_polygon: { points: [] },
        motion_changes: [],
        [state]: [1],
      },
    ];
    assert.equal(prepareJoinedStairClearance(geometry), 0, state);
  }
});
