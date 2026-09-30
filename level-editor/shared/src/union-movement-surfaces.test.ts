import test from "node:test";
import assert from "node:assert/strict";
import clipping, { type Polygon } from "polygon-clipping";
import { unionMovementSurfaces } from "./union-movement-surfaces.ts";

test("movement surface union recovers overlapping integer triangles that break the sweep tree", () => {
  const input: Polygon[] = [
    [
      [
        [785, 577],
        [789, 575],
        [788, 576],
        [785, 577],
      ],
    ],
    [
      [
        [793, 572],
        [793, 573],
        [785, 577],
        [793, 572],
      ],
    ],
    [
      [
        [782, 579],
        [785, 577],
        [793, 572],
        [782, 579],
      ],
    ],
  ];
  const warnings: string[] = [];
  const result = unionMovementSurfaces(input, "Movement layer 0", warnings);
  assert.equal(result.length, 1);
  assert.equal(result[0]!.length, 1);
  const ring = result[0]![0]!;
  const area =
    Math.abs(
      ring.reduce((sum, p, i) => {
        const q = ring[(i + 1) % ring.length]!;
        return sum + p[0] * q[1] - p[1] * q[0];
      }, 0),
    ) / 2;
  assert.ok(Math.abs(area - 5.15) < 0.00001, `Unexpected union area: ${area}`);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0]!, /fixed-point clipping/);
});

test("ordinary movement unions preserve contour ordering without warnings", () => {
  const input: Polygon[] = [
    [
      [
        [0, 0],
        [10, 0],
        [10, 10],
        [0, 10],
      ],
    ],
    [
      [
        [20, 0],
        [30, 0],
        [30, 10],
        [20, 10],
      ],
    ],
  ];
  const warnings: string[] = [];
  assert.deepEqual(
    unionMovementSurfaces(input, "Movement layer 0", warnings),
    clipping.union(...input),
  );
  assert.deepEqual(warnings, []);
});
