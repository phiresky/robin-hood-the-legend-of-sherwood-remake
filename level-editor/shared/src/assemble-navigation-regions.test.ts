import test from "node:test";
import assert from "node:assert/strict";
import { assembleNavigationRegions, type NavigationPiece } from "./assemble-navigation-regions.ts";

test("rounded wall islands without a receiving plane are omitted without losing usable surfaces", () => {
  const fragments: Pick<NavigationPiece, "polygon" | "plane">[] = [
    {
      polygon: [
        [2273.4345703125, 1080.2275390625],
        [2273.2421875, 1080.060546875],
        [2272.5078125, 1078.86328125],
        [2272.4755859375, 1078.7939453125],
      ],
      plane: [0.03493787194283309, 0.09102298951610928, -17.585290013629404],
    },
    {
      polygon: [
        [2273.4345703125, 1080.2275390625],
        [2272.470703125, 1078.7861328125],
        [2273.494140625, 1080.2041015625],
        [2273.4970703125, 1080.2041015625],
      ],
      plane: [0.03492229742739973, 0.091033404529851, -17.561132953908384],
    },
    {
      polygon: [
        [0, 0],
        [100, 0],
        [100, 100],
        [0, 100],
      ],
      plane: [0, 0, 80],
    },
  ];
  const pieces: NavigationPiece[] = fragments.map((piece) => ({
    ...piece,
    layer: 0,
    navigationRegion: "wall",
    blockers: [],
    closeDeformationSeams: true,
  }));
  const warnings: string[] = [];
  const result = assembleNavigationRegions(pieces, warnings);
  assert.equal(result.length, 1);
  assert.deepEqual(result[0]!.polygon, pieces[2]!.polygon);
  assert.deepEqual(result[0]!.pieces[0]!.plane, pieces[2]!.plane);
  assert.match(warnings.join("\n"), /all receiving fragments collapsed.*region omitted/);
  assert.throws(
    () =>
      assembleNavigationRegions(
        pieces.slice(0, 2).map((p) => ({ ...p, lift: "stairs" })),
        [],
      ),
    /no traversable receiving area/,
  );
});

test("preserved joined boundaries retain crossing contours without blocking another surface", () => {
  const crossing: [number, number][] = [
    [-3, 3],
    [3, 4],
    [3, 8],
    [-3, 8],
  ];
  const pieces: NavigationPiece[] = [
    {
      navigationRegion: "joined",
      plane: [0, 0, 0],
      layer: 0,
      preserveMovementBoundary: true,
      polygon: [
        [0, 0],
        [10, 0],
        [10, 10],
        [0, 10],
      ],
      blockers: [
        crossing,
        [
          [10, 2],
          [14, 2],
          [14, 7],
          [10, 7],
        ],
      ],
    },
    {
      navigationRegion: "joined",
      plane: [1, 0, -10],
      layer: 1,
      preserveMovementBoundary: true,
      polygon: [
        [10, 0],
        [20, 0],
        [20, 10],
        [10, 10],
      ],
      blockers: [],
    },
  ];
  const [result] = assembleNavigationRegions(pieces, []);
  assert.equal(result!.polygon.length, 4);
  assert.equal(result!.blockers.length, 1);
  assert.deepEqual(
    new Set(result!.blockers[0]!.map((p) => JSON.stringify(p))),
    new Set(crossing.map((p) => JSON.stringify(p))),
  );
  assert.equal(result!.pieces.length, 2);
  pieces[1]!.preserveMovementBoundary = false;
  assert.throws(() => assembleNavigationRegions(pieces, []), /must agree/);
});

test("joined planes preserve holes with native obstacle winding and disconnected components", () => {
  const pieces: NavigationPiece[] = [
    {
      navigationRegion: "roof",
      plane: [0, 0, 20],
      layer: 1,
      polygon: [
        [0, 0],
        [50, 0],
        [50, 100],
        [0, 100],
      ],
      blockers: [
        [
          [10, 10],
          [10, 20],
          [20, 20],
          [20, 10],
        ],
      ],
    },
    {
      navigationRegion: "roof",
      plane: [1, 0, -30],
      layer: 2,
      polygon: [
        [50, 0],
        [100, 0],
        [100, 100],
        [50, 100],
      ],
      blockers: [],
    },
    {
      navigationRegion: "roof",
      plane: [0, 0, 50],
      layer: 3,
      polygon: [
        [200, 0],
        [300, 0],
        [300, 100],
        [200, 100],
      ],
      blockers: [],
    },
  ];
  const result = assembleNavigationRegions(pieces, []);
  assert.equal(result.length, 2);
  const joined = result.find((r) => r.pieces.length === 2)!;
  assert.equal(joined.layer, 1);
  assert.equal(joined.blockers.length, 1);
  const area = joined.blockers[0]!.reduce((sum, p, i, ring) => {
    const q = ring[(i + 1) % ring.length]!;
    return sum + p[0] * q[1] - q[0] * p[1];
  }, 0);
  assert.equal(area, 200);
});

test("joined receiving fragments that collapse to one movement pixel are omitted with a warning", () => {
  // A fractional sliver left where a generated union meets a clipped surface.
  const sliver: [number, number][] = [
    [1673.9767441860465, 730.0232558139535],
    [1674.030303030303, 729.8484848484849],
    [1674, 730],
  ];
  const pieces: NavigationPiece[] = [
    {
      navigationRegion: "terrain",
      plane: [0, 0, 0],
      layer: 0,
      polygon: [
        [1600, 700],
        [1700, 700],
        [1700, 800],
        [1600, 800],
      ],
      blockers: [],
    },
    {
      navigationRegion: "terrain",
      plane: [0.1, 0, -167.4],
      layer: 1,
      polygon: sliver,
      blockers: [],
    },
  ];
  const warnings: string[] = [];
  const result = assembleNavigationRegions(pieces, warnings);
  assert.equal(result.length, 1);
  assert.equal(result[0]!.pieces.length, 1);
  assert.deepEqual(result[0]!.polygon, pieces[0]!.polygon);
  assert.match(warnings.join("\n"), /Joined receiving fragment.*collaps/i);
});
