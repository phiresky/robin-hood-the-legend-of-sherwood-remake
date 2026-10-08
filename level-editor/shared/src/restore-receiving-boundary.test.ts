import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import clipping, { type MultiPolygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import { restoreReceivingBoundary, restoreObstacleBoundary } from "./restore-receiving-boundary.ts";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";

test("near-coincident terrain obstacle edges survive a floating sweep failure", () => {
  const boundary: Point[] = [
    [1639, 1439],
    [1636, 1441],
    [1634, 1442],
  ];
  const sources: MultiPolygon = [
    [
      [
        [1637.125, 1440.125],
        [1639, 1439],
        [1636, 1441],
        [1634, 1442],
        [1639, 1438.666666984558],
        [1637.125, 1440.125],
      ],
    ],
  ];
  const restored = restoreObstacleBoundary(boundary, sources);
  assert.ok(restored);
  const rounded = quantizeGeneratedMotionPolygon([restored], Math.round, "Obstacle", []);
  assert.ok(rounded);
  assert.deepEqual(fixedPolygonBoolean("xor", rounded, [[boundary]]), []);
  assert.deepEqual(fixedPolygonBoolean("difference", [restored], [sources]), []);
});

test("a separate subpixel solid cannot erase an unambiguous landing edge", () => {
  const fixture: { points: Point[]; blockedCoverage: MultiPolygon } = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/partial-obstacle-edge-recovery.json", import.meta.url),
      "utf8",
    ),
  );
  const island: Point[] = [
    [1614.005, 1832.591],
    [1614.007, 1832.591],
    [1614.007, 1832.593],
    [1614.005, 1832.593],
  ];
  assert.deepEqual(clipping.intersection([island], fixture.blockedCoverage), []);
  assert.deepEqual(clipping.difference([island], [fixture.points]), []);
  const falseStrip: Point[] = [
    [1831.9, 1728.6],
    [1832, 1728.6],
    [1832, 1728.7],
    [1831.9, 1728.7],
  ];
  for (const rotation of [0, 1, 2, 3]) {
    const place = ([x, y]: Point): Point => {
      for (let i = 0; i < rotation; i++) [x, y] = [-y, x];
      return [x + rotation * 4000, y + rotation * 5000];
    };
    const boundary = fixture.points.map(place);
    const sources = [...fixture.blockedCoverage, [island]].map((p) => p.map((r) => r.map(place)));
    const restored = restoreObstacleBoundary(boundary, sources);
    assert.ok(restored, `quarter turns: ${rotation}`);
    const rounded = quantizeGeneratedMotionPolygon([restored], Math.round, "Restored islands", []);
    assert.ok(rounded);
    assert.deepEqual(clipping.xor(rounded, [boundary]), []);
    assert.deepEqual(clipping.difference([island.map(place)], [restored]), []);
    assert.deepEqual(clipping.intersection([restored], [falseStrip.map(place)]), []);
    assert.equal(restoreObstacleBoundary(boundary, [...sources, ...sources]), undefined);
  }
});

test("obstacle edge interiors survive unrelated rounded chains and cropped frame strips", () => {
  const fixture: { points: Point[]; blockedCoverage: MultiPolygon } = JSON.parse(
    readFileSync(
      new URL("../test-fixtures/partial-obstacle-edge-recovery.json", import.meta.url),
      "utf8",
    ),
  );
  for (const elevation of [0, 40]) {
    const boundary = fixture.points.map(([x, y]): Point => [x, y + elevation]);
    const sources = fixture.blockedCoverage.map((p) =>
      p.map((r) => r.map(([x, y]): Point => [x, y + elevation])),
    );
    const restored = restoreObstacleBoundary(boundary, sources);
    assert.ok(restored);
    const rounded = quantizeGeneratedMotionPolygon(
      [restored],
      Math.round,
      "Recovered obstacle",
      [],
    );
    assert.ok(rounded);
    assert.deepEqual(clipping.xor(rounded, [boundary]), []);
    // The middle of the stair contact moves onto its authored edge. Rounded
    // endpoint caps and unrelated collision remain represented.
    const falseStrip: Point[] = [
      [1831.9, 1728.6 + elevation],
      [1832, 1728.6 + elevation],
      [1832, 1728.7 + elevation],
      [1831.9, 1728.7 + elevation],
    ];
    assert.ok(clipping.intersection([boundary], [falseStrip]).length);
    assert.deepEqual(clipping.intersection([restored], [falseStrip]), []);
    const existing = fixedPolygonBoolean("intersection", sources, [[boundary]]);
    const lost = fixedPolygonBoolean("difference", existing, [[restored]]);
    const area = lost
      .flatMap((p) => p)
      .reduce(
        (total, ring) =>
          total +
          Math.abs(
            ring.reduce((sum, [x, y], i) => {
              const next = ring[(i + 1) % ring.length]!;
              return sum + x * next[1] - next[0] * y;
            }, 0),
          ) /
            2,
        0,
      );
    assert.ok(area < 1e-7, `only clipping-grid noise may differ: ${area}`);
    assert.equal(restoreObstacleBoundary(boundary, [...sources, ...sources]), undefined);
  }
});

