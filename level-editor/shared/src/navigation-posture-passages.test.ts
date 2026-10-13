import test from "node:test";
import assert from "node:assert/strict";
import { navigationPosturePassages } from "./navigation-posture-passages.ts";
import type { NavigationConnection } from "./assemble-navigation-joins.ts";
import type { NavigationAnchorArea } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";

function fixture() {
  const from: NavigationAnchorArea = {
    plane: [0, 0, 0],
    polygon: [
      [0, 0],
      [100, 0],
      [100, 100],
      [0, 100],
    ],
    blockers: [],
  };
  const to: NavigationAnchorArea = {
    plane: [0, 0, 0],
    polygon: [
      [100, 0],
      [200, 0],
      [200, 100],
      [100, 100],
    ],
    blockers: [],
  };
  const connection: NavigationConnection = {
    from: {
      owner: "left",
      region: "left/floor",
      edge: [
        [100, 0, 0],
        [100, 100, 0],
      ],
    },
    to: {
      owner: "right",
      region: "right/floor",
      edge: [
        [100, 100, 0],
        [100, 0, 0],
      ],
    },
    overlap: [0, 1],
  };
  return { from, to, connection };
}

test("matched seams tolerate clipping-grid noise but not an authored gap", () => {
  for (const [gap, connects] of [
    [1 / 1048576, true],
    [1 / 512, false],
  ] as const) {
    const { from, to, connection } = fixture();
    const preciseTo = {
      ...to,
      polygon: to.polygon.map(([x, y]): Point => [x === 100 ? x + gap : x, y]),
    };
    assert.equal(
      navigationPosturePassages(connection, from, to, [from, preciseTo]).length > 0,
      connects,
    );
  }
});

test("an oblique short landing connects without requiring symmetric approach offsets", () => {
  const from: NavigationAnchorArea = {
    plane: [0.9454810659191314, 0.8226182833327891, -19.5799325472492],
    polygon: [
      [33, 90],
      [52, 69],
      [68, 64],
      [73, 64],
      [64, 74],
      [56, 83],
      [54, 85],
      [43, 89],
      [38, 91],
      [33, 91],
    ],
    blockers: [],
  };
  const to: NavigationAnchorArea = {
    plane: [0, 0, 90.00101],
    polygon: [
      [-100, -100],
      [200, -100],
      [200, 200],
      [-100, 200],
    ],
    blockers: [
      [
        [52, 92],
        [39, 88],
        [54, 71],
        [64, 74],
        [67, 75],
        [56, 87],
        [66, 77],
        [140, 100],
        [103, 139],
        [29, 116],
      ],
    ],
  };
  const edge: NavigationConnection["from"]["edge"] = [
    [39.317038377274, 178.0216979187835, 90.00100999999995],
    [54.044107636543, 161.09505597167572, 90.00100999999995],
  ];
  const connection: NavigationConnection = {
    from: { owner: "ramp", region: "ramp", edge },
    to: { owner: "ground", region: "ground", edge: [edge[1], edge[0]] },
    overlap: [0, 1],
  };
  const before = structuredClone({ from, to, connection });
  assert.ok(navigationPosturePassages(connection, from, to).length > 0);
  assert.ok(
    navigationPosturePassages({ ...connection, from: connection.to, to: connection.from }, to, from)
      .length > 0,
  );
  const closed = structuredClone(from);
  closed.blockers.push([
    [20, 50],
    [90, 50],
    [90, 110],
    [20, 110],
  ]);
  assert.deepEqual(navigationPosturePassages(connection, closed, to), []);
  assert.deepEqual({ from, to, connection }, before);
});

test("approach selection checks precise collision as well as integer geometry", () => {
  const { from, to, connection } = fixture();
  assert.ok(navigationPosturePassages(connection, from, to).length > 0);
  const preciseFrom = structuredClone(from);
  preciseFrom.blockers = [
    [
      [90, 0],
      [100, 0],
      [100, 100],
      [90, 100],
    ],
  ];
  assert.deepEqual(navigationPosturePassages(connection, from, to, [preciseFrom, to]), []);
  assert.deepEqual(from.blockers, []);
});

test("posture crossings reserve both approaches and survive rotated placement", () => {
  for (const degrees of [0, 37, 90, 180]) {
    const { from, to, connection } = fixture();
    const radians = (degrees * Math.PI) / 180;
    const transform = ([x, y]: Point): Point => [
      500 + x * Math.cos(radians) - y * Math.sin(radians),
      500 + x * Math.sin(radians) + y * Math.cos(radians),
    ];
    from.polygon = from.polygon.map(transform);
    to.polygon = to.polygon.map(transform);
    for (const join of [connection.from, connection.to])
      join.edge = join.edge.map(([x, y, z]) => [...transform([x, y]), z]) as typeof join.edge;
    const before = structuredClone({ from, to, connection });
    const passages = navigationPosturePassages(connection, from, to);
    assert.ok(passages.length > 0, `rotation ${degrees}`);
    assert.ok(
      passages.every((p) => [...p.outside, ...p.middle, ...p.inside].every(Number.isInteger)),
    );
    assert.deepEqual({ from, to, connection }, before);
  }
});

