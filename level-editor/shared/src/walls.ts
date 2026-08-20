// Wall-run stitching: expand a spline (piecewise-linear for now) into a list
// of stamp positions for a spline-segment asset. Used identically by the
// editor's compose renderer and the pipeline's offline recreation so walls
// look the same in both.
//
// v1 stamps the whole segment image repeatedly along the path with overlap;
// direction-aware segment pieces (corners, ends, embrasures) come later.

export interface WallStamp {
  /** world position of the stamp's asset-local (0,0) */
  pos: [number, number];
  /** draw-order key (world anchor Y) */
  sortY: number;
}

export function expandWallRun(
  points: readonly [number, number][],
  assetSize: [number, number],
  anchor: [number, number],
  spacingFraction = 0.55,
): WallStamp[] {
  const stamps: WallStamp[] = [];
  if (points.length < 2) return stamps;
  const step = Math.max(12, assetSize[0] * spacingFraction);

  let carry = 0;
  for (let i = 0; i + 1 < points.length; i++) {
    const [x1, y1] = points[i]!;
    const [x2, y2] = points[i + 1]!;
    const dx = x2 - x1;
    const dy = y2 - y1;
    const len = Math.hypot(dx, dy);
    if (len === 0) continue;
    let t = carry;
    while (t <= len) {
      const px = x1 + (dx / len) * t;
      const py = y1 + (dy / len) * t;
      stamps.push({
        pos: [Math.round(px - anchor[0]), Math.round(py - anchor[1])],
        sortY: py,
      });
      t += step;
    }
    carry = t - len;
  }
  // ensure the run reaches the final point
  const last = points[points.length - 1]!;
  stamps.push({
    pos: [Math.round(last[0] - anchor[0]), Math.round(last[1] - anchor[1])],
    sortY: last[1],
  });
  // draw back-to-front
  stamps.sort((a, b) => a.sortY - b.sortY);
  return stamps;
}
