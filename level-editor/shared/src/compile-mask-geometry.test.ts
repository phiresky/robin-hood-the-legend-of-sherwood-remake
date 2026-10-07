import test from "node:test";
import assert from "node:assert/strict";
import {
  maskBoundaryPolyline,
  rasterizeMaskGeometry,
  type MaskTriangle,
} from "./compile-mask-geometry.ts";
import type { Mask, Point } from "./level.ts";
import type { MaskAlphaCoverage } from "./mask-alpha-sampler.ts";

const rules = {
  layer: 0,
  mask_type: 4,
  character_polyline: null,
  projectile_polyline: null,
  obstacle_indices: [],
};
const rectangle = (x: number, y: number, width: number, height: number): MaskTriangle[] => [
  [
    [x, y, 0],
    [x + width, y, 0],
    [x + width, y + height, 0],
  ],
  [
    [x, y, 0],
    [x + width, y + height, 0],
    [x, y + height, 0],
  ],
];
test("compact alpha masks preserve transparent texels, UV placement and material sidedness", () => {
  const triangles = rectangle(0, 0, 4, 4);
  const coverage: MaskAlphaCoverage = {
    textures: [{ width: 2, height: 1, alphaBase64: btoa(String.fromCharCode(255, 0)) }],
    triangles: triangles.map((triangle) => ({
      uv: triangle.map(([x, y]) => [x / 4, y / 4]) as [Point, Point, Point],
      alpha: [1, 1, 1],
      cutoff: 0.5,
      texture: 0,
      wrap: ["clamp", "clamp"],
    })),
  };
  const count = (mesh: MaskTriangle[], cull = false) =>
    rasterizeMaskGeometry(mesh, rules, cull, coverage)
      .flatMap(decode)
      .reduce((sum, n) => sum + n, 0);
  assert.equal(count(triangles), 8);
  assert.equal(count(triangles, true), 0);
  for (const rule of coverage.triangles) rule.doubleSided = true;
  assert.equal(count(triangles, true), 8);
  const moved = triangles.map(
    (triangle) => triangle.map(([x, y, z]) => [10 - y, 20 + x, z]) as MaskTriangle,
  );
  assert.equal(count(moved, true), 8);
  const opaque = structuredClone(coverage);
  opaque.textures[0]!.alphaBase64 = btoa(String.fromCharCode(255, 255));
  assert.equal(
    rasterizeMaskGeometry(triangles, rules, true, opaque)
      .flatMap(decode)
      .reduce((sum, n) => sum + n, 0),
    16,
  );
  assert.throws(
    () => rasterizeMaskGeometry(triangles, rules, false, { ...coverage, triangles: [] }),
    /match/,
  );
  const invalid = structuredClone(coverage);
  invalid.textures[0]!.alphaBase64 = "AAAA";
  assert.throws(() => rasterizeMaskGeometry(triangles, rules, false, invalid), /size mismatch/);
});

test("compact alpha sampling retains repeat, mirrored repeat and cutoff equality", () => {
  const triangles = rectangle(0, 0, 8, 2);
  const coverage: MaskAlphaCoverage = {
    textures: [{ width: 2, height: 1, alphaBase64: btoa(String.fromCharCode(255, 0)) }],
    triangles: triangles.map((triangle) => ({
      uv: triangle.map(([x, y]) => [x / 4 - 1, y / 2]) as [Point, Point, Point],
      alpha: [0.5, 0.5, 0.5],
      cutoff: 0.5,
      texture: 0,
      wrap: ["repeat", "clamp"],
    })),
  };
  const pixels = () => rasterizeMaskGeometry(triangles, rules, false, coverage).flatMap(decode);
  assert.deepEqual(pixels().slice(0, 8), [1, 1, 0, 0, 1, 1, 0, 0]);
  for (const rule of coverage.triangles) rule.wrap[0] = "mirror";
  assert.deepEqual(pixels().slice(0, 8), [0, 0, 1, 1, 1, 1, 0, 0]);
  for (const rule of coverage.triangles) rule.alpha = [0.49, 0.49, 0.49];
  assert.ok(pixels().every((pixel) => pixel === 0));
});
test("one-sided masks retain front faces and reject back faces after placement", () => {
  const back = rectangle(0, 0, 4, 4);
  const front = back.map(([a, b, c]): MaskTriangle => [c, b, a]);
  const covered = (triangles: MaskTriangle[], cull = true) =>
    rasterizeMaskGeometry(triangles, rules, cull)
      .flatMap(decode)
      .reduce((sum, n) => sum + n, 0);
  assert.equal(covered(back), 0);
  assert.equal(covered(front), 16);
  assert.equal(covered(back, false), 16);
  // Rotate a vertical leaf through 180 degrees about its local vertical axis.
  const leaf: MaskTriangle = [
    [0, 0, 0],
    [4, 0, 0],
    [0, 0, 4],
  ];
  assert.equal(covered([leaf]), 10);
  assert.equal(covered([leaf.map(([x, y, z]) => [-x, -y, z]) as MaskTriangle]), 0);
});

