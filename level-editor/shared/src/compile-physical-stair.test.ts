import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  compilePhysicalStair,
  compilePhysicalStairArea,
  type PhysicalStairInput,
} from "./compile-physical-stair.ts";
import { heightPlane, planeHeight } from "./gameplay-plane.ts";
import { compilePhysicalTransitionObstacles } from "./compile-movement-transitions.ts";
import type { Point } from "./level.ts";
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

test("physical changing barriers retain fractional geometry and state identities through emission", () => {
  for (const degrees of [0, 37, 90, 180, 270]) {
    const angle = (degrees * Math.PI) / 180;
    const move = ([x, y, z]: Vec3): Vec3 => [
      600 + x * Math.cos(angle) - y * Math.sin(angle),
      600 + x * Math.sin(angle) + y * Math.cos(angle),
      z + 40,
    ];
    const floor = rectangle(380, 300, 420, 400).map(move);
    const plane = heightPlane(floor);
    const xy = (points: Vec3[]): Point[] => points.map(([x, y]) => [x, y]);
    const thin = xy(rectangle(395.1, 349.1, 395.2, 351.1).map(move));
    const ring = xy(rectangle(390, 360, 410, 380).map(move));
    const hole = xy(rectangle(395, 365, 405, 375).map(move));
    const changing = compilePhysicalTransitionObstacles(xy(floor), [], plane, [
      { transition: "door", applied: false, polygon: thin, holes: [], plane },
      { transition: "door", applied: true, polygon: ring, holes: [hole], plane },
      { transition: "copy", applied: false, polygon: thin, holes: [], plane },
    ]);
    assert.deepEqual(
      [...changing.pairs],
      [
        ["door", 0],
        ["copy", 1],
      ],
    );
    const result = compilePhysicalStairArea({
      surfaces: [{ polygon: floor, holes: [] }],
      doors: [],
      obstacles: changing.obstacles.map((obstacle) => ({
        stateId: obstacle.state_id,
        polygon: obstacle.polygon.points.map(([x, y]): Vec3 => [x, y, planeHeight(plane, [x, y])]),
      })),
    });
    const area = (polygon: Point[]) =>
      Math.abs(
        polygon.reduce((sum, p, i) => {
          const q = polygon[(i + 1) % polygon.length]!;
          return sum + p[0] * q[1] - q[0] * p[1];
        }, 0),
      ) / 2;
    for (const [state, expected] of [
      [1, 0.2],
      [2, 300],
      [4, 0.2],
    ]) {
      const total = result.navigation.obstacles.reduce(
        (sum, obstacle) =>
          sum +
          (result.area.obstacles[obstacle.motion_obstacle]!.state_id === state
            ? area(obstacle.polygon)
            : 0),
        0,
      );
      assert.ok(Math.abs(total - expected!) < 1e-6, `${degrees} degrees, state ${state}: ${total}`);
    }
    assert.equal(changing.initial.length, 2);
  }
});

test("physical clipping dust does not allocate a collision control", () => {
  const result = compilePhysicalTransitionObstacles(
    [
      [3000, 3000],
      [3100, 3000],
      [3100, 3100],
      [3000, 3100],
    ],
    [],
    [0, 0, 0],
    [
      {
        transition: "dust",
        applied: false,
        plane: [0, 0, 0],
        holes: [],
        polygon: [
          [3010, 3010],
          [3011, 3010],
          [3011, 3010 + 1e-10],
        ],
      },
      {
        transition: "barrier",
        applied: false,
        plane: [0, 0, 0],
        holes: [],
        polygon: [
          [3020, 3020],
          [3030, 3020],
          [3030, 3030],
          [3020, 3030],
        ],
      },
    ],
  );
  assert.deepEqual([...result.pairs], [["barrier", 0]]);
  assert.equal(result.obstacles.length, 1);
  assert.equal(result.obstacles[0]!.state_id, 1);
});

test("physical volume barriers slice the real stair height before projection", () => {
  const floor = rectangle(380, 300, 420, 400);
  const plane = heightPlane(floor);
  const polygon: Point[] = floor.map(([x, y]) => [x, y]);
  const changing = compilePhysicalTransitionObstacles(polygon, [], plane, [
    {
      transition: "shutter",
      applied: false,
      polygon,
      holes: [],
      plane: [0, 0, 150],
      terrainVolume: { polygon, holes: [], plane: [0, 0, 150], below: 10, above: 10 },
    },
  ]);
  assert.equal(changing.obstacles.length, 1);
  const points = changing.obstacles[0]!.polygon.points;
  assert.equal(Math.min(...points.map(([, y]) => y)), 340);
  assert.equal(Math.max(...points.map(([, y]) => y)), 360);
  assert.equal(changing.obstacles[0]!.state_id, 1);
  assert.ok(points.every((point) => point[1] - planeHeight(plane, point) === 200));
});

test("physical area emission allocates holes and live obstacle identities together", () => {
  const input = fixture();
  const result = compilePhysicalStairArea({
    ...input,
    obstacles: [
      { stateId: 1, polygon: input.obstacles[0]!.polygon },
      { stateId: 2, polygon: rectangle(395, 359, 405, 361) },
    ],
  });
  assert.equal(result.area.obstacles.length, 3);
  assert.deepEqual(
    result.area.obstacles.map((obstacle) => obstacle.state_id),
    [0, 1, 2],
  );
  assert.deepEqual(
    result.navigation.obstacles.map((obstacle) => obstacle.motion_obstacle),
    [0, 1, 2],
  );
  for (const [index, obstacle] of result.navigation.obstacles.entries())
    assert.deepEqual(
      result.area.obstacles[index]!.polygon.points,
      obstacle.polygon.map((point) => [
        Math.round(point[0]),
        Math.round(point[1] - planeHeight(result.navigation.plane, point)),
      ]),
    );
  assert.ok(result.area.polygon.points.every(([, y]) => y === 200));
  assert.ok(result.area.polygon.points.length >= 3);
});

test("native edge-on traversal fixture is emitted by the shared compiler", () => {
  const input = fixture();
  const result = compilePhysicalStairArea({
    surfaces: input.surfaces.map((surface) => ({ ...surface, holes: [] })),
    obstacles: [],
    doors: input.doors,
  });
  const expected: unknown = JSON.parse(
    readFileSync(new URL("../test-fixtures/physical-stair-area.json", import.meta.url), "utf8"),
  );
  assert.deepEqual(result, expected);
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
  assert.throws(
    () => compilePhysicalStair(hole),
    /floor support: door 0 inside at \[387,322,122\]/,
  );
  const off = fixture();
  off.doors[0]!.middle = [300, 300, 100];
  assert.throws(() => compilePhysicalStair(off), /floor support: door 0 middle at \[300,300,100\]/);
  const invalid = fixture();
  invalid.obstacles[0]!.motionObstacle = -1;
  assert.throws(() => compilePhysicalStair(invalid), /identity/);
});
