import polygonClipping, { type MultiPolygon } from "polygon-clipping";
import earcut, { flatten } from "earcut";
import type { Point } from "./level.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import type { JumpLandingBand } from "./generate-jump-ledges.ts";
import { mergeIntervals, type Interval, type JumpEdge } from "./jump-clearance.ts";

export interface JumpWalkArea {
  plane: HeightPlane;
  polygon: Point[];
  blockers: Point[][];
}

function complement(intervals: Interval[]): Interval[] {
  const result: Interval[] = [];
  let start = 0;
  for (const [low, high] of [...mergeIntervals(intervals), [1, 1] as Interval]) {
    if (low > start) result.push([start, low]);
    start = Math.max(start, high);
  }
  return result;
}

/** Continuous separating-axis test for a walking box swept inward from a ledge. */
function blockedSpans(outside: MultiPolygon, a: Point, delta: Point, inward: Point): Interval[] {
  const result: Interval[] = [];
  for (const region of outside) {
    const flat = flatten(region);
    const indices = earcut(flat.vertices, flat.holes, flat.dimensions);
    for (let i = 0; i < indices.length; i += 3) {
      const triangle = indices
        .slice(i, i + 3)
        .map((index): Point => [flat.vertices[index * 2]!, flat.vertices[index * 2 + 1]!]);
      const axes: Point[] = [
        [1, 0],
        [0, 1],
        [-inward[1], inward[0]],
      ];
      for (let j = 0; j < 3; j++) {
        const p = triangle[j]!,
          q = triangle[(j + 1) % 3]!;
        axes.push([p[1] - q[1], q[0] - p[0]]);
      }
      let low = 0,
        high = 1;
      for (const [x, y] of axes) {
        const points = triangle.map(([px, py]) => px * x + py * y);
        const center = a[0] * x + a[1] * y;
        const velocity = delta[0] * x + delta[1] * y;
        const depth = inward[0] * x + inward[1] * y;
        // Goal authorization shrinks the stock 6-by-4 move box by one unit.
        const radius = 5 * Math.abs(x) + 3 * Math.abs(y);
        const min = Math.min(...points) - center - radius - Math.max(0, depth);
        const max = Math.max(...points) - center + radius - Math.min(0, depth);
        if (Math.abs(velocity) < 1e-9) {
          if (min > 1e-7 || max < -1e-7) {
            high = -1;
            break;
          }
        } else {
          low = Math.max(low, Math.min(min / velocity, max / velocity));
          high = Math.min(high, Math.max(min / velocity, max / velocity));
        }
      }
      if (high > low) result.push([low, high]);
    }
  }
  return mergeIntervals(result);
}

/** Require each retained approach to fit one compiled navigation area in every state. */
export function createJumpWalkingClearance(
  areas: JumpWalkArea[],
  bands: ReadonlyMap<string, JumpLandingBand>,
) {
  const walkingCache = new Map<JumpWalkArea, MultiPolygon>();
  return (edges: [JumpEdge, JumpEdge]): Interval[] => {
    const blocked: Interval[] = [];
    for (const [side, edge] of edges.entries()) {
      const band = bands.get(edge.zone);
      if (!band) continue;
      const a: Point = [edge.a[0], edge.a[1] - edge.a[2]];
      const b: Point = [edge.b[0], edge.b[1] - edge.b[2]];
      const dx = b[0] - a[0],
        dy = b[1] - a[1],
        length = Math.hypot(dx, dy);
      if (length < 1e-4) throw new Error("Jump approach collapses on the movement grid");
      const nx = dy / length,
        ny = -dx / length;
      const normal = 6 * Math.abs(nx) + 4 * Math.abs(ny);
      const along = (6 * Math.abs(dx) + 4 * Math.abs(dy)) / length;
      const at = (t: number, depth: number): Point => [
        a[0] + dx * t + nx * depth,
        a[1] + dy * t + ny * depth,
      ];
      const strip = [
        at(-along / length, -normal),
        at(1 + along / length, -normal),
        at(1 + along / length, band.depth + normal),
        at(-along / length, band.depth + normal),
      ];
      const available: Interval[] = [];
      for (const area of areas) {
        if (!area.plane.every((n, i) => Math.abs(n - band.plane[i]!) < 1e-7)) continue;
        let walking = walkingCache.get(area);
        if (!walking) {
          walking = polygonClipping.difference([area.polygon], ...area.blockers.map((p) => [p]));
          walkingCache.set(area, walking);
        }
        const outside = polygonClipping.difference([strip], walking);
        const exclusions = blockedSpans(outside, a, [dx, dy], [nx * band.depth, ny * band.depth]);
        available.push(...complement(exclusions));
      }
      for (const [lo, hi] of complement(available))
        blocked.push(side === 0 ? [lo, hi] : [1 - hi, 1 - lo]);
    }
    return mergeIntervals(blocked);
  };
}
