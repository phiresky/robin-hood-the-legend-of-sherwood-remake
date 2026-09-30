import test from "node:test";
import assert from "node:assert/strict";
import { createTerrainGrid, terrainTriangles, triangleHeightAt } from "./authored-terrain.ts";
import { roadTerrainPieces } from "./terrain-path-gameplay.ts";
import type { LevelSpline } from "./splines.ts";
const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
function ground() {
  const terrain = createTerrainGrid([0, 0, 200, 200], 200);
  for (const v of terrain.vertices) v.position[2] = v.position[0] * 0.2 + v.position[1] * 0.1;
  return terrainTriangles({ terrain });
}
function road(overrides: Partial<LevelSpline> = {}): LevelSpline {
  return {
    id: "road",
    name: "Road",
    kind: "road",
    closed: false,
    width: 40,
    points: [
      [0, 100, 0],
      [200, 100, 0],
    ],
    repeatLength: 50,
    pointMaterials: ["path_dirt", "path_dirt"],
    ...overrides,
  };
}
function materialAt(pieces: ReturnType<typeof ground>, x: number, y: number) {
  return pieces.find((t) => triangleHeightAt(t.points, x, y) !== undefined)?.cell.material;
}
test("roads partition ground without changing its area or plane heights", () => {
  const pieces = roadTerrainPieces({ camera, splines: [road()] }, ground());
  let area = 0;
  for (const t of pieces) {
    const [a, b, c] = t.points;
    area += Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2;
    for (const [x, y, z] of t.points) assert.ok(Math.abs(z - x * 0.2 - y * 0.1) < 1e-8);
  }
  assert.ok(Math.abs(area - 40000) < 1e-5);
  assert.equal(materialAt(pieces, 51, 101), "path_dirt");
  assert.equal(materialAt(pieces, 51, 150), "grass_short");
});
test("width and point material mixes determine gameplay strips", () => {
  const pieces = roadTerrainPieces(
    {
      camera,
      splines: [
        road({
          pointWidths: [10, 100],
          pointMaterialMixes: [
            { path_dirt: 0.9, road_cobblestone: 0.1 },
            { path_dirt: 0.1, road_cobblestone: 0.9 },
          ],
        }),
      ],
    },
    ground(),
  );
  assert.equal(materialAt(pieces, 11, 115), "grass_short");
  assert.equal(materialAt(pieces, 181, 115), "road_cobblestone");
  assert.equal(materialAt(pieces, 11, 101), "path_dirt");
});
test("later crossing roads take material precedence and distant ground is reused", () => {
  const base = ground();
  const crossing = road({
    id: "cross",
    width: 40,
    points: [
      [100, 0, 0],
      [100, 200, 0],
    ],
    pointMaterials: ["road_cobblestone", "road_cobblestone"],
  });
  const pieces = roadTerrainPieces({ camera, splines: [road(), crossing] }, base);
  assert.equal(materialAt(pieces, 101, 101), "road_cobblestone");
  assert.equal(materialAt(pieces, 31, 101), "path_dirt");
  const distant = road({
    points: [
      [1000, 1000, 0],
      [1200, 1000, 0],
    ],
  });
  const untouched = roadTerrainPieces({ camera, splines: [distant] }, base);
  assert.equal(untouched[0], base[0]);
});

test("road cuts retain barycentric material mixtures on uncovered ground", () => {
  const original = ground()[0]!;
  original.materialMixes = [{ grass_short: 1 }, { ground_sand: 1 }, { ground_mud: 1 }];
  const pieces = roadTerrainPieces({ camera, splines: [road()] }, [original]);
  const uncovered = pieces.filter((p) => p.cell.material === original.cell.material);
  assert.ok(uncovered.length > 0);
  for (const piece of uncovered) {
    assert.equal(piece.materialMixes, original.materialMixes);
    piece.points.forEach((point, i) => {
      const weights = piece.materialWeights![i]!;
      for (let axis = 0; axis < 3; axis++) {
        const reconstructed = original.points.reduce(
          (sum, source, j) => sum + source[axis]! * weights[j]!,
          0,
        );
        assert.ok(Math.abs(reconstructed - point[axis]!) < 1e-7);
      }
    });
  }
});

test("curved road cuts with near-coincident edges preserve ground and road coverage", () => {
  // Successive strip intersections used to overflow the floating-point sweep queue here.
  const original = ground()[0]!;
  original.points = [
    [1792, 1027.8489739410745, 0],
    [1920, 1101.2667577940083, 0],
    [1792, 1101.2667577940083, 0],
  ];
  for (const p of original.points) p[2] = p[0] * 0.2 + p[1] * 0.1;
  const curved = road({
    width: 193.71,
    repeatLength: 180,
    points: [
      [325.73, 852.2597039459637, 0],
      [761.7244462957661, 925.2436421573115, 0],
      [1222.987605469606, 1077.5562631503121, 0],
      [1470.959039974546, 1579.482748662872, 0],
      [1946.9504354817886, 1023.3710085852365, 0],
    ],
    pointMaterials: ["path_flagstone", "water_white_stone", "path_dirt", "path_dirt", "path_dirt"],
  });
  const pieces = roadTerrainPieces({ camera, splines: [curved] }, [original]);
  const area = ({ points: [a, b, c] }: (typeof pieces)[number]) =>
    Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2;
  const coverage = new Map<string, number>();
  for (const piece of pieces) {
    coverage.set(piece.cell.material, (coverage.get(piece.cell.material) ?? 0) + area(piece));
    for (const [x, y, z] of piece.points) assert.ok(Math.abs(z - x * 0.2 - y * 0.1) < 1e-8);
  }
  assert.ok(
    Math.abs(pieces.reduce((sum, piece) => sum + area(piece), 0) - area(original)) < 0.0001,
  );
  assert.deepEqual([...coverage.keys()].sort(), ["grass_short", "path_dirt"]);
  assert.ok(Math.abs(coverage.get("path_dirt")! - 3827.188) < 0.001);
  assert.ok(Math.abs(coverage.get("grass_short")! - 871.55) < 0.001);
});
