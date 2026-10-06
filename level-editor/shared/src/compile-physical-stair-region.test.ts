import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import {
  compilePhysicalStairRegion,
  type PhysicalStairRegionInput,
} from "./compile-physical-stair-region.ts";
import { heightPlane, type HeightPlane } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { joinedPhysicalStairFixture } from "../test-fixtures/joined-physical-stair.ts";

test("native joined stair traversal uses the compiler's floors and live collision", () => {
  const expected: unknown = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/joined-physical-stair-area.json", import.meta.url),
      "utf8",
    ),
  );
  assert.deepEqual(JSON.parse(JSON.stringify(joinedPhysicalStairFixture())), expected);
});

test("joined flights retain independent controls and crop each floor at its own height", () => {
  const rect = (x0: number, x1: number): Point[] => [
    [x0, 0],
    [x1, 0],
    [x1, 20],
    [x0, 20],
  ];
  const lower: HeightPlane = [1, 0, 0];
  const upper: HeightPlane = [2, 0, -10];
  const input: PhysicalStairRegionInput = {
    surfaces: [
      { polygon: rect(0, 10).map(([x, y]): Vec3 => [x, y, x]), holes: [] },
      { polygon: rect(10, 20).map(([x, y]): Vec3 => [x, y, 2 * x - 10]), holes: [] },
    ],
    solids: [],
    clearances: [],
    doors: [],
    blockers: [
      { transition: "lower", applied: false, plane: lower, polygon: rect(4, 5), holes: [] },
      { transition: "upper", applied: true, plane: upper, polygon: rect(14, 15), holes: [] },
    ],
  };
  const result = compilePhysicalStairRegion(input);
  assert.equal(result.navigation.floor_patches?.length, 2);
  assert.deepEqual(
    [...result.pairs],
    [
      ["lower", 0],
      ["upper", 1],
    ],
  );
  assert.deepEqual(
    result.area.obstacles.map((o) => o.state_id),
    [1, 8],
  );
  assert.equal(result.initialBlockers.length, 1);
  assert.ok(result.area.polygon.points.some(([x, y]) => x === 10 && y === -10));
  const cropped = compilePhysicalStairRegion({ ...input, frame: [2, -40, 18, 40] });
  assert.equal(cropped.navigation.floor_patches?.length, 2);
  assert.ok(
    cropped.navigation.floor_patches!.every((patch) =>
      patch.boundary.every(([x]) => x >= 2 && x <= 18),
    ),
  );
  const upperOnly = compilePhysicalStairRegion({ ...input, frame: [12, -40, 18, 40] });
  assert.equal(upperOnly.navigation.floor_patches, undefined);
  assert.deepEqual(upperOnly.navigation.plane, upper);
  assert.deepEqual([...upperOnly.pairs], [["upper", 0]]);
  assert.deepEqual(
    upperOnly.area.obstacles.map((o) => o.state_id),
    [2],
  );
});

test("rotated clearance contact preserves solids and still cuts actual openings", () => {
  const fixture: { polygon: Point[]; clearance: Point[] } = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/rotated-stair-clearance-contact.json", import.meta.url),
      "utf8",
    ),
  );
  const input: PhysicalStairRegionInput = {
    surfaces: [
      {
        polygon: [
          [870, 1460, 0],
          [1040, 1460, 0],
          [1040, 1580, 0],
          [870, 1580, 0],
        ],
        holes: [],
      },
    ],
    solids: [
      { owner: "tower", polygon: fixture.polygon, holes: [], bottom: [0, 0, -1], top: [0, 0, 1] },
    ],
    clearances: [{ owner: "tower", polygon: fixture.clearance, holes: [], plane: [0, 0, 0] }],
    doors: [],
    blockers: [],
  };
  const contact = compilePhysicalStairRegion(input);
  const blocked = (region: typeof contact, point: Point) =>
    region.navigation.obstacles.some((o) => pointInGameplayPolygon(point, o.polygon));
  assert.ok(blocked(contact, [898, 1530]));
  assert.ok(blocked(contact, [898, 1540]));
  const opening: PhysicalStairRegionInput["clearances"][number] = {
    owner: "tower",
    polygon: [
      [890, 1525],
      [905, 1525],
      [905, 1535],
      [890, 1535],
    ],
    holes: [],
    plane: [0, 0, 0],
  };
  const cut = compilePhysicalStairRegion({ ...input, clearances: [...input.clearances, opening] });
  assert.equal(blocked(cut, [898, 1530]), false);
  assert.ok(blocked(cut, [898, 1540]));
  const wrongOwner = compilePhysicalStairRegion({
    ...input,
    clearances: [...input.clearances, { ...opening, owner: "other" }],
  });
  assert.ok(blocked(wrongOwner, [898, 1530]));
});