const notches: { name: string; boundary: Point[]; blocked: MultiPolygon }[] = JSON.parse(
  readFileSync(new URL("../test-fixtures/rounded-notch-collision.json", import.meta.url), "utf8"),
);
for (const fixture of notches)
  test(`restore exact obstacle: ${fixture.name}`, () => {
    const restored = restoreObstacleBoundary(fixture.boundary, fixture.blocked);
    assert.ok(restored);
    assert.deepEqual(fixedPolygonBoolean("difference", [[restored]], [fixture.blocked]), []);
    assert.deepEqual(
      clipping.xor(
        [restored.map(([x, y]): Point => [Math.round(x), Math.round(y)])],
        [fixture.boundary],
      ),
      [],
    );
    assert.ok(restored.some((p) => p.some((v) => v !== Math.round(v))));
    // An unrelated source or competing source ownership must not authorize a cut.
    assert.equal(
      restoreObstacleBoundary(
        fixture.boundary,
        fixture.blocked.map((polygon) =>
          polygon.map((ring) => ring.map(([x, y]): Point => [x + 100, y])),
        ),
      ),
      undefined,
    );
    assert.equal(
      restoreObstacleBoundary(fixture.boundary, [...fixture.blocked, ...fixture.blocked]),
      undefined,
    );
  });

test("half-grid endpoint candidates use the interior of their rounding cell", () => {
  const source: Point[] = [
    [910.2813222477323, 1346.5067106699871],
    [976.4406082501519, 1332.5480393251266],
    [981.8100804105238, 1357.997484731754],
    [945.4592418670654, 1366.1088275909424],
    [929.4592714309692, 1343.1088275909424],
    [921.227822303772, 1344.2542276382446],
    [941.125862121582, 1372.4422283172607],
    [982.9398568617387, 1363.352235302785],
    [992.9558872647501, 1410.8247635735902],
    [926.7966012623306, 1424.7834349184504],
  ];
  const boundary: Point[] = [
    [929, 1343],
    [976, 1333],
    [982, 1358],
    [945, 1366],
  ];
  const restored = restoreReceivingBoundary(boundary, [[source]]);
  assert.ok(restored);
  assert.deepEqual(clipping.difference([restored], [source]), []);
  assert.deepEqual(
    clipping.xor([restored.map(([x, y]): Point => [Math.round(x), Math.round(y)])], [boundary]),
    [],
  );
  assert.ok(restored.some((point) => point[0] === source[1]![0] && point[1] === source[1]![1]));
});

test("split movement regions retain source edges without filling collision cuts", () => {
  const source: Point[] = [
    [978.7127251485637, 1339.149050189603],
    [1023.0824813075121, 1336.0692473360036],
    [1037.224175453186, 1356.919231414795],
    [997.1248989105225, 1366.0964374542236],
    [999.1218433380127, 1370.8178033828735],
    [1048.2661209106445, 1359.4047565460205],
    [1032.4182026959515, 1335.4212342779754],
    [1115.1840292112634, 1329.6762753070668],
    [1139.5202300385881, 1367.623682803864],
    [1003.0489259758884, 1377.0964576864008],
  ];
  const boundary: Point[] = [
    [979, 1339],
    [1023, 1336],
    [1037, 1357],
    [997, 1366],
    [999, 1370],
  ];
  const restored = restoreReceivingBoundary(boundary, [[source]]);
  assert.ok(restored);
  assert.ok(restored.some((p) => p[0] === source[0]![0] && p[1] === source[0]![1]));
  assert.deepEqual(clipping.difference([restored], [source]), []);
  assert.deepEqual(
    clipping.xor([restored.map(([x, y]): Point => [Math.round(x), Math.round(y)])], [boundary]),
    [],
  );
  // Neither an unrelated contour nor two competing owners can restore a region.
  const distant: MultiPolygon = [[source.map(([x, y]) => [x + 10, y])]];
  assert.equal(restoreReceivingBoundary(boundary, distant), undefined);
  assert.equal(restoreReceivingBoundary(boundary, [[source], [source]]), undefined);
});
