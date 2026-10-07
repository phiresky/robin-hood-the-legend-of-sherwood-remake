import test from "node:test";
import assert from "node:assert/strict";
import clipping, { type MultiPolygon } from "polygon-clipping";
import type { NavigationPiece } from "./assemble-navigation-regions.ts";
import {
  compileTransitionObstacles,
  compilePhysicalTransitionObstacles,
  type PlacedTransitionBlocker,
} from "./compile-movement-transitions.ts";

test("physical collision removes a clipped seam spike without changing the solid", () => {
  const polygon: [number, number][] = [
    [3088.704474456256, 2396.3919016174286],
    [3100.52152865474, 2380.0148359356363],
    [3109.4988233938807, 2382.120735877474],
    [3100.1241559570058, 2380.5655483773025],
  ];
  const result = compilePhysicalTransitionObstacles(
    [
      [3000, 2300],
      [3200, 2300],
      [3200, 2500],
      [3000, 2500],
    ],
    [],
    [0, 0, 0],
    [{ transition: "terrace", fixed: true, applied: false, plane: [0, 0, 0], polygon, holes: [] }],
  );
  assert.equal(result.obstacles.length, 1);
  assert.equal(result.obstacles[0]!.polygon.points.length, 3);
  assert.ok(result.obstacles[0]!.polygon.points.every(([x]) => x > 3100));
  assert.deepEqual(clipping.xor([polygon.slice(1)], [result.obstacles[0]!.polygon.points]), []);
});

const boundary: [number, number][] = [
  [0, 0],
  [100, 0],
  [100, 100],
  [0, 100],
];
const blocker: PlacedTransitionBlocker = {
  transition: "gate",
  applied: false,
  plane: [0, 0, 0],
  polygon: [
    [10, 10],
    [90, 10],
    [90, 90],
    [10, 90],
  ],
  holes: [
    [
      [30, 30],
      [30, 70],
      [70, 70],
      [70, 30],
    ],
  ],
};
test("a subtraction kernel failure preserves control bits and excludes permanent collision", (t) => {
  const difference = clipping.difference;
  let fail = true;
  t.mock.method(clipping, "difference", (...args: Parameters<typeof difference>) => {
    if (fail) {
      fail = false;
      throw new Error("Unable to find segment in SweepLine tree");
    }
    return difference(...args);
  });
  const result = compileTransitionObstacles(
    boundary,
    [
      [
        [40, 0],
        [60, 0],
        [60, 100],
        [40, 100],
      ],
    ],
    [0, 0, 0],
    [
      {
        ...blocker,
        polygon: [
          [0, 40],
          [100, 40],
          [100, 60],
          [0, 60],
        ],
        holes: [],
      },
    ],
    [],
  );
  assert.equal(result.pairs.get("gate"), 0);
  assert.equal(result.initial.length, 2);
  assert.ok(result.obstacles.every((obstacle) => obstacle.state_id === 1));
  const expected: MultiPolygon = [
    [
      [
        [0, 40],
        [40, 40],
        [40, 60],
        [0, 60],
      ],
    ],
    [
      [
        [60, 40],
        [100, 40],
        [100, 60],
        [60, 60],
      ],
    ],
  ];
  const actual = result.obstacles.map((obstacle) => [obstacle.polygon.points]);
  assert.deepEqual(clipping.xor(actual, expected), []);
});

test("terrain blockers follow slopes, retain holes and exclude floors beyond their reach", () => {
  const volume: PlacedTransitionBlocker = {
    ...blocker,
    terrainVolume: {
      polygon: blocker.polygon,
      holes: blocker.holes,
      plane: [0, 0, 0],
      below: 2,
      above: 8,
    },
  };
  const flat = compileTransitionObstacles(boundary, [], [0, 0, 4], [volume], []);
  const actual = clipping.union(flat.obstacles.map((o) => [o.polygon.points]));
  const shifted = [blocker.polygon, ...blocker.holes].map((ring) =>
    ring.map(([x, y]): [number, number] => [x, y - 4]),
  );
  assert.deepEqual(clipping.xor(actual, shifted), []);
  for (const height of [-3, 9, 100])
    assert.equal(
      compileTransitionObstacles(boundary, [], [0, 0, height], [volume], []).pairs.size,
      0,
    );
  // z = x / 10: the volume stops at x=80, even with preserved receiving boundaries.
  const slope = compileTransitionObstacles(
    boundary,
    [],
    [0.1, 0, 0],
    [volume],
    [],
    undefined,
    true,
  );
  assert.ok(slope.obstacles.length);
  assert.ok(slope.obstacles.every((o) => o.polygon.points.every(([x]) => x <= 80)));
});