test("export cropping preserves edge-on floor area, holes and local control bindings", () => {
  const rectangle = (x0: number, y0: number, x1: number, y1: number): Vec3[] => [
    [x0, y0, y0 - 200],
    [x1, y0, y0 - 200],
    [x1, y1, y1 - 200],
    [x0, y1, y1 - 200],
  ];
  const input: PhysicalStairRegionInput = {
    surfaces: [{ polygon: rectangle(380, 300, 420, 400), holes: [rectangle(395, 330, 405, 340)] }],
    solids: [],
    clearances: [],
    doors: [],
    blockers: [
      {
        transition: "gate",
        applied: false,
        plane: [0, 1, -200],
        polygon: [
          [380, 350],
          [420, 350],
          [420, 352],
          [380, 352],
        ],
        holes: [],
      },
    ],
  };
  const full = compilePhysicalStairRegion({ ...input, frame: [370, 199, 430, 201] });
  assert.equal(full.navigation.obstacles.length, 2);
  const cropped = compilePhysicalStairRegion({ ...input, frame: [390, 199, 410, 201] });
  assert.deepEqual(cropped.navigation.boundary, [
    [390, 300],
    [410, 300],
    [410, 400],
    [390, 400],
  ]);
  assert.deepEqual([...cropped.pairs], [["gate", 0]]);
  assert.deepEqual(
    cropped.area.obstacles.map((obstacle) => obstacle.state_id),
    [0, 1],
  );
  assert.ok(
    cropped.navigation.obstacles.every((obstacle) =>
      obstacle.polygon.every(([x]) => x >= 390 && x <= 410),
    ),
  );
  assert.ok(cropped.area.polygon.points.every(([, y]) => y === 200));
  // Cropping through the hole turns it into a boundary notch. Only the live
  // barrier remains an obstacle, and its physical reference must be reallocated.
  const notch = compilePhysicalStairRegion({ ...input, frame: [400, 199, 410, 201] });
  assert.equal(notch.area.obstacles.length, 1);
  assert.equal(notch.area.obstacles[0]!.state_id, 1);
  assert.equal(notch.navigation.obstacles[0]!.motion_obstacle, 0);
  assert.throws(
    () => compilePhysicalStairRegion({ ...input, frame: [370, 201, 430, 220] }),
    /no floor inside/,
  );
  assert.throws(
    () => compilePhysicalStairRegion({ ...input, frame: [410, 199, 390, 201] }),
    /export frame/,
  );
  assert.throws(
    () =>
      compilePhysicalStairRegion({
        ...input,
        frame: [390, 199, 410, 201],
        doors: [
          {
            inside: [385, 310, 110],
            middle: [385, 300, 100],
            outside: [385, 290, 100],
          },
        ],
      }),
    /floor support/,
  );
});

test("export cropping cannot invent a connection between separated floor islands", () => {
  assert.throws(
    () =>
      compilePhysicalStairRegion({
        surfaces: [
          {
            polygon: [
              [0, 0, 0],
              [30, 0, 0],
              [30, 30, 0],
              [20, 30, 0],
              [20, 10, 0],
              [10, 10, 0],
              [10, 30, 0],
              [0, 30, 0],
            ],
            holes: [],
          },
        ],
        solids: [],
        clearances: [],
        blockers: [],
        doors: [],
        frame: [0, 15, 30, 30],
      }),
    /connected floor/,
  );
});

test("sloping floor export cropping applies screen Y limits without flattening height", () => {
  const result = compilePhysicalStairRegion({
    surfaces: [
      {
        polygon: [
          [0, 0, 0],
          [100, 0, 0],
          [100, 100, 50],
          [0, 100, 50],
        ],
        holes: [],
      },
    ],
    solids: [],
    clearances: [],
    blockers: [],
    doors: [],
    frame: [20, 10, 80, 40],
  });
  assert.deepEqual(result.navigation.plane, [0, 0.5, 0]);
  assert.deepEqual(result.navigation.boundary, [
    [20, 20],
    [80, 20],
    [80, 80],
    [20, 80],
  ]);
  assert.deepEqual(result.area.polygon.points, [
    [20, 10],
    [80, 10],
    [80, 40],
    [20, 40],
  ]);
});

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
