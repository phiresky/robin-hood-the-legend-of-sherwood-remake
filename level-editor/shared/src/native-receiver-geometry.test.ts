import test from "node:test";
import assert from "node:assert/strict";
import { nativeReceiverGeometry } from "./native-receiver-geometry.ts";
import type { Point } from "./level.ts";

const surface = (points: Point[]) => points.map(([x, y]) => ({ x, y, z_top: 7, z_bottom: 7 }));

test("a large simple terrain outline retains its original vertices", () => {
  const points = surface(
    Array.from({ length: 10000 }, (_, i): Point => [
      12000 + 10000 * Math.cos((i / 10000) * 2 * Math.PI),
      12000 + 10000 * Math.sin((i / 10000) * 2 * Math.PI),
    ]),
  );
  const warnings: string[] = [];
  const result = nativeReceiverGeometry(points, "large outline", warnings);
  assert.deepEqual(result, [points]);
  assert.deepEqual(warnings, []);
});

test("native receiving geometry retains representable subpixel floors and concave boundaries", () => {
  for (const polygon of [
    [
      [0, 0],
      [20, 0],
      [20, 20],
      [0, 0],
    ],
    [
      [100, 100],
      [100.00002, 100],
      [100, 100.00002],
    ],
    [
      [0, 0],
      [20, 0],
      [20, 10],
      [10, 10],
      [10, 20],
      [0, 20],
    ],
  ] satisfies Point[][]) {
    const points = surface(polygon),
      warnings: string[] = [];
    assert.deepEqual(nativeReceiverGeometry(points, "receiver", warnings), [points]);
    assert.deepEqual(warnings, []);
  }
});

test("a receiving sliver which collapses natively is omitted with an explicit warning", () => {
  const warnings: string[] = [];
  const points = surface([
    [179.82971858978271, 192],
    [172.49426651000977, 172.49426651000977],
    [172.4942684173584, 172.49426651000977],
  ]);
  assert.deepEqual(nativeReceiverGeometry(points, "receiver", warnings), []);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0]!, /zero area at native coordinate precision/);
});

test("a notch which pinches shut at native precision keeps both sides without a crossed polygon", () => {
  const points = surface([
    [0, 0],
    [100, 1],
    [50, 1.000000001],
    [0, 1.000000002],
    [100, 1.000000003],
    [100, 2],
    [0, 2],
  ]);
  const before = structuredClone(points),
    warnings: string[] = [];
  const result = nativeReceiverGeometry(points, "receiver", warnings);
  assert.ok(result.length > 1);
  assert.ok(result.every((triangle) => triangle.length === 3));
  assert.ok(result.flat().every((point) => points.includes(point)));
  assert.ok(result.flat().some(({ y }) => y === 0));
  assert.ok(result.flat().some(({ y }) => y === 2));
  let area = 0;
  for (const triangle of result) {
    const a = triangle[0]!,
      b = triangle[1]!,
      c = triangle[2]!;
    const f = Math.fround;
    const twiceArea = (f(b.x) - f(a.x)) * (f(c.y) - f(a.y)) - (f(b.y) - f(a.y)) * (f(c.x) - f(a.x));
    assert.notEqual(twiceArea, 0);
    area += Math.abs(twiceArea) / 2;
  }
  assert.equal(area, 150);
  assert.match(warnings[0]!, /triangulated/);
  assert.deepEqual(points, before);
});
