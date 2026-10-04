import test from "node:test";
import assert from "node:assert/strict";
import { measureWallSections, wallSectionAt } from "./wall-section-profile.ts";
import type { Vec3 } from "./scene.ts";

const quad = (points: Vec3[]) => [
  [points[0]!, points[1]!, points[2]!],
  [points[0]!, points[2]!, points[3]!],
];

test("wall profiles retain varying centers and widths on either source axis", () => {
  const vertices: Vec3[] = [
    [0, 0, 0],
    [100, 20, 0],
    [100, 60, 40],
    [0, 10, 40],
  ];
  for (const axis of [0, 1] as const) {
    const points = axis === 0 ? vertices : vertices.map(([x, y, z]): Vec3 => [y, x, z]);
    const profile = measureWallSections(quad(points), axis, 0, 100);
    for (const along of [0, 0.125, 0.5, 0.875, 1]) {
      const section = wallSectionAt(profile, along);
      assert.ok(Math.abs(section.center - (5 + 35 * along)) < 1e-9);
      assert.ok(Math.abs(section.width - (10 + 30 * along)) < 1e-9);
    }
    const cropped = measureWallSections(quad(points), axis, 25, 75);
    assert.deepEqual(wallSectionAt(cropped, 0), { center: 13.75, width: 17.5 });
    assert.deepEqual(wallSectionAt(cropped, 1), { center: 31.25, width: 32.5 });
  }
});

test("a gap is rejected until the source is trimmed to a continuous section", () => {
  const left = quad([
    [0, 0, 0],
    [40, 0, 0],
    [40, 10, 20],
    [0, 10, 20],
  ]);
  const right = quad([
    [60, 0, 0],
    [100, 0, 0],
    [100, 10, 20],
    [60, 10, 20],
  ]);
  assert.throws(() => measureWallSections([...left, ...right], 0, 0, 100), /gap/);
  const cropped = measureWallSections([...left, ...right], 0, 0, 40);
  assert.ok(cropped.sections.every((s) => s.center === 5 && s.width === 10));
});

test("tapered endpoints use the adjacent nonempty cross-section", () => {
  const profile = measureWallSections(
    [
      [
        [0, 0, 0],
        [100, 10, 0],
        [0, 20, 30],
      ],
    ],
    0,
    0,
    100,
  );
  assert.deepEqual(profile.sections.at(-1), profile.sections.at(-2));
  assert.deepEqual(wallSectionAt(profile, -1), profile.sections[0]);
  assert.deepEqual(wallSectionAt(profile, 2), profile.sections.at(-1));
});

test("invalid calibration coordinates cannot produce a wall profile", () => {
  assert.throws(() => measureWallSections([], 0, 0, 0), /no length/);
  assert.throws(() => measureWallSections([], 0, NaN, 1), /no length/);
  assert.throws(() => measureWallSections([], 0, 0, 100), /no measurable/);
  assert.throws(
    () =>
      measureWallSections(
        [
          [
            [0, 0, 0],
            [100, 10, Infinity],
            [0, 20, 30],
          ],
        ],
        0,
        0,
        100,
      ),
    /finite triangles/,
  );
});
