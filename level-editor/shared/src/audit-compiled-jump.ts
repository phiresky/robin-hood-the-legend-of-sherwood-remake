import type { JumpLinePair } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import type { createJumpClearance, JumpEdge } from "./jump-clearance.ts";

/** Inspect the integer geometry the runtime receives, including authored pairs. */
export function auditCompiledJump(
  pair: JumpLinePair,
  clearance: ReturnType<typeof createJumpClearance>,
  zoneIds?: readonly [string, string],
): string[] {
  const { line1, line2 } = pair;
  const issues: string[] = [];
  // Landing translation uses distance along the source vector in both directions.
  if (
    [0, 1].some(
      (axis) =>
        line1.point_b[axis]! - line1.point_a[axis]! !== line2.point_a[axis]! - line2.point_b[axis]!,
    )
  )
    issues.push(
      "opposing edges differ after movement-grid rounding; runtime landings can miss the authored edge",
    );
  if (
    pair.jump_long &&
    [
      [line1, line2],
      [line2, line1],
    ].some(([source, destination]) => {
      const dx = source!.point_b[0] - source!.point_a[0];
      const dy = source!.point_b[1] - source!.point_a[1];
      return [
        [source!.point_a, destination!.point_b],
        [source!.point_b, destination!.point_a],
      ].some(([from, to]) => -dy * (to![0] - from![0]) + dx * (to![1] - from![1]) <= 0);
    })
  )
    issues.push(
      "long-jump takeoff normal does not face the destination across the full span; review placed endpoint order",
    );
  const world = ([x, y, z]: Vec3): Vec3 => [x, y + z, z];
  const edges: [JumpEdge, JumpEdge] = [line1, line2].map((line, index) => ({
    // Each line stores its destination zone; the opposite line identifies
    // the receiver under this source edge.
    zone: zoneIds?.[index] ?? String((index === 0 ? line2 : line1).jump_zone_index),
    a: world(line.point_a),
    b: world(line.point_b),
  })) as [JumpEdge, JumpEdge];
  try {
    // Clearance sweeps each source vector from the opposite endpoint, including
    // the actual landing span when the authored destination vector differs.
    if (clearance(edges, pair.jump_long).length)
      issues.push(
        "solid geometry intersects the exported flight or takeoff; connection retained for review",
      );
  } catch (error) {
    if (!(error instanceof Error)) throw error;
    issues.push(`flight clearance is unverified: ${error.message}`);
  }
  return issues;
}
