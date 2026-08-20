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
  spacingFraction = 1,
): DirectionalStamp[] {
  const stamps: DirectionalStamp[] = [];
  if (points.length < 2 || segments.length === 0) return stamps;

  // The drawn polyline is a guide: each stretch snaps to the closest
  // art direction and the wall walks in that direction with exact butt-joint
  // steps (slice extent in screen X, or Y for steep art), chaining from the
  // previous stamp so joints stay seamless even when the path slope differs
  // from the art slope.
  let cx = points[0]![0];
  let cy = points[0]![1];
  for (let i = 0; i + 1 < points.length; i++) {
    const [tx, ty] = points[i + 1]!;
    const pdx = tx - cx;
    const pdy = ty - cy;
    const plen = Math.hypot(pdx, pdy);
    if (plen < 4) continue;
    const angle = (Math.atan2(pdy, pdx) * 180) / Math.PI;
    let best = segments[0]!;
    for (const s of segments) {
      if (angularDist(s.directionDeg, angle) < angularDist(best.directionDeg, angle)) best = s;
    }
    // art direction, flipped if needed to point along the path heading
    let artDeg = best.directionDeg;
    if (Math.cos(((artDeg - angle) * Math.PI) / 180) < 0) artDeg += 180;
    const artRad = (artDeg * Math.PI) / 180;
    const ux = Math.cos(artRad);
    const uy = Math.sin(artRad);
    const step =
      (Math.abs(normDeg(artDeg)) <= 55
        ? best.size[0] / Math.max(0.35, Math.abs(ux))
        : best.size[1] / Math.max(0.35, Math.abs(uy))) * spacingFraction;
    // walk in art direction until progress along the path stretch is covered
    const pux = pdx / plen;
    const puy = pdy / plen;
    let progress = 0;
    while (progress < plen) {
      stamps.push({
        asset: best.id,
        pos: [Math.round(cx - best.anchor[0]), Math.round(cy - best.anchor[1])],
        sortY: cy,
      });
      cx += ux * step;
      cy += uy * step;
      progress = (cx - points[i]![0]) * pux + (cy - points[i]![1]) * puy;
    }
  }
  // closing stamp at the chain's end
  const lastSeg = segments[0];
  if (lastSeg) {
    stamps.push({
      asset: stamps.length ? stamps[stamps.length - 1]!.asset : lastSeg.id,
      pos: [
        Math.round(cx - (segments.find((s) => s.id === stamps.at(-1)?.asset) ?? lastSeg).anchor[0]),
        Math.round(cy - (segments.find((s) => s.id === stamps.at(-1)?.asset) ?? lastSeg).anchor[1]),
      ],
      sortY: cy,
    });
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
