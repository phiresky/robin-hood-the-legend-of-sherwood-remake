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

test("duplicate native vertices retain one continuous roof receiver after rotation", () => {
  const coordinates = [
    [595.4296169281006, 860.0963896494826, 243.29275119123065],
    [596, 859.8414146015464, 242.84141460154638],
    [624.1391267776489, 881.1961031886772, 243.29275140492473],
    [624.1391849517822, 881.1961464300267, 243.29275173093004],
    [665.8215398788452, 912.8287430285409, 243.96131329526452],
    [665.8215379714966, 912.8287429824807, 243.9613142028786],
    [666, 912.964175475103, 243.96417547510305],
    [654.024658203125, 918.1095716855464, 253.30094169592235],
    [654.0245943069458, 918.1096004379376, 253.30099238282287],
    [606, 938.7441362569997, 290.7441362569997],
    [535, 887.1099426192023, 291.10994261920234],
  ];
  const points = coordinates.map(([x, y, z]) => ({ x: x!, y: y!, z_top: z!, z_bottom: z! }));
  const before = structuredClone(points),
    warnings: string[] = [];
  const result = nativeReceiverGeometry(points, "rotated roof", warnings);
  assert.equal(result.length, 1);
  assert.equal(result[0]!.length, points.length - 1);
  assert.deepEqual(warnings, []);
  const nativeVertices = (values: typeof points) =>
    new Set(values.map((p) => JSON.stringify([p.x, p.y, p.z_top, p.z_bottom].map(Math.fround))));
  assert.deepEqual(nativeVertices(result[0]!), nativeVertices(points));
  assert.deepEqual(points, before);
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
