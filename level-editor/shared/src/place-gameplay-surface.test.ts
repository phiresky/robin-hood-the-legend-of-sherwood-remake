import test from "node:test";
import assert from "node:assert/strict";
import type { AssetWalkableSurface } from "./asset-gameplay.ts";
import { placeGameplaySurface } from "./place-gameplay-surface.ts";
import { compilePhysicalStairRegion } from "./compile-physical-stair-region.ts";
import { planeHeight } from "./gameplay-plane.ts";
import type { Vec3 } from "./scene.ts";

test("asset-local floors and holes reach physical assembly even at an edge-on placement", () => {
  const surface: AssetWalkableSurface = {
    id: "stairs",
    node: "mesh",
    polygon: [
      [380, 300],
      [420, 300],
      [420, 400],
      [380, 400],
    ],
    height: [100, 100, 200, 200],
    holes: [
      [
        [385, 320],
        [390, 320],
        [390, 325],
        [385, 325],
      ],
    ],
  };
  const original = structuredClone(surface);
  for (const degrees of [0, 37, 90, 180, 270])
    for (const elevation of [0, 40]) {
      const angle = (degrees * Math.PI) / 180;
      const place = (node: string, [x, y, z]: Vec3): Vec3 => {
        assert.equal(node, "mesh");
        return [
          600 + x * Math.cos(angle) - y * Math.sin(angle),
          600 + x * Math.sin(angle) + y * Math.cos(angle),
          z + elevation,
        ];
      };
      const placed = placeGameplaySurface(surface, place);
      for (const point of [...placed.navigationPoints, ...placed.navigationHoles.flat()])
        assert.ok(Math.abs(planeHeight(placed.worldPlane, [point[0], point[1]]) - point[2]) < 1e-6);
      const result = compilePhysicalStairRegion({
        surfaces: [{ polygon: placed.navigationPoints, holes: placed.navigationHoles }],
        solids: [],
        clearances: [],
        blockers: [],
        doors: [],
      });
      assert.equal(result.area.obstacles.length, 1);
      assert.equal(result.navigation.obstacles[0]!.motion_obstacle, 0);
      assert.equal(result.area.obstacles[0]!.state_id, 0);
      if (degrees === 0)
        assert.ok(result.area.polygon.points.every(([, y]) => y === 800 - elevation));
    }
  assert.deepEqual(surface, original);
});

test("raised clearance contours retain distinct physical and navigation heights, including holes", () => {
  const surface: AssetWalkableSurface = {
    id: "clearance",
    node: "mesh",
    polygon: [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ],
    height: 80,
    navigationHeight: 0,
    holes: [
      [
        [10, 10],
        [20, 10],
        [20, 20],
        [10, 20],
      ],
    ],
  };
  const placed = placeGameplaySurface(surface, (_, [x, y, z]) => [-y + 500, x + 500, z + 40]);
  assert.ok([...placed.points, ...placed.holes.flat()].every((point) => point[2] === 120));
  assert.ok(
    [...placed.navigationPoints, ...placed.navigationHoles.flat()].every(
      (point) => point[2] === 40,
    ),
  );
  assert.deepEqual(placed.worldPlane, [0, 0, 40]);
  assert.deepEqual(
    placed.holes[0]!.map(([x, y]) => [x, y]),
    [
      [490, 510],
      [490, 520],
      [480, 520],
      [480, 510],
    ],
  );
});
