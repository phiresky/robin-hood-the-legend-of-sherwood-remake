import test from "node:test";
import assert from "node:assert/strict";
import { closeNavigationSeams } from "./close-navigation-seams.ts";
import type { MultiPolygon } from "polygon-clipping";

test("deformation cracks close without filling authored openings or joining separated decks", () => {
  const deck: MultiPolygon = [
    [
      [
        [0, 0],
        [10, 0],
        [10, 20],
        [0, 20],
      ],
      [
        [3, 3],
        [3, 7],
        [7, 7],
        [7, 3],
      ],
    ],
    [
      [
        [10.001, 0],
        [20, 0],
        [20, 20],
        [10.001, 20],
      ],
    ],
  ];
  const joined = closeNavigationSeams(deck);
  assert.equal(joined.length, 1);
  assert.equal(joined[0]!.length, 2);
  const points = joined[0]![0]!;
  assert.ok(points.every(([x, y]) => x >= -1e-5 && x <= 20.00001 && y >= -1e-5 && y <= 20.00001));
  const separated = structuredClone(deck);
  separated[1]![0]!.forEach((p) => (p[0] += 1));
  assert.equal(closeNavigationSeams(separated).length, 2);
});
