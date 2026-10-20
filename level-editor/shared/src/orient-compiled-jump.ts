import type { JumpLinePair } from "./level.ts";

/** Keep a placed parallel long-jump pair's launch normals facing the gap. */
export function orientCompiledJump(pair: JumpLinePair): JumpLinePair {
  if (!pair.jump_long) return pair;
  const { line1, line2 } = pair;
  const dx = line1.point_b[0] - line1.point_a[0];
  const dy = line1.point_b[1] - line1.point_a[1];
  if (dx !== line2.point_a[0] - line2.point_b[0] || dy !== line2.point_a[1] - line2.point_b[1])
    return pair;
  // The runtime launches along (-dy, dx). A changed screen projection can
  // reverse which side of parallel edges contains the paired landing.
  const gapX = line2.point_b[0] - line1.point_a[0];
  const gapY = line2.point_b[1] - line1.point_a[1];
  if (-dy * gapX + dx * gapY >= 0) return pair;
  const reverse = (line: JumpLinePair["line1"]) => ({
    ...line,
    point_a: line.point_b,
    point_b: line.point_a,
  });
  return { ...pair, line1: reverse(line1), line2: reverse(line2) };
}
