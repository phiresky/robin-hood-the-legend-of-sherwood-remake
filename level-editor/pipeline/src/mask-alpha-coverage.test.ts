import test from "node:test";
import assert from "node:assert/strict";
import { maskAlphaCoverage, maskWrappedAlphaCoverage } from "./mask-alpha-coverage.ts";
import {
  rasterizeMaskGeometry,
  type MaskTriangle,
} from "../../shared/src/compile-mask-geometry.ts";
import { maskCoverage } from "./mask-roundtrip.ts";

const a: MaskTriangle = [
  [0, 0, 0],
  [4, 0, 0],
  [0, 4, 0],
];
const b: MaskTriangle = [
  [4, 0, 0],
  [4, 4, 0],
  [0, 4, 0],
];
const uv = (triangle: MaskTriangle): [number, number][] => triangle.map(([x, y]) => [x / 4, y / 4]);
const area = (triangles: MaskTriangle[]) =>
  triangles.reduce(
    (sum, [a, b, c]) =>
      sum + Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2,
    0,
  );

test("wrapped alpha footprints follow repeat and mirrored tiles across negative UVs", () => {
  const texture = { width: 2, height: 1, alpha: new Uint8Array([255, 0]) };
  const rules = {
    layer: 0,
    mask_type: 4,
    character_polyline: null,
    projectile_polyline: null,
    obstacle_indices: [],
  };
  for (const mode of ["repeat", "mirror"] as const) {
    const clipped = [a, b].flatMap((t) => [
      ...maskWrappedAlphaCoverage(
        t,
        t.map(([x, y]) => [x / 2 - 1, y / 4]),
        [1, 1, 1],
        0.5,
        texture,
        [mode, "clamp"],
      ),
    ]);
    assert.equal(area(clipped), 8);
    const pixels = new Set(
      rasterizeMaskGeometry(clipped, rules).flatMap((m) => [...maskCoverage(m)]),
    );
    const expectedXs = mode === "repeat" ? [0, 2] : [1, 2];
    assert.deepEqual(
      pixels,
      new Set(expectedXs.flatMap((x) => [0, 1, 2, 3].map((y) => `${x},${y}`))),
    );
  }
  assert.throws(
    () => [
      ...maskWrappedAlphaCoverage(
        a,
        [
          [0, 0],
          [10000, 0],
          [0, 1],
        ],
        [1, 1, 1],
        0.5,
        texture,
        ["repeat", "clamp"],
      ),
    ],
    /4096/,
  );
});

test("nearest alpha clips holes and retains cutoff equality exactly", () => {
  const texture = {
    width: 4,
    height: 4,
    alpha: Uint8Array.from([255, 255, 0, 0, 255, 0, 0, 0, 255, 0, 255, 255, 255, 255, 255, 255]),
  };
  const triangles = [a, b].flatMap((t) =>
    maskAlphaCoverage(t, uv(t), [0.5, 0.5, 0.5], 0.5, texture),
  );
  assert.equal(area(triangles), 10);
  const pixels = new Set(
    rasterizeMaskGeometry(triangles, {
      layer: 0,
      mask_type: 4,
      character_polyline: null,
      projectile_polyline: null,
      obstacle_indices: [],
    }).flatMap((m) => [...maskCoverage(m)]),
  );
  assert.deepEqual(
    pixels,
    new Set(["0,0", "1,0", "0,1", "0,2", "2,2", "3,2", "0,3", "1,3", "2,3", "3,3"]),
  );
  assert.equal(maskAlphaCoverage(a, uv(a), [0.49, 0.49, 0.49], 0.5, texture).length, 0);
});

