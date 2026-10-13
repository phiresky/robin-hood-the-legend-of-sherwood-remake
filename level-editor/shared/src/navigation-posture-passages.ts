import polygonClipping from "polygon-clipping";
import type { NavigationConnection } from "./assemble-navigation-joins.ts";
import type { NavigationAnchorArea } from "./navigation-anchor.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { NAVIGATION_HALF_DIAGONAL as half } from "./navigation-footprint.ts";
import type { Point } from "./level.ts";
import { prepareFoldedNavigationPassage } from "./folded-navigation-passage.ts";
import { closeNavigationSeams } from "./close-navigation-seams.ts";

/** Find collision-checked crossings on an explicitly matched projected seam. */
export function navigationPosturePassages(
  connection: NavigationConnection,
  from: NavigationAnchorArea,
  to: NavigationAnchorArea,
  precise?: readonly [NavigationAnchorArea, NavigationAnchorArea],
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
  // A supporting volume can leave a hole in the lower floor while its top is
  // walkable. Subtract exclusions on their own floor before joining coverage.
  const free = (area: NavigationAnchorArea) =>
    area.blockers.length
      ? polygonClipping.difference([area.polygon], ...area.blockers.map((ring) => [ring]))
      : [[area.polygon]];
  const representations: readonly (readonly [NavigationAnchorArea, NavigationAnchorArea])[] =
    precise ? [[from, to], precise] : [[from, to]];
  const views = representations.map(([source, target]) => ({
    from: prepareCorridorStates(obstacles(source), source.polygon),
    to: prepareCorridorStates(obstacles(target), target.polygon),
    // Independently clipped sides of an already matched seam can differ by
    // a few fixed-point units. This export-only tolerance is below a pixel;
    // endpoint authorization still checks each unchanged floor separately.
    crossings: closeNavigationSeams(
      polygonClipping.union(free(source), free(target)),
      4 / 1048576,
    ).map(([outer, ...holes]) =>
      prepareCorridorStates(
        holes.map((points) => ({ polygon: { points }, state_id: 0 })),
        outer,
      ),
    ),
  }));
  const fromClear = (a: Point, b: Point) =>
    views.every((view) => view.from(a, b).length) ? [0] : [];
  const toClear = (a: Point, b: Point) => (views.every((view) => view.to(a, b).length) ? [0] : []);
  const crossings = [
    (a: Point, b: Point) =>
      views.every((view) => view.crossings.some((clear) => clear(a, b).length)) ? [0] : [],
  ];
  const destinationChecks = [views.at(-1)!.from, views.at(-1)!.to].map(
    (clear) => (point: Point) =>
      [-1, 1].every((dx) =>
        [-1, 1].every((dy) => {
          const shifted: Point = [point[0] + dx, point[1] + dy];
          return clear(shifted, shifted).length > 0;
        }),
      ),
  );
  const usableDestinations = (candidate: { outside: Point; inside: Point }) =>
    destinationChecks[0]!(candidate.outside) && destinationChecks[1]!(candidate.inside);
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
  const middles = new Map<string, Point>();
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
    middles.set(middle.join(","), middle);
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
  const perpendicular = [...result];
  if (!result.some(usableDestinations) && !folded) {
    // Short oblique landings may have usable approaches away from the seam's
    // perpendicular. Search only a footprint-sized neighborhood during export,
    // retaining the same floor and swept-corridor checks for both legs.
    const alongOffsets = [
      0,
      ...Array.from({ length: Math.ceil(margin) }, (_, i) => [i + 1, -i - 1]).flat(),
    ];
    const endpointChecks = [fromClear, toClear].map((clear) => {
      const cache = new Map<string, boolean>();
      return (point: Point) => {
        const key = point.join(",");
        if (!cache.has(key)) cache.set(key, clear(point, point).length > 0);
        return cache.get(key)!;
      };
    });
    for (const middle of middles.values())
      for (const clear of crossings) {
        // Off-axis approaches must not travel beyond a receiving edge before
        // the sector handoff. Rounding a seam can put its midpoint outside it.
        if (
          [from, to].some(
            (area) =>
              area.receivingFloors &&
              !area.receivingFloors.some((floor) =>
                pointInGameplayPolygon(middle, floor.polygon, true),
              ),
          )
        )
          continue;
        const locate = (side: number): Point | undefined => {
          // Prefer full-box destinations over transit-only inset positions.
          // Runtime movement and click authorization use different footprints.
          for (const destination of [true, false])
            for (let distance = 1; distance <= Math.ceil(depth * 2); distance++)
              for (const along of alongOffsets) {
                const sign = side === 0 ? 1 : -1;
                const point: Point = [
                  Math.round(middle[0] + sign * normal[0] * distance + tangent[0] * along),
                  Math.round(middle[1] + sign * normal[1] * distance + tangent[1] * along),
                ];
                if (
                  endpointChecks[side]!(point) &&
                  (!destination || destinationChecks[side]!(point)) &&
                  clear(point, middle).length
                )
                  return point;
              }
          return undefined;
        };
        const outside = locate(0),
          inside = locate(1);
        if (!outside || !inside) continue;
        const key = `${outside}/${inside}`;
        if (seen.has(key)) continue;
        seen.add(key);
        result.push({ outside, middle, inside });
      }
  }
  if (folded) return result;
  const destinations = result.filter(usableDestinations);
  return destinations.length ? destinations : perpendicular.length ? perpendicular : result;
}
