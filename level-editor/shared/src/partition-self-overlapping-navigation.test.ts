import test from "node:test";
import assert from "node:assert/strict";
import { assembleNavigationRegions, type NavigationPiece } from "./assemble-navigation-regions.ts";
import { partitionSelfOverlappingNavigation } from "./partition-self-overlapping-navigation.ts";
import { warnSelfOverlappingNavigation } from "./warn-self-overlapping-navigation.ts";
import { navigationPosturePassages } from "./navigation-posture-passages.ts";
import type { HeightPlane } from "./gameplay-plane.ts";
import type { Point } from "./level.ts";

function rectangle(
  plane: HeightPlane,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
): NavigationPiece {
  return {
    navigationRegion: "asset/loop",
    layer: 0,
    plane,
    blockers: [],
    polygon: [
      [x0, y0],
      [x1, y0],
      [x1, y1],
      [x0, y1],
    ],
  };
}
function loop() {
  return [
    rectangle([0, 0, 0], 0, 0, 200, 100),
    rectangle([0, 0, 0], 200, 0, 300, 100),
    rectangle([0, 0.4, -40], 200, 100, 300, 300),
    rectangle([0, 0, 80], 200, 300, 300, 400),
    rectangle([0, 0, 80], 0, 300, 200, 400),
    rectangle([0, 0, 80], 0, 0, 100, 300),
  ];
}

test("folded receivers retain a physical seam in either partition order", () => {
  for (const reverse of [false, true]) {
    const pieces = [
      rectangle([0, 0, 100], 0, -120, 100, -100),
      rectangle([0, -2, -100], 0, -120, 100, -100),
    ];
    if (reverse) pieces.reverse();
    const [region] = assembleNavigationRegions(pieces, []);
    assert.ok(region);
    const before = structuredClone(region);
    const result = partitionSelfOverlappingNavigation(region, []);
    assert.equal(result.regions.length, 2);
    assert.equal(result.passages.length, 1);
    const { from, to, connection } = result.passages[0]!;
    const floor = (r: typeof region) => ({
      ...r,
      plane: r.pieces[0]!.plane,
      receivingFloors: r.pieces.map((p) => ({ plane: p.plane, polygon: p.polygon })),
    });
    assert.ok(navigationPosturePassages(connection, floor(from), floor(to)).length);
    assert.deepEqual(region, before);
  }
});

test("a collapsed singleton exports integer movement while retaining its physical receiver", () => {
  const piece = rectangle([0, -6, -500], 0, -104, 100, -100);
  piece.worldPlane = [0, 1.2, 100];
  piece.receivingPolygon = structuredClone(piece.polygon);
  const [region] = assembleNavigationRegions([piece], []);
  assert.ok(region);
  const before = structuredClone(region);
  const result = partitionSelfOverlappingNavigation(region, []);
  assert.equal(result.regions.length, 1);
  const prepared = result.regions[0]!;
  assert.ok(prepared.polygon.flat().every(Number.isInteger));
  assert.notDeepEqual(prepared.polygon, before.polygon);
  assert.deepEqual(prepared.pieces[0]!.receivingPolygon, piece.receivingPolygon);
  assert.deepEqual(region, before);
});

test("partitioned loop retains receiving geometry and connects only matching-height boundaries", () => {
  for (const reverse of [false, true]) {
    const pieces = loop();
    if (reverse) pieces.reverse();
    const [region] = assembleNavigationRegions(pieces, []);
    assert.ok(region);
    const before = structuredClone(region);
    const warnings: string[] = [];
    const result = partitionSelfOverlappingNavigation(region, warnings);
    assert.equal(result.regions.length, 2);
    assert.equal(
      result.regions.reduce((sum, r) => sum + r.pieces.length, 0),
      6,
    );
    assert.ok(
      result.regions.every((r) => r.pieces.every((p) => p.navigationRegion === "asset/loop")),
    );
    warnSelfOverlappingNavigation(result.regions, warnings);
    assert.deepEqual(warnings, []);
    assert.deepEqual(region, before);
    assert.equal(result.passages.length, 1);
    for (const { from, to, connection } of result.passages) {
      assert.ok(connection.from.edge.every((p) => p[2] === (reverse ? 0 : 80)));
      const floor = (r: typeof region) => ({ ...r, plane: r.pieces[0]!.plane });
      assert.ok(navigationPosturePassages(connection, floor(from), floor(to)).length > 0);
    }
  }
});

test("a receiver that collapses during partitioning retains the original joined region", () => {
  const [region] = assembleNavigationRegions(loop(), []);
  assert.ok(region);
  region.pieces.push(rectangle([0, 0, 40], 20.1, 20, 20.2, 80));
  const before = structuredClone(region);
  const warnings: string[] = [];
  const result = partitionSelfOverlappingNavigation(region, warnings);
  assert.deepEqual(result, { regions: [region], passages: [] });
  assert.deepEqual(region, before);
  assert.ok(warnings.length > 0);
});

