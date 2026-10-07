import test from "node:test";
import assert from "node:assert/strict";
import {
  behindDisplayPolyline,
  mergeNativeDisplay,
  type NativeOrderedDraw,
} from "./native-display-merge.ts";
const row = (
  identity: string,
  order: number,
  rank: number,
  point: [number, number],
  polyline: [number, number][] = [],
  x = 0,
): NativeOrderedDraw => ({
  identity,
  order,
  rank,
  mapPosition: point,
  polyline,
  draw: { x, y: 0, pixels: { width: 1, height: 1, data: new Uint8Array([1, 2, 3, 255]) } },
});
test("animation merge uses current map point and polyline rather than scalar display depth", () => {
  const effect = row(
    "sign",
    0,
    104,
    [1000, 1000],
    [
      [0, 50],
      [100, 50],
    ],
  );
  const behind = row("behind", 900, 40, [50, 49]);
  const front = row("front", 1, 41, [50, 50]);
  assert.deepEqual(
    mergeNativeDisplay([front, effect, behind]).rows.map((r) => r.identity),
    ["behind", "sign", "front"],
  );
  assert.equal(behindDisplayPolyline(effect.polyline, [-10, 49]), true);
  assert.equal(behindDisplayPolyline(effect.polyline, [110, 50]), false);
  assert.equal(
    behindDisplayPolyline(
      [
        [0, 0],
        [100, 100],
      ],
      [50, 49],
    ),
    true,
  );
});
test("scalar lane retains full construction rank while another target uses action map point", () => {
  const effect = row(
    "line",
    900,
    104,
    [0, 0],
    [
      [0, 50],
      [100, 50],
    ],
  );
  const target = row("target", 500, 105, [50, 49]);
  const actor = row("actor", 500, 40, [50, 51]);
  assert.deepEqual(
    mergeNativeDisplay([effect, target, actor]).rows.map((r) => r.identity),
    ["target", "line", "actor"],
  );
  assert.deepEqual(
    mergeNativeDisplay([target, actor]).rows.map((r) => r.identity),
    ["actor", "target"],
  );
});
test("unknown min-Y tie is accepted only when every changed pair is alpha-disjoint", () => {
  const a = row(
      "a",
      0,
      0,
      [0, 0],
      [
        [0, 10],
        [100, 10],
      ],
      0,
    ),
    b = row(
      "b",
      0,
      1,
      [0, 0],
      [
        [0, 10],
        [100, 50],
      ],
      2,
    );
  const actor = row("actor", 20, 40, [50, 20], [], 4);
  const got = mergeNativeDisplay([a, b, actor]);
  assert.equal(got.tieProof.variants, 2);
  // The actor is behind b but in front of a, so tie permutations change its order too.
  assert.deepEqual(
    got.rows.map((r) => r.identity),
    ["a", "actor", "b"],
  );
  const overlap = { ...actor, draw: { ...actor.draw, x: 0 } };
  assert.throws(() => mergeNativeDisplay([a, b, overlap]), /Unresolved overlapping/);
  const transparent = {
    ...overlap,
    draw: { ...overlap.draw, pixels: { ...overlap.draw.pixels, data: new Uint8Array(4) } },
  };
  assert.equal(mergeNativeDisplay([a, b, transparent]).tieProof.variants, 2);
});
test("malformed display polylines and excessive unresolved ties fail explicitly", () => {
  assert.throws(() => mergeNativeDisplay([row("a", 0, 0, [0, 0], [[0, 0]])]), /two points/);
  assert.throws(
    () =>
      mergeNativeDisplay([
        row(
          "a",
          0,
          0,
          [0, 0],
          [
            [1, 0],
            [0, 1],
          ],
        ),
      ]),
    /Invalid/,
  );
  assert.throws(
    () =>
      mergeNativeDisplay(
        Array.from({ length: 5 }, (_, i) =>
          row(
            String(i),
            0,
            i,
            [0, 0],
            [
              [0, 0],
              [1, 1],
            ],
            i,
          ),
        ),
      ),
    /bounded/,
  );
});
