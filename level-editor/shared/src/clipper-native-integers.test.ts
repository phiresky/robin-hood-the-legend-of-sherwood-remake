import test from "node:test";
import assert from "node:assert/strict";
import ClipperLib from "clipper-lib";

type Point = { X: number; Y: number };
type Collinearity = (a: Point, b: Point, c: Point, fullRange: boolean) => boolean;
// The dependency exposes these predicates without including them in its typings.
const predicates = (
  ClipperLib as typeof ClipperLib & {
    ClipperBase: { SlopesEqual: Collinearity; SlopesEqual4: Collinearity };
  }
).ClipperBase;

test("native collinearity preserves results across large coordinate translations", () => {
  for (const origin of [0, 2 ** 30, -(2 ** 30), 2 ** 51, -(2 ** 51)]) {
    for (const [dx, dy] of [
      [1, 1],
      [2 ** 25, 97],
      [97, -(2 ** 25)],
      [2 ** 26, 2 ** 25 + 1],
    ]) {
      const a = { X: origin, Y: -origin };
      const b = { X: origin + dx!, Y: -origin + dy! };
      for (const deviation of [-1, 0, 1]) {
        const c = { X: origin + 2 * dx!, Y: -origin + 2 * dy! + deviation };
        for (const points of [
          [a, b, c],
          [c, b, a],
          [a, a, c],
        ] as const) {
          assert.equal(
            predicates.SlopesEqual4(...points, true),
            predicates.SlopesEqual(...points, true),
            JSON.stringify(points),
          );
        }
        assert.equal(predicates.SlopesEqual4(a, b, c, true), deviation === 0);
      }
    }
  }
});

test("wide coordinate deltas retain one-unit deviations from collinearity", () => {
  const dx = 2 ** 40,
    dy = 2 ** 39 + 1;
  for (const origin of [0, 2 ** 51, -(2 ** 51)]) {
    const a = { X: origin, Y: -origin };
    const b = { X: origin + dx, Y: -origin + dy };
    for (const deviation of [-1, 0, 1]) {
      const c = { X: origin + 2 * dx, Y: -origin + 2 * dy + deviation };
      assert.equal(predicates.SlopesEqual4(a, b, c, true), deviation === 0);
      assert.equal(predicates.SlopesEqual4(c, b, a, true), deviation === 0);
    }
  }
});

test("rounded equal products still distinguish a one-unit determinant", () => {
  for (const size of [2 ** 27, 2 ** 40, 2 ** 50]) {
    const a = { X: 0, Y: 0 };
    const b = { X: size, Y: size + 1 };
    const c = { X: 2 * size + 1, Y: 2 * size + 3 };
    assert.equal((a.Y - b.Y) * (b.X - c.X), (a.X - b.X) * (b.Y - c.Y));
    assert.equal(predicates.SlopesEqual4(a, b, c, true), false);
    assert.equal(predicates.SlopesEqual4(c, b, a, true), false);
  }
});

test("small-range collinearity retains the ordinary numeric path", () => {
  for (const deviation of [-1, 0, 1]) {
    const a = { X: 1, Y: 2 },
      b = { X: 3, Y: 5 },
      c = { X: 5, Y: 8 + deviation };
    assert.equal(predicates.SlopesEqual4(a, b, c, false), deviation === 0);
    assert.equal(predicates.SlopesEqual4(a, b, c, false), predicates.SlopesEqual(a, b, c, false));
  }
});
