import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import type { Polygon } from "polygon-clipping";

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