test("blocked middle of a socket retains independently clear crossings", () => {
  const { from, to, connection } = fixture();
  const blocker: Point[] = [
    [90, 40],
    [110, 40],
    [110, 60],
    [90, 60],
  ];
  from.blockers.push(blocker);
  to.blockers.push(blocker);
  const passages = navigationPosturePassages(connection, from, to);
  assert.ok(passages.some((p) => p.middle[1] < 37));
  assert.ok(passages.some((p) => p.middle[1] > 63));
  assert.ok(passages.every((p) => p.middle[1] <= 37 || p.middle[1] >= 63));
});

test("a single usable integer crossing is not lost between coarse samples", () => {
  const { from, to, connection } = fixture();
  for (const floor of [from, to])
    floor.blockers.push(
      [
        [90, 0],
        [110, 0],
        [110, 43],
        [90, 43],
      ],
      [
        [90, 49],
        [110, 49],
        [110, 100],
        [90, 100],
      ],
    );
  const passages = navigationPosturePassages(connection, from, to);
  assert.deepEqual(
    passages.map((p) => p.middle),
    [[100, 46]],
  );
});

test("narrow, disconnected and completely blocked seams have no posture crossing", () => {
  const { from, to, connection } = fixture();
  connection.overlap = [0.47, 0.53];
  assert.deepEqual(navigationPosturePassages(connection, from, to), []);
  connection.overlap = [0, 1];
  to.polygon = to.polygon.map(([x, y]) => [x + 1, y]);
  assert.deepEqual(navigationPosturePassages(connection, from, to), []);
  to.polygon = to.polygon.map(([x, y]) => [x - 1, y]);
  from.blockers.push([
    [90, 0],
    [100, 0],
    [100, 100],
    [90, 100],
  ]);
  assert.deepEqual(navigationPosturePassages(connection, from, to), []);
});

function foldedFixture() {
  const polygon: Point[] = [
    [0, -120],
    [100, -120],
    [100, -100],
    [0, -100],
  ];
  const from: NavigationAnchorArea = {
    plane: [0, 0, 100],
    polygon,
    blockers: [],
    receivingFloors: [{ plane: [0, 0, 100], polygon }],
  };
  const to: NavigationAnchorArea = {
    plane: [0, -2, -100],
    polygon,
    blockers: [],
    receivingFloors: [{ plane: [0, -2, -100], polygon }],
  };
  const connection: NavigationConnection = {
    from: {
      owner: "left",
      region: "left/floor",
      edge: [
        [100, 0, 100],
        [0, 0, 100],
      ],
    },
    to: {
      owner: "right",
      region: "right/floor",
      edge: [
        [0, 0, 100],
        [100, 0, 100],
      ],
    },
    overlap: [0, 1],
  };
  return { from, to, connection };
}

test("folded physical neighbors connect in either order despite overlapping projections", () => {
  for (const reverse of [false, true]) {
    const fixture = foldedFixture();
    const { from, to, connection } = fixture;
    const before = structuredClone(fixture);
    const crossings = navigationPosturePassages(
      reverse ? { ...connection, from: connection.to, to: connection.from } : connection,
      reverse ? to : from,
      reverse ? from : to,
    );
    assert.ok(crossings.length > 0);
    for (const crossing of crossings) {
      assert.equal(crossing.middle[1], -100);
      assert.ok(crossing.outside[1] < -103 && crossing.inside[1] < -103);
      assert.notDeepEqual(crossing.outside, crossing.inside);
    }
    assert.deepEqual(fixture, before);
  }
});

test("expanded movement contours cannot bridge a missing physical floor", () => {
  const { from, to, connection } = foldedFixture();
  from.receivingFloors![0]!.polygon = [
    [0, -120],
    [100, -120],
    [100, -108],
    [0, -108],
  ];
  assert.deepEqual(navigationPosturePassages(connection, from, to), []);
});

test("folded crossings retain solid blockers and require physical receiving metadata", () => {
  const { from, to, connection } = foldedFixture();
  from.blockers = [
    [
      [0, -120],
      [100, -120],
      [100, -100],
      [0, -100],
    ],
  ];
  assert.deepEqual(navigationPosturePassages(connection, from, to), []);
  delete from.receivingFloors;
  assert.throws(
    () => navigationPosturePassages(connection, from, to),
    /physical receiving contours/,
  );
});
