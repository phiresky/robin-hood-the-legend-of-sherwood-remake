import { compilePhysicalStairRegion } from "../src/compile-physical-stair-region.ts";
import type { Point } from "../src/level.ts";
import type { Vec3 } from "../src/scene.ts";

/** Two different slopes sharing a live barrier and one traversal sector. */
export function joinedPhysicalStairFixture() {
  const rectangle = (left: number, right: number): Point[] => [
    [left, 300],
    [right, 300],
    [right, 400],
    [left, 400],
  ];
  const result = compilePhysicalStairRegion({
    surfaces: [
      { polygon: rectangle(390, 400).map(([x, y]): Vec3 => [x, y, 4 * x - 1560]), holes: [] },
      { polygon: rectangle(400, 410).map(([x, y]): Vec3 => [x, y, 6 * x - 2360]), holes: [] },
    ],
    solids: [],
    clearances: [],
    blockers: [
      {
        transition: "barrier",
        applied: false,
        plane: [4, 0, -1560],
        polygon: rectangle(398, 400),
        holes: [],
      },
      {
        transition: "barrier",
        applied: false,
        plane: [6, 0, -2360],
        polygon: rectangle(400, 402),
        holes: [],
      },
    ],
    doors: [
      { inside: [392, 350, 8], middle: [390, 350, 0], outside: [380, 350, 0] },
      { inside: [408, 350, 88], middle: [410, 350, 100], outside: [420, 350, 100] },
    ],
  });
  return { navigation: result.navigation, area: result.area };
}
