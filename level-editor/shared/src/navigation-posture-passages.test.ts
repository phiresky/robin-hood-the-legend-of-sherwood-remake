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