test("clamp-to-edge extends edge texels without changing interior UV interpolation", () => {
  for (const axis of [0, 1] as const)
    for (const reverse of [false, true]) {
      const texture = {
        width: axis === 0 ? 2 : 1,
        height: axis === 1 ? 2 : 1,
        alpha: new Uint8Array(reverse ? [0, 255] : [255, 0]),
      };
      const triangles = [a, b].flatMap((t) =>
        maskAlphaCoverage(
          t,
          t.map((p) => {
            const coords: [number, number] = [p[0] / 4, p[1] / 4];
            const value = p[axis] / 2 - 1;
            coords[axis] = reverse ? 1 - value : value;
            return coords;
          }),
          [1, 1, 1],
          0.5,
          texture,
          [axis === 0, axis === 1],
        ),
      );
      assert.equal(area(triangles), 12);
      for (const p of triangles.flat()) assert.ok(p[axis] <= 3);
    }
  assert.deepEqual(
    maskAlphaCoverage(
      a,
      [
        [-2, 3],
        [-2, 3],
        [-2, 3],
      ],
      [1, 1, 1],
      0.5,
      { width: 1, height: 1, alpha: new Uint8Array([255]) },
      [true, true],
    ),
    [a],
  );
  assert.throws(
    () =>
      maskAlphaCoverage(
        a,
        [
          [-2, 3],
          [-2, 3],
          [-2, 3],
        ],
        [1, 1, 1],
        0.5,
        { width: 1, height: 1, alpha: new Uint8Array([255]) },
        [true, false],
      ),
    /in-range/,
  );
});

test("vertex alpha clips geometry independently of texture UV degeneracy", () => {
  assert.equal(area(maskAlphaCoverage(a, uv(a), [1, 0, 1], 0.5)), 6);
  const texture = { width: 2, height: 1, alpha: new Uint8Array([0, 255]) };
  assert.deepEqual(
    maskAlphaCoverage(
      a,
      [
        [0.75, 0.5],
        [0.75, 0.5],
        [0.75, 0.5],
      ],
      [1, 1, 1],
      0.5,
      texture,
    ),
    [a],
  );
  assert.deepEqual(
    maskAlphaCoverage(
      a,
      [
        [0.25, 0.5],
        [0.25, 0.5],
        [0.25, 0.5],
      ],
      [1, 1, 1],
      0.5,
      texture,
    ),
    [],
  );
});

test("alpha recovery rejects unsupported UV range and corrupt alpha images", () => {
  assert.throws(
    () =>
      maskAlphaCoverage(
        a,
        [
          [-1, 0],
          [1, 0],
          [0, 1],
        ],
        [1, 1, 1],
        0.5,
      ),
    /in-range/,
  );
  assert.throws(() => maskAlphaCoverage(a, uv(a), [1, NaN, 1], 0.5), /in-range/);
  assert.throws(
    () =>
      maskAlphaCoverage(a, uv(a), [1, 1, 1], 0.5, {
        width: 2,
        height: 2,
        alpha: new Uint8Array(1),
      }),
    /dimensions/,
  );
});

test("cutout interpolation stays on the authored sloping mesh", () => {
  const sloped: MaskTriangle = a.map(([x, y]) => [x, y + 2 * x + 7, 2 * x + 7]) as MaskTriangle;
  const triangles = maskAlphaCoverage(sloped, uv(a), [1, 1, 1], 0.5, {
    width: 2,
    height: 1,
    alpha: new Uint8Array([255, 0]),
  });
  assert.ok(triangles.length);
  for (const triangle of triangles)
    for (const [x, , z] of triangle) {
      assert.equal(z, 2 * x + 7);
      assert.ok(x <= 2);
    }
});

test("uniform vertex alpha merges accepted texels regardless of their stored alpha", () => {
  const texture = {
    width: 4,
    height: 4,
    alpha: Uint8Array.from({ length: 16 }, (_, i) => 128 + i * 7),
  };
  assert.deepEqual(maskAlphaCoverage(a, uv(a), [0.75, 0.75, 0.75], 0.375, texture), [a]);
  assert.deepEqual(maskAlphaCoverage(a, uv(a), [0, 0, 0], 0, texture), [a]);
});

test("varying vertex alpha retains texel-specific clip boundaries", () => {
  const texture = { width: 2, height: 1, alpha: new Uint8Array([128, 255]) };
  const triangles = maskAlphaCoverage(a, uv(a), [1, 0.5, 1], 0.5, texture);
  assert.equal(area(triangles), 2.12451171875);
});