test("terrain blockers join slope fragments before rounding and retain independent state pairs", () => {
  const volume: PlacedTransitionBlocker = {
    ...blocker,
    terrainVolume: { polygon: blocker.polygon, holes: [], plane: [0, 0, 0], below: 2, above: 8 },
  };
  const receivers: NavigationPiece[] = [
    {
      plane: [0, 0, 0],
      layer: 0,
      polygon: [
        [0, 0],
        [50, 0],
        [50, 100],
        [0, 100],
      ],
      blockers: [],
    },
    {
      plane: [0.1, 0, -5],
      layer: 0,
      polygon: [
        [50, 0],
        [100, 0],
        [100, 100],
        [50, 100],
      ],
      blockers: [],
    },
  ];
  const result = compileTransitionObstacles(
    boundary,
    [],
    [0, 0, 0],
    [volume, { ...volume, applied: true }, { ...volume, transition: "copy" }],
    [],
    receivers,
  );
  assert.deepEqual(
    result.obstacles.map((o) => o.state_id),
    [1, 2, 4],
  );
  assert.deepEqual(
    clipping.xor(
      [result.obstacles[0]!.polygon.points],
      [
        [
          [10, 10],
          [50, 10],
          [90, 6],
          [90, 86],
          [50, 90],
          [10, 90],
        ],
      ],
    ),
    [],
  );
});

test("labelled transition fragments rejoin before rounding and remain scoped to each state", () => {
  const left: PlacedTransitionBlocker = {
    transition: "gate",
    applied: false,
    plane: [0, 0, 0],
    holes: [],
    movementContour: "edge",
    polygon: [
      [10, 10],
      [40.3, 10],
      [40.3, 38.21],
      [10, 17],
    ],
  };
  const right: PlacedTransitionBlocker = {
    ...left,
    polygon: [
      [40.3, 10],
      [90, 10],
      [90, 73],
      [40.3, 38.21],
    ],
  };
  const result = compileTransitionObstacles(
    boundary,
    [],
    [0, 0, 0],
    [
      left,
      right,
      { ...left, applied: true },
      { ...right, applied: true },
      { ...left, transition: "other" },
      { ...right, transition: "other" },
    ],
    [],
  );
  assert.deepEqual(
    result.obstacles.map((o) => o.state_id),
    [1, 2, 4],
  );
  for (const obstacle of result.obstacles)
    assert.deepEqual(
      clipping.xor(
        [obstacle.polygon.points],
        [
          [
            [10, 10],
            [90, 10],
            [90, 73],
            [10, 17],
          ],
        ],
      ),
      [],
    );
  const separate = compileTransitionObstacles(
    boundary,
    [],
    [0, 0, 0],
    [left, { ...right, movementContour: "separate" }],
    [],
  );
  assert.equal(separate.obstacles.length, 2);
});
test("preserved state contours retain implicit fractional boundary crossings", () => {
  const triangle: [number, number][] = [
    [0, 0],
    [100, 0],
    [0, 71],
  ];
  const gate: PlacedTransitionBlocker = {
    ...blocker,
    holes: [],
    polygon: [
      [40, -10],
      [60, -10],
      [60, 100],
      [40, 100],
    ],
  };
  const kept = compileTransitionObstacles(triangle, [], [0, 0, 0], [gate], [], undefined, true);
  assert.deepEqual(kept.obstacles[0]!.polygon.points, gate.polygon);
  assert.equal(kept.obstacles[0]!.state_id, 1);
  assert.deepEqual(kept.initial, [gate.polygon]);
  const clipped = compileTransitionObstacles(triangle, [], [0, 0, 0], [gate], []);
  assert.notDeepEqual(clipped.obstacles[0]!.polygon.points, gate.polygon);
  const absent = { ...gate, polygon: gate.polygon.map(([x, y]): [number, number] => [x + 200, y]) };
  assert.equal(
    compileTransitionObstacles(triangle, [], [0, 0, 0], [absent], [], undefined, true).pairs.size,
    0,
  );
});
test("preserved state contours on joined planes cannot block the neighboring plane", () => {
  const receivers: NavigationPiece[] = [
    {
      plane: [0, 0, 0],
      layer: 0,
      polygon: [
        [0, 0],
        [50, 0],
        [50, 100],
        [0, 100],
      ],
      blockers: [],
    },
    {
      plane: [1, 0, -50],
      layer: 1,
      polygon: [
        [50, 0],
        [100, 0],
        [100, 100],
        [50, 100],
      ],
      blockers: [],
    },
  ];
  const gate: PlacedTransitionBlocker = {
    ...blocker,
    holes: [],
    polygon: [
      [40, -10],
      [60, -10],
      [60, 110],
      [40, 110],
    ],
  };
  const kept = compileTransitionObstacles(boundary, [], [0, 0, 0], [gate], [], receivers, true);
  const blocked = kept.obstacles.map((o) => [o.polygon.points]);
  assert.deepEqual(clipping.intersection(blocked, [receivers[1]!.polygon]), []);
  assert.deepEqual(
    clipping.xor(clipping.intersection(blocked, [boundary]), [
      [
        [40, 0],
        [50, 0],
        [50, 100],
        [40, 100],
      ],
    ]),
    [],
  );
  assert(kept.obstacles.some((o) => o.polygon.points.some(([, y]) => y < 0)));
});
test("one transition shares state bits across receiving planes and clips to each receiver", () => {
  const receivers: NavigationPiece[] = [
    {
      plane: [0, 0, 0],
      layer: 0,
      polygon: [
        [0, 0],
        [50, 0],
        [50, 100],
        [0, 100],
      ],
      blockers: [],
    },
    {
      plane: [1, 0, -50],
      layer: 1,
      polygon: [
        [50, 0],
        [100, 0],
        [100, 100],
        [50, 100],
      ],
      blockers: [],
    },
  ];
  const result = compileTransitionObstacles(
    boundary,
    [],
    [0, 0, 0],
    [
      { ...blocker, holes: [] },
      { ...blocker, holes: [], plane: [1, 0, -50], applied: true },
      { ...blocker, holes: [], plane: [0, 0, 999], transition: "unrelated" },
    ],
    [],
    receivers,
  );
  assert.deepEqual([...result.pairs], [["gate", 0]]);
  assert.deepEqual(
    result.obstacles.map((o) => o.state_id),
    [1, 2],
  );
  assert.ok(result.obstacles[0]!.polygon.points.every(([x]) => x <= 50));
  assert.ok(result.obstacles[1]!.polygon.points.every(([x]) => x >= 50));
  assert.equal(result.initial.length, 1);
});
test("state blocker holes survive as nonoverlapping triangles", () => {
  const result = compileTransitionObstacles(boundary, [], [0, 0, 0], [blocker], []);
  const area = result.obstacles.reduce(
    (sum, o) =>
      sum +
      Math.abs(
        o.polygon.points.reduce((a, p, i) => {
          const q = o.polygon.points[(i + 1) % o.polygon.points.length]!;
          return a + p[0] * q[1] - q[0] * p[1];
        }, 0),
      ) /
        2,
    0,
  );
  assert.equal(area, 4800);
  assert.ok(result.obstacles.every((o) => o.state_id === 1));
});

