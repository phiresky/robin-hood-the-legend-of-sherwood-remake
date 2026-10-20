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

test("long-jump audit warns about backward or tangent takeoff while retaining the authored pair", () => {
  for (const distance of [-60, 0]) {
    const input = pair();
    input.line2.point_a[0] = distance;
    input.line2.point_b[0] = distance;
    const before = structuredClone(input);
    let checked = false;
    const issues = auditCompiledJump(input, () => {
      checked = true;
      return [];
    });
    assert.equal(issues.length, 1);
    assert.match(issues[0]!, /takeoff normal does not face the destination/);
    assert.ok(checked, "orientation warnings must not skip the flight audit");
    assert.deepEqual(input, before);
  }
  assert.deepEqual(
    auditCompiledJump(pair(), () => []),
    [],
  );
});

test("long-jump orientation audit checks both endpoints and both directions", () => {
  const input = pair();
  // The first normal faces the landing, but the return edge faces away.
  input.line2.point_a = [60, 120, 12];
  input.line2.point_b = [60, 100, 12];
  assert.match(auditCompiledJump(input, () => []).join("\n"), /takeoff normal/);
  // A different edge faces the source at one endpoint but crosses it at the other.
  input.line2.point_a = [-10, 0, 12];
  input.line2.point_b = [60, 100, 12];
  assert.match(auditCompiledJump(input, () => []).join("\n"), /takeoff normal/);
  input.jump_long = false;
  assert.doesNotMatch(auditCompiledJump(input, () => []).join("\n"), /takeoff normal/);
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
  const unequal = pair();
  unequal.line2.point_a[1]++;
  const issues = auditCompiledJump(unequal, createJumpClearance([obstacle]));
  assert.equal(issues.length, 2);
  assert.match(issues[0]!, /landings can miss/);
  assert.match(issues[1]!, /intersects/);
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
  input.line2.point_a[1]++;
  const issues = auditCompiledJump(input, createJumpClearance([]));
  assert.equal(issues.length, 2);
  assert.match(issues[0]!, /landings can miss/);
  assert.match(issues[1]!, /unverified/);
});
