import test from "node:test";
import assert from "node:assert/strict";
import { orientCompiledJump } from "./orient-compiled-jump.ts";
import { auditCompiledJump } from "./audit-compiled-jump.ts";
import type { JumpLinePair } from "./level.ts";

function backward(): JumpLinePair {
  return {
    line1: { point_a: [0, 0, 20], point_b: [0, 10, 30], jump_zone_index: 1 },
    line2: { point_a: [30, 10, 40], point_b: [30, 0, 50], jump_zone_index: 0 },
    jump_long: true,
  };
}

test("placed parallel long jumps face the gap without changing receivers or endpoint heights", () => {
  const input = backward();
  const before = structuredClone(input);
  const output = orientCompiledJump(input);
  assert.deepEqual(input, before);
  assert.deepEqual(output, {
    ...before,
    line1: { ...before.line1, point_a: before.line1.point_b, point_b: before.line1.point_a },
    line2: { ...before.line2, point_a: before.line2.point_b, point_b: before.line2.point_a },
  });
  assert.deepEqual(
    auditCompiledJump(output, () => []),
    [],
  );
  assert.equal(orientCompiledJump(output), output, "already oriented pairs remain unchanged");
});

test("ambiguous, tangent, collapsed and non-forced edges remain authored for the existing audit", () => {
  const unequal = backward();
  unequal.line2.point_a[1]++;
  const tangent = backward();
  tangent.line2.point_a[0] = tangent.line2.point_b[0] = 0;
  const collapsed = backward();
  collapsed.line1.point_b = collapsed.line1.point_a;
  collapsed.line2.point_b = collapsed.line2.point_a;
  const unforced = { ...backward(), jump_long: false };
  for (const input of [unequal, tangent, collapsed, unforced])
    assert.equal(orientCompiledJump(input), input);
});
