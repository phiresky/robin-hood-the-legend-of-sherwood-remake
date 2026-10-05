import test from "node:test";
import assert from "node:assert/strict";
import { compilePhysicalStair, type PhysicalStairInput } from "./compile-physical-stair.ts";
import { planeHeight } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

const rectangle = (x0: number, y0: number, x1: number, y1: number): Vec3[] => [
  [x0, y0, y0 - 200],
  [x1, y0, y0 - 200],
  [x1, y1, y1 - 200],
  [x0, y1, y1 - 200],
];
const fixture = (): PhysicalStairInput => ({
  surfaces: [
    { polygon: rectangle(380, 300, 420, 350), holes: [rectangle(385, 320, 390, 325)] },
    { polygon: rectangle(380, 350, 420, 400), holes: [] },
  ],
  obstacles: [{ motionObstacle: 7, polygon: rectangle(395, 349, 405, 351) }],
  doors: [
    { inside: [400, 310, 110], middle: [400, 300, 100], outside: [400, 290, 100] },
    { inside: [400, 390, 190], middle: [400, 400, 200], outside: [400, 410, 200] },
  ],
});

test("physical stair compilation retains an edge-on floor and distinct door identities", () => {
  const input = fixture();
  const { navigation, holes } = compilePhysicalStair(input);
  assert.deepEqual(navigation.plane, [0, 1, -200]);
  assert.equal(holes.length, 1);
  assert.equal(navigation.obstacles[0]!.motion_obstacle, 7);
  assert.deepEqual(navigation.doors, input.doors);
  assert.ok(
    navigation.boundary.every((point) => point[1] - planeHeight(navigation.plane, point) === 200),
  );
  const area =
    navigation.boundary.reduce((sum, p, i) => {
      const q = navigation.boundary[(i + 1) % navigation.boundary.length]!;
      return sum + p[0] * q[1] - p[1] * q[0];
    }, 0) / 2;
  assert.equal(area, 4000);
  assert.equal(
    navigation.doors[0]!.middle[1] - navigation.doors[0]!.middle[2],
    navigation.doors[1]!.middle[1] - navigation.doors[1]!.middle[2],
  );
  navigation.doors[0]!.inside[0] = 0;
  assert.equal(
    input.doors[0]!.inside[0],
    400,
    "compiled output must not mutate asset-local anchors",
  );
});

test("placed rotations and elevations preserve the physical floor and collision", () => {
  for (const degrees of [0, 37, 90, 180, 270])
    for (const elevation of [0, 40]) {
      const angle = (degrees * Math.PI) / 180;
      const move = ([x, y, z]: Vec3): Vec3 => [
        x * Math.cos(angle) - y * Math.sin(angle) + 600,
        x * Math.sin(angle) + y * Math.cos(angle) + 600,
        z + elevation,
      ];
      const original = fixture();
      const placed: PhysicalStairInput = {
        surfaces: original.surfaces.map((surface) => ({
          polygon: surface.polygon.map(move),
          holes: surface.holes.map((hole) => hole.map(move)),
        })),
        obstacles: original.obstacles.map((obstacle) => ({
          ...obstacle,
          polygon: obstacle.polygon.map(move),
        })),
        doors: original.doors.map((door) => ({
          inside: move(door.inside),
          middle: move(door.middle),
          outside: move(door.outside),
        })),
      };
      const result = compilePhysicalStair(placed);
      for (const p of placed.surfaces.flatMap((surface) => surface.polygon))
        assert.ok(Math.abs(planeHeight(result.navigation.plane, [p[0], p[1]]) - p[2]) < 1e-6);
      assert.deepEqual(result.navigation.doors, placed.doors);
      assert.equal(result.holes.length, 1);
      assert.equal(result.navigation.obstacles[0]!.motion_obstacle, 7);
    }
});

test("disconnected floors, inconsistent heights and unsupported door anchors are rejected", () => {
  const disconnected = fixture();
  disconnected.surfaces[1]!.polygon = rectangle(380, 360, 420, 400);
  assert.throws(() => compilePhysicalStair(disconnected), /connected floor/);
  const warped = fixture();
  warped.surfaces[0]!.polygon[0]![2] += 1;
  assert.throws(() => compilePhysicalStair(warped), /planar/);
  const hole = fixture();
  hole.doors[0]!.inside = [387, 322, 122];
  assert.throws(() => compilePhysicalStair(hole), /floor support/);
  const off = fixture();
  off.doors[0]!.middle = [300, 300, 100];
  assert.throws(() => compilePhysicalStair(off), /floor support/);
  const invalid = fixture();
  invalid.obstacles[0]!.motionObstacle = -1;
  assert.throws(() => compilePhysicalStair(invalid), /identity/);
});
