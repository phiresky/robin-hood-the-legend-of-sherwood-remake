import test from "node:test";
import assert from "node:assert/strict";
import { NativeRemainderOwnership } from "./native-remainder-ownership.ts";
import { nativePresentationFrame } from "../../shared/src/native-state-presentation.ts";
const pixels = (...colors: number[][]) => ({
  width: colors.length,
  height: 1,
  data: new Uint8Array(colors.flat()),
});
const blank = [0, 0, 0, 0],
  art = [40, 30, 20, 255],
  orange = [233, 149, 72, 255];
test("later inferred spill preserves earlier kept artwork, while own source coverage still wins", () => {
  const ownership = new NativeRemainderOwnership(3, 1);
  ownership.native(pixels(art, art, blank), 0, 0, true);
  const layer = pixels(orange, orange, orange),
    original = new Uint8Array(layer.data);
  const result = ownership.physical(layer, pixels(blank, art, blank), 0, 0);
  assert.deepEqual([...result.data], [...blank, ...orange, ...orange]);
  assert.deepEqual(layer.data, original);
});
test("ordinary later source art retires earlier kept ownership; shadows do not create owners", () => {
  const ownership = new NativeRemainderOwnership(2, 1),
    shadow = {
      rgb: [1, 2, 3] as [number, number, number],
      strength_percent: 50,
      pixel_format: "rgb565" as const,
    };
  ownership.native(pixels(art, blank), 0, 0, true);
  ownership.native(pixels(art, [1, 2, 3, 255]), 0, 0, false, shadow);
  const layer = pixels(orange, orange);
  assert.equal(ownership.physical(layer, pixels(blank, blank), 0, 0), layer);
  ownership.native(pixels(art, blank), 0, 0, true);
  const result = ownership.physical(layer, pixels([1, 2, 3, 255], blank), 0, 0, shadow);
  assert.deepEqual([...result.data], [...blank, ...orange]);
});
test("offset clipping and per-composition ownership do not retain stale frames", () => {
  const ownership = new NativeRemainderOwnership(3, 1);
  ownership.native(pixels(art, art), -1, 0, true);
  assert.deepEqual(
    [...ownership.physical(pixels(orange, orange, orange), pixels(blank), 2, 0).data],
    [...blank, ...orange, ...orange],
  );
  assert.deepEqual(
    [
      ...new NativeRemainderOwnership(3, 1).physical(
        pixels(orange, orange, orange),
        pixels(blank),
        2,
        0,
      ).data,
    ],
    [...orange, ...orange, ...orange],
  );
});
test("source31/32/33 handoff delays and endpoint freezes remain independent", () => {
  const stage = (count: number, delay: number) => ({
    loop: false,
    frames: Array.from({ length: count }, (_, i) => ({ delay: i === 0 ? delay : 0 })),
  });
  const a = stage(100, 0),
    b = stage(34, 99),
    c = stage(31, 132);
  for (const [tick, expected] of [
    [99, [99, 0, 0]],
    [100, [99, 1, 0]],
    [132, [99, 33, 0]],
    [133, [99, 33, 1]],
  ] as const)
    assert.deepEqual(
      [a, b, c].map((e) => nativePresentationFrame(e, tick)),
      expected,
    );
});

test("fractional kept alpha does not become an opaque blocker", () => {
  const ownership = new NativeRemainderOwnership(2, 1),
    layer = pixels(orange, orange);
  ownership.native(pixels([40, 30, 20, 1], [40, 30, 20, 128]), 0, 0, true);
  assert.equal(ownership.physical(layer, pixels(blank, blank), 0, 0), layer);
  ownership.native(pixels(art, blank), 0, 0, true);
  ownership.native(pixels([40, 30, 20, 1], blank), 0, 0, true);
  assert.deepEqual(
    [...ownership.physical(layer, pixels(blank, blank), 0, 0).data],
    [...blank, ...orange],
  );
});
