import test from "node:test";
import assert from "node:assert/strict";
import { preserveMovementBoundary } from "./preserve-movement-boundary.ts";
import type { Point } from "./level.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import clipping, { type MultiPolygon } from "polygon-clipping";

test("preserved terrain retains fractional obstacle contours for physical landings", () => {
  const outer: Point[] = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  const solid: Point[] = [
    [20.25, 20.25],
    [60.25, 20.25],
    [60.25, 60.25],
    [20.25, 60.25],
  ];
  const result = preserveMovementBoundary(outer, [[solid]], []);
  assert.deepEqual(result.preciseBlockers, [solid]);
  assert.deepEqual(
    clipping.xor(
      result.blockers.map((p) => [p]),
      [[solid.map(([x, y]) => [Math.round(x), Math.round(y)])]],
    ),
    [],
  );
  assert.ok(
    clipping.difference(
      [[solid]],
      result.blockers.map((p) => [p]),
    ).length,
  );
});

test("outside contacts do not round into movement obstacles across a sloped boundary", () => {
  const outer: Point[] = [
    [0, 0],
    [100, 0],
    [0, 71],
  ];
  const contact = (intrusion: number): MultiPolygon => [
    [
      [
        [0, 71],
        [40.3, 42.387 - intrusion],
        [50, 90],
      ],
    ],
  ];
  assert.deepEqual(preserveMovementBoundary(outer, contact(0), []).blockers, []);
  assert.deepEqual(preserveMovementBoundary(outer, contact(1 / 1048576), []).blockers, []);
  assert.equal(preserveMovementBoundary(outer, contact(0.01), []).blockers.length, 1);
});

test("distinct integer contours retain fractional overlap intersections without rounding", () => {
  const cutouts: MultiPolygon = [
    [
      [
        [0, 0],
        [100, 0],
        [0, 71],
      ],
    ],
    [
      [
        [40, -10],
        [60, -10],
        [60, 100],
        [40, 100],
      ],
    ],
  ];
  const preserved = preserveMovementBoundary(boundary, cutouts, [], ["slope", "wall"]);
  assert.equal(preserved.blockers.length, 2);
  const free = (blockers: MultiPolygon) => clipping.difference([boundary], blockers);
  const expected = free(cutouts);
  assert.deepEqual(clipping.xor(expected, free(preserved.blockers.map((b) => [b]))), []);
  const independent = preserveMovementBoundary(boundary, cutouts, []);
  assert.equal(independent.blockers.length, 2);
  assert.deepEqual(clipping.xor(expected, free(independent.blockers.map((b) => [b]))), []);
});

test("matching contour fragments assemble before snapping and follow independent placement", () => {
  const left: Point[] = [
    [0, 0],
    [37.3, 0],
    [37.3, 44.517],
    [0, 71],
  ];
  const right: Point[] = [
    [37.3, 0],
    [100, 0],
    [37.3, 44.517],
  ];
  const assembled = preserveMovementBoundary(boundary, [[left], [right]], [], ["slope", "slope"]);
  assert.deepEqual(
    clipping.xor(
      [
        [
          [0, 0],
          [100, 0],
          [0, 71],
        ],
      ],
      assembled.blockers.map((b) => [b]),
    ),
    [],
  );
  const moved = preserveMovementBoundary(
    boundary,
    [[left], [right.map(([x, y]) => [x + 50, y])]],
    [],
    ["slope", "slope"],
  );
  assert.equal(moved.blockers.length, 2);
  assert.throws(() => preserveMovementBoundary(boundary, [[left]], [], []), /labels do not match/);
});

const boundary: Point[] = [
  [0, 0],
  [100, 0],
  [100, 100],
  [0, 100],
];

test("rounding a redundant collision fragment cannot expand its integer exclusion", () => {
  const exclusion: Point[] = [
    [0, 0],
    [100, 0],
    [0, 71],
  ];
  const fragment: Point[] = [
    [40.6, 0],
    [60, 0],
    [40.6, 42.174],
  ];
  for (const contours of [
    [[exclusion], [fragment]],
    [[fragment], [exclusion]],
  ]) {
    const result = preserveMovementBoundary(boundary, contours, []);
    assert.equal(result.blockers.length, 1);
    assert.deepEqual(
      clipping.xor(
        result.blockers.map((b) => [b]),
        [exclusion],
      ),
      [],
    );
  }
  const extending: Point[] = [
    [40.6, 0],
    [60, 0],
    [40.6, 43],
  ];
  assert.equal(
    preserveMovementBoundary(boundary, [[exclusion], [extending]], []).blockers.length,
    2,
  );
});

test("preserved movement boundaries discard unrelated blockers but retain crossing contours", () => {
  const result = preserveMovementBoundary(
    boundary,
    [
      [
        [
          [50, -10],
          [60, -10],
          [60, 110],
          [50, 110],
        ],
      ],
      [
        [
          [200, 200],
          [210, 200],
          [210, 210],
          [200, 210],
        ],
      ],
    ],
    [],
  );
  assert.equal(result.blockers.length, 1);
  assert.deepEqual(result.blockers[0], [
    [50, -10],
    [60, -10],
    [60, 110],
    [50, 110],
  ]);
  assert.deepEqual(result.polygon, boundary);
});

test("preserved movement boundaries retain enclosed walkable islands", () => {
  const obstacles: Point[][] = [
    [
      [10, 10],
      [90, 10],
      [90, 90],
      [10, 90],
    ],
    [
      [30, 30],
      [70, 30],
      [70, 70],
      [30, 70],
    ],
  ];
  const result = preserveMovementBoundary(boundary, [obstacles], []);
  assert.equal(result.blockers.length, 8);
  assert.deepEqual(
    fixedPolygonBoolean(
      "xor",
      obstacles,
      result.blockers.map((p) => [p]),
    ),
    [],
  );
  assert(result.blockers.flat().every((p) => p.every(Number.isInteger)));
});

test("preserved movement boundaries reject collapsed envelopes", () => {
  assert.throws(
    () =>
      preserveMovementBoundary(
        [
          [0, 0],
          [0.1, 0],
          [0, 0.1],
        ],
        [],
        [],
      ),
    /collapsed/,
  );
});
