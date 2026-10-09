import test from "node:test";
import assert from "node:assert/strict";
import { auditCompiledJump } from "./audit-compiled-jump.ts";
import { createJumpClearance } from "./jump-clearance.ts";
import type { JumpLinePair, SightObstacle } from "./level.ts";

function pair(): JumpLinePair {
  return {
    line1: { point_a: [0, 100, 12], point_b: [0, 0, 12], jump_zone_index: 1 },
    line2: { point_a: [60, 0, 12], point_b: [60, 100, 12], jump_zone_index: 0 },
    jump_long: true,
  };
}

test("integer jump audit detects unequal landing vectors without changing the pair", () => {
  const input = pair();
  input.line2.point_a[1]++;
  const before = structuredClone(input);
  assert.match(auditCompiledJump(input, createJumpClearance([]))[0]!, /landings can miss/);
  assert.deepEqual(input, before);
});

test("integer jump audit checks fractional solids in world coordinates", () => {
  const obstacle: SightObstacle = {
    points: [
      [-5, 0],
      [10, 0],
      [10, 120],
      [-5, 120],
    ].map(([x, y]) => ({
      x: x!,
      y: y!,
      z_bottom: 0,
      z_top: 12.001,
    })),
    solid: true,
    opaque: true,
    mouse: true,
    show_shadow_polygon: false,
    projection_area: null,
    default_material: 0,
    material_indices: [],
  };
  assert.match(auditCompiledJump(pair(), createJumpClearance([obstacle]))[0]!, /intersects/);
  obstacle.points.forEach((point) => {
    point.z_top = 11;
  });
  assert.deepEqual(auditCompiledJump(pair(), createJumpClearance([obstacle])), []);
});

test("integer flight audit resolves source receivers through the opposite destination references", () => {
  const checked: string[][] = [];
  const clearance: ReturnType<typeof createJumpClearance> = (edges) => {
    checked.push(edges.map((edge) => edge.zone));
    return [];
  };
  auditCompiledJump(pair(), clearance);
  auditCompiledJump(pair(), clearance, ["placed-left", "placed-right"]);
  assert.deepEqual(checked, [
    ["0", "1"],
    ["placed-left", "placed-right"],
  ]);
});

test("unsupported flight geometry is reported without rejecting best-effort export", () => {
  const input = pair();
  input.line1.point_b[2] = 13;
  assert.match(auditCompiledJump(input, createJumpClearance([]))[0]!, /unverified/);
});
