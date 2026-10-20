import test from "node:test";
import assert from "node:assert/strict";
import { createJumpDepartureSupportAudit } from "./audit-jump-departure-support.ts";
import type { JumpLinePair, JumpZone, SightObstacle } from "./level.ts";

const pair: JumpLinePair = {
  line1: { point_a: [0, 0, 20], point_b: [10, 0, 20], jump_zone_index: 1 },
  line2: { point_a: [10, 100, 30], point_b: [0, 100, 30], jump_zone_index: 0 },
  jump_long: true,
};
const zones: JumpZone[] = [0, 1].map((sector) => ({
  sector,
  layer: 1,
  helper_needed: false,
  polygon: {
    points: [
      [-20, -20],
      [120, -20],
      [120, 120],
      [-20, 120],
    ],
  },
}));
function face(sector: number, x0: number, y0: number, x1: number, y1: number): SightObstacle {
  const height = sector ? 30 : 20;
  return {
    projection_area: [sector, 1],
    points: [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ].map(([x, y]) => ({
      x: x!,
      y: y! + height,
      z_top: height,
      z_bottom: height,
    })),
    opaque: false,
    solid: false,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  };
}
const upper = () => face(1, -1, 84, 11, 101);

test("departure checks use each source receiver and cover the whole animation strip", () => {
  assert.deepEqual(
    createJumpDepartureSupportAudit([face(0, -1, -1, 11, 16), upper()], zones)(pair),
    [],
  );
  const issues = createJumpDepartureSupportAudit([face(0, -1, -1, 11, 9), upper()], zones)(pair);
  assert.equal(issues.length, 1);
  assert.match(issues[0]!, /direction 1 elevated takeoff animation lacks receiving coverage/);
});

test("adjacent faces support departure, but an enclosed hole must not pass boundary-only checks", () => {
  assert.deepEqual(
    createJumpDepartureSupportAudit(
      [face(0, -1, -1, 5, 16), face(0, 5, -1, 11, 16), upper()],
      zones,
    )(pair),
    [],
  );
  const hole = [
    face(0, -1, -1, 4, 16),
    face(0, 6, -1, 11, 16),
    face(0, 4, -1, 6, 6),
    face(0, 4, 8, 6, 16),
    upper(),
  ];
  assert.match(createJumpDepartureSupportAudit(hole, zones)(pair)[0]!, /direction 1/);
});

test("receivers on another topology cannot prevent a takeoff ground drop", () => {
  const wrong = face(0, -1, -1, 11, 16);
  wrong.projection_area = [0, 2];
  assert.match(createJumpDepartureSupportAudit([wrong, upper()], zones)(pair)[0]!, /direction 1/);
});

test("sloped authored edges still receive a projected departure-coverage check", () => {
  const sloped = structuredClone(pair);
  sloped.line1.point_b[2] = 30;
  const receiver = face(0, -1, -1, 11, 16);
  for (const point of receiver.points) {
    const height = 20 + point.x;
    point.y += height - point.z_top;
    point.z_bottom = point.z_top = height;
  }
  assert.deepEqual(createJumpDepartureSupportAudit([receiver, upper()], zones)(sloped), []);
});

test("ground and non-forced departures are not mistaken for unsupported elevated long jumps", () => {
  const ground = structuredClone(pair);
  for (const line of [ground.line1, ground.line2]) {
    line.point_a[2] = 0;
    line.point_b[2] = 0;
  }
  assert.deepEqual(createJumpDepartureSupportAudit([], zones)(ground), []);
  assert.deepEqual(createJumpDepartureSupportAudit([], zones)({ ...pair, jump_long: false }), []);
  assert.equal(createJumpDepartureSupportAudit([], zones)(pair).length, 2);
});