test("partition planning leaves ordinary slopes and lift regions untouched", () => {
  const [flat] = assembleNavigationRegions(loop().slice(0, 5), []);
  assert.ok(flat);
  assert.deepEqual(partitionSelfOverlappingNavigation(flat, []), { regions: [flat], passages: [] });
  const [overlap] = assembleNavigationRegions(loop(), []);
  assert.ok(overlap);
  overlap.lift = "ladder";
  assert.deepEqual(partitionSelfOverlappingNavigation(overlap, []), {
    regions: [overlap],
    passages: [],
  });
});

test("rotated partial seams share rounded vertices without changing source geometry", () => {
  const angle = (37 * Math.PI) / 180;
  const c = Math.cos(angle),
    s = Math.sin(angle);
  const pieces = loop().map((piece): NavigationPiece => {
    const receivingPolygon = piece.polygon.map(([x, y]): Point => [
      500 + c * x - s * y,
      500 + s * x + c * y,
    ]);
    const a = piece.plane[0] * c - piece.plane[1] * s;
    const b = piece.plane[0] * s + piece.plane[1] * c;
    return {
      ...piece,
      receivingPolygon,
      polygon: receivingPolygon.map(([x, y]): Point => [Math.round(x), Math.round(y)]),
      plane: [a, b, piece.plane[2] - 500 * (a + b)],
    };
  });
  const [region] = assembleNavigationRegions(pieces, []);
  assert.ok(region);
  const before = structuredClone(region);
  const result = partitionSelfOverlappingNavigation(region, []);
  assert.equal(result.regions.length, 2);
  assert.equal(result.passages.length, 1);
  for (const { from, to, connection } of result.passages)
    assert.ok(
      navigationPosturePassages(
        connection,
        { ...from, plane: from.pieces[0]!.plane },
        { ...to, plane: to.pieces[0]!.plane },
      ).length > 0,
    );
  assert.ok(result.regions.every((r) => r.polygon.every((p) => p.every(Number.isInteger))));
  assert.deepEqual(region, before);
});

test("a lower receiver under a hole does not create a separate partition", () => {
  const high = rectangle([0, 0, 80], 0, 0, 100, 100);
  high.blockers = [
    [
      [20, 20],
      [80, 20],
      [80, 80],
      [20, 80],
    ],
  ];
  const low = rectangle([0, 0, 0], 30, 30, 70, 70);
  const region = { layer: 0, polygon: high.polygon, blockers: [], pieces: [high, low] };
  assert.deepEqual(partitionSelfOverlappingNavigation(region, []), {
    regions: [region],
    passages: [],
  });
});

test("adjacent boundaries at different heights do not produce a passage", () => {
  const [region] = assembleNavigationRegions(
    [rectangle([0, 0, 0], 0, 0, 100, 100), rectangle([0, 0, 80], 50, 0, 150, 100)],
    [],
  );
  assert.ok(region);
  const result = partitionSelfOverlappingNavigation(region, []);
  assert.equal(result.regions.length, 2);
  assert.deepEqual(result.passages, []);
});

test("a raised ramp reconnects along an internal ground hole", () => {
  for (const reversed of [false, true]) {
    const ground = rectangle([0, 0, 0], 0, 0, 300, 300);
    const hole: Point[] = [
      [100, 100],
      [200, 100],
      [200, 200],
      [100, 200],
    ];
    ground.blockers = [reversed ? hole.toReversed() : hole];
    const ramp = rectangle([0, 0.4, -40], 100, 100, 200, 250);
    const [region] = assembleNavigationRegions([ground, ramp], []);
    assert.ok(region);
    const before = structuredClone(region);
    const result = partitionSelfOverlappingNavigation(region, []);
    assert.equal(result.regions.length, 2);
    assert.equal(result.passages.length, 1);
    const { from, to, connection } = result.passages[0]!;
    assert.ok(connection.from.edge.every((p) => p[2] === 0));
    assert.ok(
      navigationPosturePassages(
        connection,
        { ...from, plane: from.pieces[0]!.plane },
        { ...to, plane: to.pieces[0]!.plane },
      ).length > 0,
    );
    const closed = structuredClone(to);
    closed.blockers.push([
      [100, 100],
      [200, 100],
      [200, 120],
      [100, 120],
    ]);
    assert.deepEqual(
      navigationPosturePassages(
        connection,
        { ...from, plane: from.pieces[0]!.plane },
        { ...closed, plane: closed.pieces[0]!.plane },
      ),
      [],
    );
    assert.deepEqual(region, before);
  }
});
