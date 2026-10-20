import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import type { Polygon } from "polygon-clipping";

test("high-precision physical clipping preserves nonempty full-range intersections", () => {
  for (const offset of [0, 2200, 12000]) {
    const rectangle = (low: number, high: number): Polygon => [
      [
        [offset + low, offset + low],
        [offset + high, offset + low],
        [offset + high, offset + high],
        [offset + low, offset + high],
      ],
    ];
    const result = fixedPolygonBoolean(
      "intersection",
      rectangle(0, 100),
      [rectangle(10, 90)],
      2 ** 32,
    );
    assert.equal(result.length, 1, "The locked clipping patch must preserve a contained rectangle");
    assert.equal(result[0]!.length, 1);
    assert.deepEqual(
      result[0]![0]!.slice(0, -1).sort((a, b) => a[0] - b[0] || a[1] - b[1]),
      rectangle(10, 90)[0]!.sort((a, b) => a[0] - b[0] || a[1] - b[1]),
    );
  }
});

test("landing comparison preserves real seam changes among nearly coincident edges", () => {
  const [precise, rounded]: Polygon[] = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/near-coincident-landing-footprints.json", import.meta.url),
      "utf8",
    ),
  );
  const difference = fixedPolygonBoolean("xor", precise!, [rounded!]);
  assert.ok(difference.length > 0, "The exact receiving seams differ from the movement grid");
  assert.deepEqual(fixedPolygonBoolean("xor", precise!, [precise!]), []);
  const noisy: Polygon = rounded!.map((ring) => ring.map(([x, y]) => [x, y - 1e-13]));
  assert.deepEqual(
    fixedPolygonBoolean("xor", rounded!, [noisy]),
    [],
    "Roundoff alone must not create a distinct receiver",
  );
});

test("sub-grid polygons that collapse during snapping retain empty-set semantics", () => {
  const tiny: Polygon = [
    [
      [0.1, 0.1],
      [0.2, 0.1],
      [0.1, 0.2],
    ],
  ];
  const square: Polygon = [
    [
      [0, 0],
      [10, 0],
      [10, 10],
      [0, 10],
    ],
  ];
  assert.deepEqual(fixedPolygonBoolean("difference", tiny, [tiny], 1), []);
  assert.deepEqual(fixedPolygonBoolean("intersection", tiny, [square], 1), []);
  for (const operation of ["union", "xor"] as const)
    assert.deepEqual(
      fixedPolygonBoolean(operation, tiny, [square], 1),
      fixedPolygonBoolean("union", square, [], 1),
    );
  assert.deepEqual(
    fixedPolygonBoolean("difference", square, [tiny], 1),
    fixedPolygonBoolean("union", square, [], 1),
  );
});

test("solid slices preserve near-coincident fractional edges", () => {
  const footprint: Polygon = [
    [
      [1797.3295, 375.18195],
      [1782.1462, 394.54633],
      [1733.7853, 382.0714],
      [1748.9685, 362.70703],
    ],
  ];
  const slice: Polygon = [
    [
      [1748.9685921591606, 362.70703],
      [1797.3295, 362.70703],
      [1797.3295, 394.54633],
      [1733.7853, 394.54633],
      [1733.7853, 382.0714000000001],
    ],
  ];
  const area = (polygon: Polygon) =>
    Math.abs(
      polygon[0]!.reduce((sum, p, i, ring) => {
        const q = ring[(i + 1) % ring.length]!;
        return sum + p[0] * q[1] - q[0] * p[1];
      }, 0),
    ) / 2;
  const result = fixedPolygonBoolean("intersection", footprint, [slice]);
  assert.equal(result.length, 1);
  assert.ok(area(result[0]!) > area(footprint) - 0.01);
  assert.ok(area(result[0]!) <= area(footprint) + 0.0001);
  assert.deepEqual(fixedPolygonBoolean("difference", result, [footprint]), []);
});
