import test from "node:test";
import assert from "node:assert/strict";
import { createJumpReceiverSupportAudit } from "./audit-jump-receiver-support.ts";
import type { JumpLinePair, JumpZone, SightObstacle } from "./level.ts";

const pair: JumpLinePair = {
  line1: { point_a: [0, 0, 20], point_b: [10, 0, 20], jump_zone_index: 1 },
  line2: { point_a: [10, 100, 20], point_b: [0, 100, 20], jump_zone_index: 0 },
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
function face(sector: number, low = -1, high = 11): SightObstacle {
  return {
    projection_area: [sector, 1],
    points: [
      [low, -1],
      [high, -1],
      [high, 1],
      [low, 1],
    ].map(([x, y]) => ({
      x: x!,
      y: y! + 100 * sector + 20,
      z_top: 20,
      z_bottom: 20,
    })),
    opaque: false,
    solid: false,
    mouse: true,
    show_shadow_polygon: false,
    default_material: 0,
    material_indices: [],
  };
}

test("final jump coverage accepts adjacent receiver pieces but detects an interior gap", () => {
  const left = face(1, -1, 4.001),
    right = face(1, 4.001, 11);
  assert.deepEqual(createJumpReceiverSupportAudit([face(0), left, right], zones)(pair), []);
  right.points
    .filter((p) => p.x === 4.001)
    .forEach((p) => {
      p.x = 4.002;
    });
  const issues = createJumpReceiverSupportAudit([face(0), left, right], zones)(pair);
  assert.equal(issues.length, 1);
  assert.match(issues[0]!, /direction 1 landing span/);
});

test("coverage follows actual source-distance transfer rather than destination endpoint alone", () => {
  const input = structuredClone(pair);
  input.line1.point_b[0] = 12;
  assert.match(
    createJumpReceiverSupportAudit([face(0, -1, 13), face(1)], zones)(input)[0]!,
    /direction 1/,
  );
  assert.deepEqual(pair.line1.point_b, [10, 0, 20]);
});

test("receivers on another layer do not cover an elevated landing", () => {
  const wrong = face(1);
  wrong.projection_area = [1, 2];
  assert.match(createJumpReceiverSupportAudit([face(0), wrong], zones)(pair)[0]!, /direction 1/);
});

test("implicit ground remains available beside other receivers in its sector", () => {
  const ground = structuredClone(pair);
  for (const line of [ground.line1, ground.line2]) {
    line.point_a[2] = 0;
    line.point_b[2] = 0;
  }
  assert.deepEqual(createJumpReceiverSupportAudit([], zones)(ground), []);
  assert.equal(createJumpReceiverSupportAudit([], zones)(pair).length, 2);
  const missing = face(1, 20, 30);
  assert.deepEqual(createJumpReceiverSupportAudit([missing], zones)(ground), []);
});
