import { fixedClipping } from "./fixed-polygon-boolean.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import type { NavigationAnchorArea } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";

const worldPoint = (p: Point, plane: HeightPlane): Point => [p[0], p[1] + planeHeight(plane, p)];
type Clearance = ReturnType<typeof prepareCorridorStates>;

// The corridor compiler uses the one-unit walking inset. Click endpoints and
// physical support require the full box, covered by the four shifted sweeps.
function fullBox(clear: Clearance, a: Point, b = a) {
  return [-1, 1].every((dx) =>
    [-1, 1].every((dy) => clear([a[0] + dx, a[1] + dy], [b[0] + dx, b[1] + dy]).length > 0),
  );
}

/** Prepare folded crossings once during export; runtime consumes ordinary passages. */
export function prepareFoldedNavigationPassage(
  from: NavigationAnchorArea,
  to: NavigationAnchorArea,
) {
  if (!from.receivingFloors?.length || !to.receivingFloors?.length)
    throw new Error("Folded navigation passages require physical receiving contours");
  const floors = [...from.receivingFloors, ...to.receivingFloors].map((floor) => [
    floor.polygon.map((p) => worldPoint(p, floor.plane)),
  ]);
  const coverage = fixedClipping.union(floors[0]!, ...floors.slice(1));
  const obstacles = (area: NavigationAnchorArea) =>
    area.blockers.map((points) => ({ polygon: { points }, state_id: 0 }));
  const worldBlockers = [from, to].flatMap((area) =>
    area.blockers.map((points) => points.map((p) => worldPoint(p, area.plane))),
  );
  const support = coverage.map(([outer, ...holes]) =>
    prepareCorridorStates(
      [...holes, ...worldBlockers].map((points) => ({ polygon: { points }, state_id: 0 })),
      outer,
    ),
  );
  const physicallyClear = (a: Point, b = a) => support.some((clear) => fullBox(clear, a, b));
  const fromClear = prepareCorridorStates(obstacles(from), from.polygon);
  const toClear = prepareCorridorStates(obstacles(to), to.polygon);
  const fromObstacles = prepareCorridorStates(obstacles(from));
  const toObstacles = prepareCorridorStates(obstacles(to));
  return (middle: Point, normal: Point, tangent: Point, depth: number) => {
    const locate = (
      floor: NavigationAnchorArea,
      clear: Clearance,
      along: number,
    ): Point | undefined => {
      for (let distance = 0.5; distance <= depth * 2; distance += 0.5) {
        const candidate: Point = [
          Math.round(middle[0] + normal[0] * distance + tangent[0] * along),
          Math.round(middle[1] + normal[1] * distance + tangent[1] * along),
        ];
        if (fullBox(clear, candidate) && physicallyClear(worldPoint(candidate, floor.plane)))
          return candidate;
      }
      return undefined;
    };
    // Separate the approaches along the seam as well as in world depth. Nearly
    // coincident projected endpoints cannot provide distinct usable approaches.
    const outside = locate(from, fromClear, 4);
    const inside = locate(to, toClear, -4);
    if (
      !outside ||
      !inside ||
      !fromObstacles(outside, middle).length ||
      !toObstacles(middle, inside).length ||
      !physicallyClear(worldPoint(outside, from.plane), worldPoint(middle, from.plane)) ||
      !physicallyClear(worldPoint(middle, to.plane), worldPoint(inside, to.plane))
    )
      return undefined;
    return { outside, middle, inside };
  };
}
