import { fixedClipping } from "./fixed-polygon-boolean.ts";
import { prepareCorridorStates } from "./compile-navigation-graph.ts";
import { planeHeight, type HeightPlane } from "./gameplay-plane.ts";
import { pointInGameplayPolygon, type NavigationAnchorArea } from "./navigation-anchor.ts";
import { NAVIGATION_HALF_DIAGONAL } from "./navigation-footprint.ts";
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
  precise?: readonly [NavigationAnchorArea, NavigationAnchorArea],
) {
  if (!from.receivingFloors?.length || !to.receivingFloors?.length)
    throw new Error("Folded navigation passages require physical receiving contours");
  // A ground exclusion beneath a ramp belongs to the ground. Subtract each
  // floor's own exclusions before combining support across the matched seam.
  const floors = [from, to].flatMap((area) =>
    area.receivingFloors!.flatMap((floor) =>
      fixedClipping.difference(
        [floor.polygon.map((p) => worldPoint(p, floor.plane))],
        ...(floor.holes ?? area.blockers).map((points) => [
          points.map((p) => worldPoint(p, floor.plane)),
        ]),
      ),
    ),
  );
  const coverage = floors.length ? fixedClipping.union(floors[0]!, ...floors.slice(1)) : [];
  const obstacles = (area: NavigationAnchorArea) =>
    area.blockers.map((points) => ({ polygon: { points }, state_id: 0 }));
  const support = coverage.map(([outer, ...holes]) =>
    prepareCorridorStates(
      holes.map((points) => ({ polygon: { points }, state_id: 0 })),
      outer,
    ),
  );
  const physicallyClear = (a: Point, b = a) => support.some((clear) => fullBox(clear, a, b));
  const fromClear = prepareCorridorStates(obstacles(from), from.polygon);
  const toClear = prepareCorridorStates(obstacles(to), to.polygon);
  const fine = precise?.map((area) => prepareCorridorStates(obstacles(area), area.polygon));
  return (middle: Point, normal: Point, tangent: Point, depth: number, seam: Point) => {
    const locate = (
      floor: NavigationAnchorArea,
      clear: Clearance,
      along: number,
    ): Point | undefined => {
      const offsets = [
        along,
        0,
        ...Array.from({ length: Math.ceil(Math.max(...NAVIGATION_HALF_DIAGONAL)) }, (_, i) => [
          i + 1,
          -i - 1,
        ]).flat(),
      ];
      for (let distance = 0.5; distance <= depth * 2; distance += 0.5)
        for (const offset of offsets) {
          const candidate: Point = [
            Math.round(middle[0] + normal[0] * distance + tangent[0] * offset),
            Math.round(middle[1] + normal[1] * distance + tangent[1] * offset),
          ];
          if (
            fullBox(clear, candidate) &&
            (!fine || fullBox(fine[floor === from ? 0 : 1]!, candidate)) &&
            floor.receivingFloors!.some(
              (receiver) =>
                pointInGameplayPolygon(candidate, receiver.polygon, true) &&
                Math.abs(
                  planeHeight(receiver.plane, candidate) - planeHeight(floor.plane, candidate),
                ) < 1e-4,
            ) &&
            physicallyClear(worldPoint(candidate, floor.plane))
          )
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
      !physicallyClear(worldPoint(outside, from.plane), seam) ||
      !physicallyClear(seam, worldPoint(inside, to.plane))
    )
      return undefined;
    return { outside, middle, inside };
  };
}