function decode(mask: Mask): number[] {
  const [width, height] = mask.box_size;
  const pixels = Array<number>(width * height).fill(0);
  let offset = 0;
  for (let y = 0; y < height; y++) {
    const end = offset + 1 + mask.mask_data[offset++]!;
    let x = 0;
    while (offset < end) {
      const control = mask.mask_data[offset++]!,
        count = control & 127;
      for (let block = 0; block < count; block++) {
        const byte = mask.mask_data[offset + (control & 128 ? 0 : block)]!;
        for (let bit = 0; bit < 8; bit++, x++)
          if (x < width) pixels[y * width + x] = (byte >> (7 - bit)) & 1;
      }
      offset += control & 128 ? 1 : count;
    }
  }
  return pixels;
}

test("mask boundary envelopes preserve concave steps, winding and rotation", () => {
  const boundary: Point[] = [
    [0, 0],
    [10, 0],
    [10, 20],
    [5, 20],
    [5, 10],
    [0, 10],
  ];
  const expected: Point[] = [
    [0, 10],
    [5, 10],
    [5, 20],
    [10, 20],
  ];
  assert.deepEqual(maskBoundaryPolyline(boundary), expected);
  assert.deepEqual(maskBoundaryPolyline([...boundary].reverse()), expected);
  const rotated = boundary.map(([x, y]): Point => [30 - y, x]);
  assert.deepEqual(maskBoundaryPolyline(rotated), [
    [10, 10],
    [20, 10],
    [30, 10],
  ]);
});

test("open masking polylines retain valleys without an invented closing edge", () => {
  const boundary: Point[] = [
    [0, 10],
    [5, 0],
    [10, 10],
  ];
  assert.deepEqual(maskBoundaryPolyline(boundary, false), boundary);
  assert.deepEqual(maskBoundaryPolyline([...boundary].reverse(), false), boundary);
  assert.notDeepEqual(maskBoundaryPolyline(boundary), boundary);
  assert.deepEqual(
    maskBoundaryPolyline(
      [
        [3, 4],
        [8, 9],
      ],
      false,
    ),
    [
      [3, 4],
      [8, 9],
    ],
  );
  assert.deepEqual(
    maskBoundaryPolyline(
      boundary.map(([x, y]) => [20 - x, 30 - y]),
      false,
    ),
    [
      [10, 20],
      [15, 30],
      [20, 20],
    ],
  );
});

test("open boundary endpoint steps survive placement and reversed ordering", () => {
  const boundary: Point[] = [
    [10, 20],
    [20, 10],
    [20, 11],
  ];
  assert.deepEqual(maskBoundaryPolyline(boundary, false), boundary);
  assert.deepEqual(maskBoundaryPolyline([...boundary].reverse(), false), boundary);
  assert.throws(
    () =>
      maskBoundaryPolyline(
        [
          [10, 10],
          [10, 20],
        ],
        false,
      ),
    /collapses/,
  );
});

test("rasterized mask coverage preserves holes and triangle winding", () => {
  const geometry = [
    ...rectangle(0, 0, 6, 2),
    ...rectangle(0, 4, 6, 2),
    ...rectangle(0, 2, 2, 2),
    ...rectangle(4, 2, 2, 2),
  ];
  const masks = rasterizeMaskGeometry(geometry, rules);
  assert.equal(masks.length, 1);
  const expected = ["111111", "111111", "110011", "110011", "111111", "111111"].join("");
  assert.equal(decode(masks[0]!).join(""), expected);
  assert.deepEqual(
    rasterizeMaskGeometry(
      geometry.map(([a, b, c]) => [c, b, a]),
      rules,
    ),
    masks,
  );
});

test("wide masks tile without seams or overflowing native scanline lengths", () => {
  const masks = rasterizeMaskGeometry(rectangle(0, 0, 2050, 1), rules);
  assert.deepEqual(
    masks.map((mask) => [mask.box_top_left, mask.box_size]),
    [
      [
        [0, 0],
        [1024, 1],
      ],
      [
        [1024, 0],
        [1024, 1],
      ],
      [
        [2048, 0],
        [2, 1],
      ],
    ],
  );
  assert.ok(masks.flatMap(decode).every((pixel) => pixel === 1));
  assert.equal(masks.flatMap(decode).length, 2050);
});

test("mask coverage projects elevation and preserves transparent edge-on states", () => {
  const geometry = rectangle(10, 20, 2, 2).map(
    (triangle): MaskTriangle => triangle.map(([x, y]) => [x, y, 5]) as MaskTriangle,
  );
  assert.deepEqual(rasterizeMaskGeometry(geometry, rules)[0]!.box_top_left, [10, 15]);
  const edge = rasterizeMaskGeometry(
    [
      [
        [0, 0, 0],
        [0, 2, 0],
        [0, 2, 2],
      ],
    ],
    rules,
  );
  assert.ok(decode(edge[0]!).every((pixel) => pixel === 0));
  assert.throws(() => rasterizeMaskGeometry(rectangle(32766, 0, 2, 2), rules), /16-bit/);
});
