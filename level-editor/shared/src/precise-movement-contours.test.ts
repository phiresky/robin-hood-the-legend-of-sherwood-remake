import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import clipping, { type Polygon } from "polygon-clipping";
import type { Point } from "./level.ts";
import type { NavigationPiece } from "./assemble-navigation-regions.ts";
import {
  joinedBlockedCoverage,
  joinedReceivingBoundary,
  indexPreciseBlockers,
  motionBoundsKey,
} from "./precise-movement-contours.ts";
import { restoreObstacleBoundary } from "./restore-receiving-boundary.ts";
import { quantizeGeneratedMotionPolygon } from "./motion-quantization.ts";

const fixture: { points: Point[]; pieces: NavigationPiece[] } = JSON.parse(
  readFileSync(new URL("../test-fixtures/joined-landing-obstacle.json", import.meta.url), "utf8"),
);
const frame: Polygon = [
  [
    [0, 0],
    [4000, 0],
    [4000, 4000],
    [0, 4000],
  ],
];

test("multi-plane landing regions retain their joined fractional stair contact", () => {
  const fixture: { boundary: Point[]; pieces: NavigationPiece[] } = JSON.parse(
    readFileSync(new URL("../test-fixtures/joined-landing-boundary.json", import.meta.url), "utf8"),
  );
  assert.equal(fixture.pieces.length, 5);
  const individual =
    indexPreciseBlockers(fixture.pieces.map((p) => p.receivingPolygon!)).get(
      motionBoundsKey(fixture.boundary),
    ) ?? [];
  assert.equal(
    individual.filter(({ rounded }) => clipping.xor([rounded], [fixture.boundary]).length === 0)
      .length,
    0,
  );
  const restored = joinedReceivingBoundary(fixture.pieces, fixture.boundary);
  assert.ok(restored);
  const rounded = quantizeGeneratedMotionPolygon([restored], Math.round, "Joined landing", []);
  assert.ok(rounded);
  assert.deepEqual(clipping.xor(rounded, [fixture.boundary]), []);
  for (const point of [
    [290.0213518, 1415.50673288],
    [336.03327203, 1408.78963049],
  ])
    assert.ok(restored.some((p) => Math.hypot(p[0] - point[0]!, p[1] - point[1]!) < 2e-6));
  const disconnected = fixture.pieces.map((p, index) => ({
    ...p,
    polygon: p.polygon.map(([x, y]): Point => [x + index * 4000, y]),
    receivingPolygon: p.receivingPolygon!.map(([x, y]): Point => [x + index * 4000, y]),
  }));
  assert.equal(joinedReceivingBoundary(disconnected, fixture.boundary), undefined);
});

test("joined floor pieces retain the precise contact of their newly enclosed obstacle", () => {
  assert.ok(fixture.pieces.every((piece) => piece.blockers.length === 0));
  const blocked = joinedBlockedCoverage(fixture.pieces, frame);
  const restored = restoreObstacleBoundary(fixture.points, blocked);
  assert.ok(restored);
  const rounded = quantizeGeneratedMotionPolygon([restored], Math.round, "Joined obstacle", []);
  assert.ok(rounded);
  assert.deepEqual(clipping.xor(rounded, [fixture.points]), []);
  const seam: Point[] = [
    [1738.5, 1582.93],
    [1738.55, 1582.93],
    [1738.55, 1582.94],
    [1738.5, 1582.94],
  ];
  assert.ok(clipping.intersection([fixture.points], [seam]).length);
  assert.deepEqual(clipping.intersection([restored], [seam]), []);
});

test("joined blocked coverage retains each floor's precise and ambiguous holes", () => {
  const hole: Point[] = [
    [1735, 1600],
    [1740, 1600],
    [1740, 1605],
    [1735, 1605],
  ];
  const precise = hole.map(([x, y]): Point => [x + 0.25, y + 0.25]);
  for (const ambiguous of [false, true]) {
    const pieces = structuredClone(fixture.pieces);
    pieces[1]!.blockers.push(hole);
    pieces[1]!.preciseBlockers = ambiguous ? [precise, precise] : [precise];
    const blocked = joinedBlockedCoverage(pieces, frame);
    const expected = ambiguous ? hole : precise;
    assert.deepEqual(clipping.difference([expected], blocked), []);
    const probe: Point[] = [
      [1735.05, 1600.05],
      [1735.1, 1600.05],
      [1735.1, 1600.1],
      [1735.05, 1600.1],
    ];
    assert.equal(clipping.intersection([probe], blocked).length > 0, ambiguous);
  }
});