test("small state blocker holes do not use their array index as a simplification tolerance", () => {
  const result = compileTransitionObstacles(
    boundary,
    [],
    [0, 0, 0],
    [
      {
        ...blocker,
        holes: [
          [
            [30, 30],
            [31, 30],
            [30, 31],
          ],
        ],
      },
    ],
    [],
  );
  const area = result.obstacles.reduce(
    (sum, obstacle) =>
      sum +
      Math.abs(
        obstacle.polygon.points.reduce((a, p, i) => {
          const q = obstacle.polygon.points[(i + 1) % obstacle.polygon.points.length]!;
          return a + p[0] * q[1] - q[0] * p[1];
        }, 0),
      ) /
        2,
    0,
  );
  assert.equal(area, 6399.5);
});
test("crossing permanent obstacles do not create state coverage outside the movement envelope", () => {
  const crossing: [number, number][] = [
    [50, -20],
    [120, -20],
    [120, 80],
    [50, 80],
  ];
  const outside: PlacedTransitionBlocker = {
    ...blocker,
    holes: [],
    polygon: [
      [105, 10],
      [115, 10],
      [115, 20],
      [105, 20],
    ],
  };
  const result = compileTransitionObstacles(boundary, [crossing], [0, 0, 0], [outside], []);
  assert.equal(result.pairs.size, 0);
  assert.deepEqual(result.obstacles, []);
});
test("state bit pairs remain unsigned and unrelated planes receive no binding", () => {
  const blockers = Array.from({ length: 16 }, (_, i) => ({
    ...blocker,
    holes: [],
    transition: `gate-${i}`,
    applied: true,
  }));
  const result = compileTransitionObstacles(boundary, [], [0, 0, 0], blockers, []);
  assert.equal(result.obstacles.at(-1)!.state_id, 2147483648);
  assert.equal(compileTransitionObstacles(boundary, [], [0, 0, 1], blockers, []).pairs.size, 0);
  blockers.push({ ...blocker, holes: [], transition: "overflow", applied: true });
  assert.throws(
    () => compileTransitionObstacles(boundary, [], [0, 0, 0], blockers, []),
    /More than 16/,
  );
});
