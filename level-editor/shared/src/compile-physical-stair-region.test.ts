import test from "node:test";
import assert from "node:assert/strict";
import { compilePhysicalStairRegion } from "./compile-physical-stair-region.ts";
import { heightPlane, type HeightPlane } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";

test("physical regions retain solid heights, owner clearances, holes and controls after placement", () => {
  for (const degrees of [0, 37, 90, 180, 270])
    for (const headroom of [0, 20]) {
      const angle = (degrees * Math.PI) / 180;
      const move = ([x, y, z]: Vec3): Vec3 => [
        600 + x * Math.cos(angle) - y * Math.sin(angle),
        600 + x * Math.sin(angle) + y * Math.cos(angle),
        z + 40,
      ];
      const rect = (x0: number, y0: number, x1: number, y1: number): Point[] => [
        [x0, y0],
        [x1, y0],
        [x1, y1],
        [x0, y1],
      ];
      const placed = (points: Point[]) =>
        points.map(([x, y]): Point => {
          const p = move([x, y, 0]);
          return [p[0], p[1]];
        });
      const plane = heightPlane(rect(380, 300, 420, 400).map(([x, y]) => move([x, y, y - 200])));
      const flat = (z: number): HeightPlane => [0, 0, z + 40];
      const wall = placed(rect(380, 340, 420, 360));
      const thin = placed(rect(395.1, 349.1, 395.2, 351.1));
      const result = compilePhysicalStairRegion({
        surfaces: [
          {
            polygon: rect(380, 300, 420, 400).map(([x, y]) => move([x, y, y - 200])),
            holes: [rect(385, 320, 390, 325).map(([x, y]) => move([x, y, y - 200]))],
          },
        ],
        doors: [],
        solids: [
          { owner: "stairs", polygon: wall, holes: [], bottom: flat(0), top: flat(300) },
          {
            owner: "post",
            polygon: placed(rect(398, 348, 402, 352)),
            holes: [],
            bottom: flat(0),
            top: flat(300),
          },
          {
            owner: "beam",
            polygon: placed(rect(380, 360, 420, 400)),
            holes: [],
            bottom: flat(165 - headroom),
            top: flat(180),
          },
          // Touching the floor from below supplies support, not a blocked floor.
          {
            owner: "foundation",
            polygon: placed(rect(380, 300, 420, 400)),
            holes: [],
            bottom: flat(0),
            top: plane,
          },
        ],
        clearances: [
          { owner: "stairs", plane, polygon: placed(rect(395, 340, 405, 360)), holes: [] },
        ],
        blockers: [
          { transition: "door", applied: false, plane, polygon: thin, holes: [] },
          {
            transition: "door",
            applied: true,
            plane,
            polygon: placed(rect(395, 370, 405, 380)),
            holes: [],
          },
          { transition: "copy", applied: false, plane, polygon: thin, holes: [] },
        ],
      });
      assert.deepEqual(
        [...result.pairs],
        [
          ["door", 0],
          ["copy", 1],
        ],
      );
      const areas = new Map<number, number>();
      for (const obstacle of result.navigation.obstacles) {
        const state = result.area.obstacles[obstacle.motion_obstacle]!.state_id;
        const polygon = obstacle.polygon;
        const area =
          Math.abs(
            polygon.reduce((sum, p, i) => {
              const q = polygon[(i + 1) % polygon.length]!;
              return sum + p[0] * q[1] - q[0] * p[1];
            }, 0),
          ) / 2;
        areas.set(state, (areas.get(state) ?? 0) + area);
      }
      for (const [state, expected] of [
        [0, headroom ? 1441 : 1241],
        [1, 0.2],
        [2, 100],
        [4, 0.2],
      ])
        assert.ok(
          Math.abs(areas.get(state!)! - expected!) < 1e-6,
          `${degrees} degrees/headroom ${headroom}/state ${state}: ${areas.get(state!)}`,
        );
      assert.equal(
        result.initialBlockers.length,
        result.area.obstacles.filter((o) => o.state_id !== 2).length,
      );
      if (degrees === 0) assert.ok(result.area.polygon.points.every(([, y]) => y === 760));
    }
});
