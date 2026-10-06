import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import type { Polygon } from "polygon-clipping";
import {
  partitionMovementObstacles,
  partitionPreciseMovementObstacles,
} from "./partition-movement-obstacles.ts";
import { normalizeGeneratedMotion } from "./normalize-generated-motion.ts";
import { quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import polygonClipping from "polygon-clipping";

test("terrace collision retains exact partitions around its stair landing", () => {
  const region: Polygon = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/partitioned-terrace-collision.json", import.meta.url),
      "utf8",
    ),
  );
  const rounded = normalizeGeneratedMotion([region], "terrace", []);
  const result = partitionPreciseMovementObstacles(region, rounded);
  assert.ok(result);
  assert.equal(result.exact.length, 14);
  assert.deepEqual(
    polygonClipping.xor(polygonClipping.union(result.grid.map((p) => [p])), rounded),
    [],
  );
  for (const [i, exact] of result.exact.entries()) {
    const grid = quantizeGeneratedMotionPolygon([exact], Math.round, "partition", []);
    assert.ok(grid);
    assert.deepEqual(polygonClipping.xor(grid, [result.grid[i]!]), []);
  }
  // A point inside the authored landing must remain clear of every triangle.
  const landing: Polygon = [
    [
      [3425, 2843.5],
      [3425.01, 2843.5],
      [3425.01, 2843.51],
      [3425, 2843.51],
    ],
  ];
  assert.deepEqual(polygonClipping.intersection(region, landing), []);
  assert.deepEqual(
    polygonClipping.intersection(polygonClipping.union(result.exact.map((p) => [p])), landing),
    [],
  );
});

test("integer obstacle islands retain exact coverage across shared triangulation edges", () => {
  const region: Polygon = [
    [
      [2941, 1360],
      [1114, 1403],
      [1537, 1226],
      [1457, 1084],
      [1471, 1040],
      [1525, 1025],
      [1823, 1196],
      [1909, 1180],
      [2941, 454],
    ],
    [
      [2116, 1205],
      [2155, 1209],
      [2116, 1204],
    ],
    [
      [1480, 1070],
      [1471, 1088],
      [1482, 1070],
      [1532, 1052],
    ],
    [
      [2586, 1068],
      [2821, 1000],
      [2889, 919],
    ],
  ];
  const pieces = partitionMovementObstacles(region);
  assert(pieces.flat().every((point) => point.every(Number.isInteger)));
  assert.deepEqual(polygonClipping.xor(region, polygonClipping.union(pieces.map((p) => [p]))), []);
});

test("recovery partitions a nearly touching hole without filling it", () => {
  const region: Polygon = [
    [
      [206110849 / 1048576, 611038976 / 1048576],
      [194, 582],
      [177, 602],
      [158, 592],
      [185292163 / 1048576, 603101552 / 1048576],
      [206110849 / 1048576, 611038976 / 1048576],
    ],
    [
      [177250104 / 1048576, 610339406 / 1048576],
      [166651440 / 1048576, 619878203 / 1048576],
      [185455136 / 1048576, 628313974 / 1048576],
      [192443445 / 1048576, 623189231 / 1048576],
      [200043218 / 1048576, 614248321 / 1048576],
      [182594306 / 1048576, 606420337 / 1048576],
      [177250104 / 1048576, 610339406 / 1048576],
    ],
  ];
  assert.throws(() => partitionMovementObstacles(region), /changed coverage/);
  const pieces = partitionMovementObstacles(region, true);
  const delta = fixedPolygonBoolean(
    "xor",
    region,
    pieces.map((p) => [p]),
  );
  const area = delta.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce(
        (s, ring, i) =>
          s +
          ((i ? -1 : 1) *
            Math.abs(
              ring.reduce((a, p, j) => {
                const q = ring[(j + 1) % ring.length]!;
                return a + p[0] * q[1] - q[0] * p[1];
              }, 0),
            )) /
            2,
        0,
      ),
    0,
  );
  assert(area < 0.001, `Partition coverage error: ${area}`);
});
