import type { MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import type { JumpEdge } from "./jump-clearance.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import { pointInGameplayPolygon as contains } from "./navigation-anchor.ts";

type Edge = [Point, Point];
interface Receiver {
  polygon: Point[];
  plane: HeightPlane;
}
const EPS = 1e-9;
const cross = (a: Point, b: Point) => a[0] * b[1] - a[1] * b[0];
const minus = (a: Point, b: Point): Point => [a[0] - b[0], a[1] - b[1]];

/** Sweep receiving bonds in top-bound priority order; outside motion areas bindings persist. */
export function takeoffReceiverRegions(
  source: JumpEdge,
  sourcePlane: HeightPlane,
  receivers: Receiver[],
  motionPolygon?: Point[],
): { polygons: MultiPolygon; plane: HeightPlane }[] {
  const origin: Point = [source.a[0], source.a[1] - source.a[2]];
  const along: Point = [source.b[0] - source.a[0], source.b[1] - source.b[2] - origin[1]];
  const length = Math.hypot(...along);
  const forward: Point = [(-15 * along[1]) / length, (15 * along[0]) / length];
  const local = (point: Point): Point => {
    const d = minus(point, origin);
    return [
      (d[0] * along[0] + d[1] * along[1]) / length ** 2,
      (d[0] * forward[0] + d[1] * forward[1]) / 225,
    ];
  };
  const world = (t: number, s: number): Point => [
    origin[0] + along[0] * t + forward[0] * s,
    origin[1] + along[1] * t + forward[1] * s,
  ];
  const polygons = receivers.map((receiver) => receiver.polygon.map(local));
  const area = motionPolygon?.map(local);
  const edges: Edge[] = [];
  for (const polygon of [...polygons, ...(area ? [area] : [])])
    for (let i = 0; i < polygon.length; i++) {
      const a = polygon[i]!,
        b = polygon[(i + 1) % polygon.length]!;
      const delta = minus(b, a);
      let low = 0,
        high = 1;
      for (const axis of [0, 1]) {
        if (Math.abs(delta[axis]!) < EPS) {
          if (a[axis]! < 0 || a[axis]! > 1) high = -1;
        } else {
          const first = -a[axis]! / delta[axis]!,
            second = (1 - a[axis]!) / delta[axis]!;
          low = Math.max(low, Math.min(first, second));
          high = Math.min(high, Math.max(first, second));
        }
      }
      if (high > low + EPS)
        edges.push([
          [a[0] + delta[0] * low, a[1] + delta[1] * low],
          [a[0] + delta[0] * high, a[1] + delta[1] * high],
        ]);
    }
  const cuts = [0, 1, ...edges.flatMap(([a, b]) => [a[0], b[0]])];
  for (let i = 0; i < edges.length; i++)
    for (let j = i + 1; j < edges.length; j++) {
      const [a, b] = edges[i]!,
        [c, d] = edges[j]!;
      const ab = minus(b, a),
        cd = minus(d, c),
        offset = minus(c, a),
        determinant = cross(ab, cd);
      if (Math.abs(determinant) < EPS) continue;
      const t = cross(offset, cd) / determinant,
        u = cross(offset, ab) / determinant;
      if (t > 0 && t < 1 && u > 0 && u < 1) cuts.push(a[0] + ab[0] * t);
    }
  const sorted = cuts
    .sort((a, b) => a - b)
    .filter((value, i, values) => i === 0 || value - values[i - 1]! > EPS);
  const owner = (point: Point) => polygons.findIndex((polygon) => contains(point, polygon));
  const isBond = (edge: Edge, t: number, s: number) => {
    if (!motionPolygon) return true;
    const middle = world(t, s),
      a = world(...edge[0]),
      b = world(...edge[1]);
    const direction = minus(b, a),
      size = Math.hypot(...direction);
    const normal: Point = [-direction[1] / size, direction[0] / size];
    let distance = 1e-4;
    for (const receiver of receivers)
      for (let i = 0; i < receiver.polygon.length; i++) {
        const a = receiver.polygon[i]!,
          b = receiver.polygon[(i + 1) % receiver.polygon.length]!;
        const direction = minus(b, a),
          determinant = cross(normal, direction);
        if (Math.abs(determinant) <= 1e-7) continue;
        const offset = minus(a, middle),
          along = cross(offset, normal) / determinant;
        const crossing = Math.abs(cross(offset, direction) / determinant);
        if (along >= -1e-7 && along <= 1 + 1e-7 && crossing > 1e-7)
          distance = Math.min(distance, crossing * 0.25);
      }
    return [-1, 1].every((side) =>
      contains(
        [middle[0] + side * normal[0] * distance, middle[1] + side * normal[1] * distance],
        motionPolygon,
      ),
    );
  };
  const result: { polygons: MultiPolygon; plane: HeightPlane }[] = [];
  for (let i = 1; i < sorted.length; i++) {
    const low = sorted[i - 1]!,
      high = sorted[i]!,
      middle = (low + high) / 2;
    const events: { at: (t: number) => number; edge?: Edge }[] = [
      { at: (_t: number) => 0 },
      { at: (_t: number) => 1 },
    ];
    for (const [a, b] of edges) {
      if (middle <= Math.min(a[0], b[0]) || middle >= Math.max(a[0], b[0])) continue;
      const at = (t: number) => a[1] + ((b[1] - a[1]) * (t - a[0])) / (b[0] - a[0]);
      if (at(middle) > EPS && at(middle) < 1 - EPS) events.push({ at, edge: [a, b] });
    }
    events.sort((a, b) => a.at(middle) - b.at(middle));
    const boundaries = events.filter(
      (event, j) => j === 0 || event.at(middle) - events[j - 1]!.at(middle) > EPS,
    );
    const initial = owner([middle, 0]);
    let plane: HeightPlane = receivers[initial]?.plane ?? sourcePlane;
    for (let j = 1; j < boundaries.length; j++) {
      const before = boundaries[j - 1]!,
        after = boundaries[j]!;
      const probe: Point = [middle, (before.at(middle) + after.at(middle)) / 2];
      if (j > 1) {
        const previous: Point = [middle, (boundaries[j - 2]!.at(middle) + before.at(middle)) / 2];
        if (
          owner(previous) !== owner(probe) &&
          before.edge &&
          isBond(before.edge, middle, before.at(middle))
        ) {
          const next = owner(probe);
          plane = receivers[next]?.plane ?? [0, 0, 0];
        }
      }
      const polygon = [
        world(low, before.at(low)),
        world(high, before.at(high)),
        world(high, after.at(high)),
        world(low, after.at(low)),
      ];
      result.push({ polygons: [[polygon]], plane });
    }
  }
  return result;
}
