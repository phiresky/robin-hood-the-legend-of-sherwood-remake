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

export interface WallSegmentSpec {
  /** library asset id */
  id: string;
  /** asset image size [w, h] */
  size: [number, number];
  anchor: [number, number];
  /**
   * screen-space direction the segment's art runs, in degrees, normalized to
   * (-90, 90] where 0 = horizontal, negative = ascending left→right
   */
  directionDeg: number;
}

export interface DirectionalStamp extends WallStamp {
  asset: string;
}

const normDeg = (d: number) => {
  let a = d % 180;
  if (a > 90) a -= 180;
  if (a <= -90) a += 180;
  return a;
};

const angularDist = (a: number, b: number) => {
  const d = Math.abs(normDeg(a) - normDeg(b));
  return Math.min(d, 180 - d);
};

/**
 * Direction-aware stitching: each path segment uses the wall-segment asset
 * whose art direction is closest to the segment's angle.
 */
export function expandWallRunDirectional(
  points: readonly [number, number][],
  segments: readonly WallSegmentSpec[],
  spacingFraction = 0.55,
): DirectionalStamp[] {
  const stamps: DirectionalStamp[] = [];
  if (points.length < 2 || segments.length === 0) return stamps;

  for (let i = 0; i + 1 < points.length; i++) {
    const [x1, y1] = points[i]!;
    const [x2, y2] = points[i + 1]!;
    const dx = x2 - x1;
    const dy = y2 - y1;
    const len = Math.hypot(dx, dy);
    if (len === 0) continue;
    const angle = (Math.atan2(dy, dx) * 180) / Math.PI;
    let best = segments[0]!;
    for (const s of segments) {
      if (angularDist(s.directionDeg, angle) < angularDist(best.directionDeg, angle)) best = s;
    }
    // step by the segment's extent along the path direction
    const rad = (Math.abs(normDeg(angle)) * Math.PI) / 180;
    const along = Math.max(
      24,
      (best.size[0] * Math.cos(rad) + best.size[1] * Math.sin(rad)) * spacingFraction,
    );
    for (let t = 0; t <= len; t += along) {
      const px = x1 + (dx / len) * t;
      const py = y1 + (dy / len) * t;
      stamps.push({
        asset: best.id,
        pos: [Math.round(px - best.anchor[0]), Math.round(py - best.anchor[1])],
        sortY: py,
      });
    }
  }
  stamps.sort((a, b) => a.sortY - b.sortY);
  return stamps;
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
