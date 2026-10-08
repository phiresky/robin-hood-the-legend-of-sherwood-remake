import type { JumpLinePair } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import type { createJumpClearance, JumpEdge } from "./jump-clearance.ts";

/** Inspect the integer geometry the runtime receives, including authored pairs. */
export function auditCompiledJump(
  pair: JumpLinePair,
  clearance: ReturnType<typeof createJumpClearance>,
): string[] {
  const { line1, line2 } = pair;
  // Landing translation uses distance along the source vector in both directions.
  if (
    [0, 1].some(
      (axis) =>
        line1.point_b[axis]! - line1.point_a[axis]! !== line2.point_a[axis]! - line2.point_b[axis]!,
    )
  )
    return [
      "opposing edges differ after movement-grid rounding; runtime landings can miss the authored edge",
    ];
  const world = ([x, y, z]: Vec3): Vec3 => [x, y + z, z];
  const edges: [JumpEdge, JumpEdge] = [line1, line2].map((line) => ({
    zone: String(line.jump_zone_index),
    a: world(line.point_a),
    b: world(line.point_b),
  })) as [JumpEdge, JumpEdge];
  try {
    return clearance(edges, pair.jump_long).length
      ? ["solid geometry intersects the exported flight or takeoff; connection retained for review"]
      : [];
  } catch (error) {
    if (!(error instanceof Error)) throw error;
    return [`flight clearance is unverified: ${error.message}`];
  }
}
