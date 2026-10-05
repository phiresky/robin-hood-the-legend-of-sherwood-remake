import test from "node:test";
import assert from "node:assert/strict";
import {
  containsNavigationAnchor,
  navigationAnchorHeight,
  type NavigationAnchorArea,
} from "./navigation-anchor.ts";
import { compilePhysicalStairRegion } from "./compile-physical-stair-region.ts";

test("physical anchors resolve distinct heights and holes despite identical screen positions", () => {
  const region = compilePhysicalStairRegion({
    surfaces: [
      {
        polygon: [
          [380, 300, 100],
          [420, 300, 100],
          [420, 400, 200],
          [380, 400, 200],
        ],
        holes: [
          [
            [390, 340, 140],
            [410, 340, 140],
            [410, 360, 160],
            [390, 360, 160],
          ],
        ],
      },
    ],
    solids: [],
    clearances: [],
    blockers: [],
    doors: [],
  });
  const area: NavigationAnchorArea = {
    coordinateSpace: "world",
    plane: region.navigation.plane,
    polygon: region.navigation.boundary,
    blockers: region.initialBlockers,
  };
  assert.ok(containsNavigationAnchor(area, [400, 310, 110]));
  assert.ok(containsNavigationAnchor(area, [400, 390, 190]));
  assert.equal(navigationAnchorHeight(area, [400, 310, 110]), 110);
  assert.equal(navigationAnchorHeight(area, [400, 390, 190]), 190);
  assert.ok(!containsNavigationAnchor(area, [400, 410, 210]));
  assert.ok(!containsNavigationAnchor(area, [400, 310, 190]));
  assert.ok(!containsNavigationAnchor(area, [400, 350, 150]));
  assert.ok(containsNavigationAnchor(area, [400, 350, 150], { allowBlocked: true }));
  assert.ok(containsNavigationAnchor(area, [400, 300, 100]));
  assert.ok(
    !containsNavigationAnchor(area, [400, 340, 140]),
    "hole edges do not supply floor support",
  );
  assert.ok(!containsNavigationAnchor(area, [400, 310, NaN]));
  // Snapped screen probes must never replace the world-space floor position.
  assert.ok(containsNavigationAnchor(area, [400, 310, 110], { projected: [-100, -100] }));
});

test("projected anchors retain separate exact height and quantized contour probes", () => {
  const area: NavigationAnchorArea = {
    plane: [1, 0, 0],
    polygon: [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ],
    blockers: [
      [
        [4, 4],
        [6, 4],
        [6, 6],
        [4, 6],
      ],
    ],
  };
  assert.ok(containsNavigationAnchor(area, [0.1, 5.1, 0.1], { projected: [0, 5] }));
  assert.ok(!containsNavigationAnchor(area, [9.9, 14.9, 9.9], { projected: [10, 5] }));
  assert.ok(!containsNavigationAnchor(area, [5, 10, 5]));
  assert.ok(containsNavigationAnchor(area, [5, 10, 5], { allowBlocked: true }));
  assert.ok(!containsNavigationAnchor(area, [5, 12, 7]));
  assert.ok(
    containsNavigationAnchor(area, [5, 12, 7], { allowBlocked: true, requireHeight: false }),
  );
});
