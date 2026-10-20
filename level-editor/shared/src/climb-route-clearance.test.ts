import assert from "node:assert/strict";
import test from "node:test";
import clipping from "polygon-clipping";
import { climbRouteClearance } from "./climb-route-clearance.ts";
import { supportedFootprintCenters } from "./supported-footprint-centers.ts";
import { pointInGameplayPolygon } from "./navigation-anchor.ts";
import type { Point } from "./level.ts";

const folded: Point[] = [
  [0, 328],
  [1, 325],
  [9, 309],
  [36, 41],
  [40, 31],
  [41, 30],
  [43, 0],
  [30, 21],
  [22, 72],
  [30, 53],
];
const entries: Point[] = [
  [6, 304],
  [28, 49],
  [39, 25],
];

test("folded climb prepares connected body clearance without unsupported root positions", () => {
  assert.deepEqual(supportedFootprintCenters([[folded]], [[folded]], [6, 3]), []);
  const envelope = climbRouteClearance(folded, entries);
  assert.ok(envelope);
  const centers = supportedFootprintCenters([[envelope]], [[envelope]], [6, 3]);
  assert.equal(centers.length, 1);
  for (const entry of entries) assert.ok(pointInGameplayPolygon(entry, centers[0]![0]!, true));
  assert.deepEqual(clipping.difference(centers, [folded]), []);
  assert.deepEqual(clipping.difference([folded], [envelope]), []);
});

test("climb envelopes preserve already supported floors and reject invalid entries", () => {
  const square: Point[] = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ];
  assert.equal(
    climbRouteClearance(square, [
      [20, 20],
      [80, 80],
    ]),
    undefined,
  );
  assert.equal(climbRouteClearance(folded, [[-100, -100], ...entries]), undefined);
  assert.equal(climbRouteClearance(folded, []), undefined);
  assert.equal(climbRouteClearance(folded, [[NaN, 0], ...entries]), undefined);
});

test("subpixel branch contours retain supported routes after elevation changes", () => {
  const floor: Point[] = [
    [1669.939006286443, 1817.356832077877],
    [1671.2635191390434, 1814.4929463931758],
    [1678.6929388929448, 1798.4289204102947],
    [1706.214516772886, 1529.6987115534494],
    [1710.4966899068268, 1520.4397192276092],
    [1711.0264950478668, 1519.2941649537324],
    [1712.7392654132814, 1488.657599867528],
    [1699.76046864946, 1509.6128189982162],
    [1691.9365030793456, 1560.5708914214056],
    [1700.4231035594598, 1542.2210101204744],
  ];
  const anchors: Point[] = [
    [1676, 1793],
    [1698, 1538],
    [1709, 1514],
  ];
  for (const elevation of [0, 40]) {
    // Collinear split points acquire small independent errors during projection.
    const projected: Point[] = floor.map(([x, y], i) => [
      x,
      y - elevation + (elevation ? (i % 2 ? 1e-11 : -1e-11) : 0),
    ]);
    const goals: Point[] = anchors.map(([x, y]) => [x, y - elevation]);
    const envelope = climbRouteClearance(projected, goals);
    assert.ok(envelope);
    const centers = supportedFootprintCenters([[envelope]], [[envelope]], [6, 3]);
    assert.deepEqual(clipping.difference(centers, [projected]), []);
    assert.ok(
      centers.some((polygon) =>
        goals.every((point) => pointInGameplayPolygon(point, polygon[0]!, true)),
      ),
    );
  }
});
