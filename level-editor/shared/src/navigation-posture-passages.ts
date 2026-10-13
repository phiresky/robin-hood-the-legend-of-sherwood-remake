import polygonClipping from "polygon-clipping";
import type { NavigationConnection } from "./assemble-navigation-joins.ts";
import type { NavigationAnchorArea } from "./navigation-anchor.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import type { Point } from "./level.ts";
import { prepareFoldedNavigationPassage } from "./folded-navigation-passage.ts";

/** Find collision-checked crossings on an explicitly matched projected seam. */
export function navigationPosturePassages(
  connection: NavigationConnection,
  from: NavigationAnchorArea,
  to: NavigationAnchorArea,
): { outside: Point; middle: Point; inside: Point }[] {
  if (from.coordinateSpace === "world" || to.coordinateSpace === "world")
    throw new Error("Posture passages require projected movement contours");
  const [a, b] = connection.from.edge;
  const dx = b[0] - a[0],
    dy = b[1] - b[2] - (a[1] - a[2]);
  const length = Math.hypot(dx, dy);
  if (length < 1e-6) return [];
  const tangent: Point = [dx / length, dy / length];
  const orientation = Math.sign(1 + from.plane[1]);
  const normal: Point = [-tangent[1] * orientation, tangent[0] * orientation];
  const depth = half[0] * Math.abs(normal[0]) + half[1] * Math.abs(normal[1]) + 1;
  const margin = half[0] * Math.abs(tangent[0]) + half[1] * Math.abs(tangent[1]) + 1;
  const start = connection.overlap[0] * length + margin;
  const end = connection.overlap[1] * length - margin;
  if (end <= start) return [];
  const folded =
    (1 + from.plane[1]) * (1 + to.plane[1]) < 0
      ? prepareFoldedNavigationPassage(from, to)
      : undefined;
  const obstacles = (area: NavigationAnchorArea) =>
    area.blockers.map((points) => ({
      polygon: { points },
      state_id: 0,
    }));
  const fromClear = prepareCorridorStates(obstacles(from), from.polygon);
  const toClear = prepareCorridorStates(obstacles(to), to.polygon);
  // A supporting volume can leave a hole in the lower floor while its top is
  // walkable. Subtract exclusions on their own floor before joining coverage.
  const free = (area: NavigationAnchorArea) =>
    area.blockers.length
      ? polygonClipping.difference([area.polygon], ...area.blockers.map((ring) => [ring]))
      : [[area.polygon]];
  const combined = polygonClipping.union(free(from), free(to));
  const crossings = combined.map(([outer, ...holes]) =>
    prepareCorridorStates(
      holes.map((points) => ({ polygon: { points }, state_id: 0 })),
      outer,
    ),
  );
  const events = [start, end];
  for (const [origin, direction] of [
    [a[0], tangent[0]],
    [a[1] - a[2], tangent[1]],
  ] as const) {
    if (Math.abs(direction) < 1e-12) continue;
    const low = Math.min(origin + direction * start, origin + direction * end);
    const high = Math.max(origin + direction * start, origin + direction * end);
    for (let grid = Math.ceil(low - 0.5); grid + 0.5 < high; grid++) {
      const offset = (grid + 0.5 - origin) / direction;
      if (offset > start && offset < end) events.push(offset);
    }
  }
  events.sort((a, b) => a - b);
  const result: { outside: Point; middle: Point; inside: Point }[] = [];
  const seen = new Set<string>();
  // Enumerate every interval of constant integer projection, including its
  // boundaries. Fixed-distance sampling can miss a one-cell usable opening.
  for (const offset of [
    (start + end) / 2,
    ...events,
    ...events.slice(1).map((value, i) => (value + events[i]!) / 2),
  ]) {
    const middle: Point = [
      Math.round(a[0] + tangent[0] * offset),
      Math.round(a[1] - a[2] + tangent[1] * offset),
    ];
    if (folded) {
      const crossing = folded(middle, normal, tangent, depth);
      if (crossing) {
        const key = `${crossing.outside}/${crossing.inside}`;
        if (!seen.has(key)) {
          seen.add(key);
          result.push(crossing);
        }
      }
      continue;
    }
    const outside: Point = [
      Math.round(middle[0] + normal[0] * depth),
      Math.round(middle[1] + normal[1] * depth),
    ];
    const inside: Point = [
      Math.round(middle[0] - normal[0] * depth),
      Math.round(middle[1] - normal[1] * depth),
    ];
    const key = `${outside}/${inside}`;
    if (seen.has(key)) continue;
    seen.add(key);
    if (!fromClear(outside, outside).length || !toClear(inside, inside).length) continue;
    if (!crossings.some((clear) => clear(outside, middle).length && clear(middle, inside).length))
      continue;
    result.push({ outside, middle, inside });
  }
  return result;
}
