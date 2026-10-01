import test from "node:test";
import assert from "node:assert/strict";
import {
  createTerrainGrid,
  terrainHeightAt,
  validateTerrainGrid,
  subdivideTerrainCells,
  expandTerrainGrid,
  terrainTriangles,
} from "./authored-terrain.ts";
import { compileAssetGameplay } from "./compile-asset-gameplay.ts";
import { parseLevel3D } from "./validation.ts";
import type { Level3D } from "./level3d.ts";
export function terrainFixture(): Level3D {
  return {
    version: 1,
    map: "Village",
    size: null,
    camera: { kind: "oblique-orthographic", elevation_deg: 35 },
    objects: [],
    groups: [],
    sceneAssets: [],
    terrain: createTerrainGrid([100, 100, 600, 500], 100),
  };
}
test("grid XYZ edits define one continuous triangulated surface and survive serialization", () => {
  const d = terrainFixture();
  d.terrain!.vertices.forEach((v) => {
    v.position[2] = v.position[0] * 0.1;
  });
  assert.equal(terrainHeightAt(d, 250, 250), 25);
  assert.equal(terrainHeightAt(d, 20, 20), undefined);
  assert.deepEqual(parseLevel3D(JSON.parse(JSON.stringify(d))).terrain, d.terrain);
});
test("inverted, collapsed and invalid vertex references are rejected", () => {
  const g = createTerrainGrid([0, 0, 100, 100]);
  g.vertices[1]!.position = [-20, 0, 0];
  assert.throws(() => validateTerrainGrid(g), /invert/);
  g.vertices[1]!.position = [100, 0, NaN];
  assert.throws(() => validateTerrainGrid(g), /finite/);
});
test("selected subdivision preserves slopes and shares refined edges with neighbors", () => {
  const g = createTerrainGrid([0, 0, 200, 100], 100);
  g.vertices[1]!.position[2] = 40;
  g.vertices[4]!.position[2] = 20;
  const next = subdivideTerrainCells(g, [g.cells[0]!.id]);
  assert.ok(next.cells.length > g.cells.length);
  for (let y = 1; y < 100; y += 7)
    for (let x = 1; x < 200; x += 9)
      assert.ok(
        Math.abs(
          terrainHeightAt({ terrain: g }, x, y)! - terrainHeightAt({ terrain: next }, x, y)!,
        ) < 1e-8,
      );
  const edgeMid = next.vertices.findIndex((v) => v.position[0] === 100 && v.position[1] === 50);
  assert.ok(edgeMid >= 0);
  assert.ok(next.cells.filter((c) => c.vertices.includes(edgeMid)).length >= 4);
});
test("workspace expansion retains exact original geometry and shrinking deletes nothing", () => {
  const g = createTerrainGrid([0, 0, 100, 100], 100);
  g.vertices[1]!.position[2] = 20;
  const bigger = expandTerrainGrid(g, [0, 0, 200, 200]);
  assert.deepEqual(bigger.vertices.slice(0, g.vertices.length), g.vertices);
  assert.deepEqual(bigger.cells.slice(0, g.cells.length), g.cells);
  assert.notEqual(terrainHeightAt({ terrain: bigger }, 150, 150), undefined);
  assert.deepEqual(expandTerrainGrid(bigger, [0, 0, 50, 50]), bigger);
});
test("sloped adjacent grid triangles export as one navigation region", () => {
  const d = terrainFixture();
  d.terrain!.vertices.forEach((v) => (v.position[2] = (v.position[0] - 100) * 0.1));
  const result = compileAssetGameplay(d, new Map(), [0, 0, 1000, 1000]);
  assert.equal(result.motion_data.layers.flat().length, 1);
  assert.ok(result.sight_obstacles.some((o) => o.default_material === 3));
  assert.equal(terrainTriangles(d).length, 60);
});
test("cell walkability cuts traversal without deleting ground", () => {
  const d = terrainFixture();
  d.terrain!.cells[0]!.walkable = false;
  const result = compileAssetGameplay(d, new Map(), [0, 0, 1000, 1000]);
  assert.ok(result.motion_data.layers.flat().length > 0);
  assert.ok(result.sight_obstacles.length > 0);
});
test("river receivers block water while a ford remains part of terrain navigation", () => {
  const d = terrainFixture();
  d.splines = [
    {
      id: "river",
      name: "River",
      kind: "river",
      points: [
        [400, 100, 0],
        [400, 600, 0],
      ],
      width: 60,
      repeatLength: 100,
      closed: false,
      pointMaterials: ["water_still", "water_still"],
    },
  ];
  const river = compileAssetGameplay(d, new Map(), [0, 0, 1000, 1000]);
  assert.ok(river.material_sectors?.some((o) => o.material === 5));
  d.splines = [{ ...d.splines[0]!, pointMaterials: ["water_ford", "water_ford"] }];
  const ford = compileAssetGameplay(d, new Map(), [0, 0, 1000, 1000]);
  assert.ok(ford.motion_data.layers.flat().length > 0);
  assert.ok(ford.sight_obstacles.some((o) => o.default_material === 5));
});
test("refinement retains the original edge diagonal surface for nonplanar cells", () => {
  const g = createTerrainGrid([0, 0, 100, 100], 100);
  g.vertices[3]!.position[2] = 50;
  const finer = subdivideTerrainCells(g, [g.cells[0]!.id]);
  for (const [x, y] of [
    [20, 30],
    [60, 10],
    [10, 60],
    [80, 80],
  ])
    assert.ok(
      Math.abs(
        terrainHeightAt({ terrain: g }, x!, y!)! - terrainHeightAt({ terrain: finer }, x!, y!)!,
      ) < 1e-8,
    );
});
test("dominant terrain gameplay material is independent of triangle vertex order and honors custom mapping", () => {
  const d = terrainFixture();
  d.terrain = createTerrainGrid([100, 100, 100, 100], 100);
  d.customMaterials = [
    {
      id: "custom_brick",
      name: "Brick",
      color: "#a04030",
      gameplayMaterial: 2,
      textureBase: "paved",
    },
  ];
  d.terrain.cells = [{ id: "triangle", vertices: [0, 1, 3], material: "grass_short" }];
  d.terrain.vertices[0]!.material = "custom_brick";
  d.terrain.vertices[1]!.material = "grass_short";
  d.terrain.vertices[3]!.material = "custom_brick";
  const first = compileAssetGameplay(d, new Map(), [0, 0, 400, 400]);
  d.terrain = { ...d.terrain, cells: [{ ...d.terrain.cells[0]!, vertices: [1, 3, 0] }] };
  const reordered = compileAssetGameplay(d, new Map(), [0, 0, 400, 400]);
  assert.deepEqual(
    first.sight_obstacles.map((o) => o.default_material),
    reordered.sight_obstacles.map((o) => o.default_material),
  );
  assert.ok(first.sight_obstacles.some((o) => o.default_material === 2));
  assert.deepEqual(parseLevel3D(JSON.parse(JSON.stringify(d))).customMaterials, d.customMaterials);
});
test("subdivision preserves painted material mixtures throughout the original triangles", () => {
  const grid = createTerrainGrid([0, 0, 100, 100], 100);
  grid.vertices[0]!.material = "grass_short";
  grid.vertices[1]!.material = "path_dirt";
  grid.vertices[2]!.material = "ground_sand";
  grid.vertices[3]!.material = "road_cobblestone";
  const refined = subdivideTerrainCells(grid, [grid.cells[0]!.id]);
  const sample = (g: typeof grid, x: number, y: number) => {
    for (const t of terrainTriangles({ terrain: g })) {
      const [a, b, c] = t.points,
        den = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
      const v = ((x - a[0]) * (c[1] - a[1]) - (y - a[1]) * (c[0] - a[0])) / den,
        w = ((b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0])) / den,
        u = 1 - v - w;
      if (Math.min(u, v, w) < -1e-7) continue;
      const weights: Record<string, number> = {};
      t.materialMixes!.forEach((mix, i) =>
        Object.entries(mix).forEach(([id, weight]) => {
          weights[id] = (weights[id] ?? 0) + weight * [u, v, w][i]!;
        }),
      );
      return weights;
    }
    throw Error("outside");
  };
  for (let x = 5; x < 100; x += 15)
    for (let y = 5; y < 100; y += 15) {
      const a = sample(grid, x, y),
        b = sample(refined, x, y);
      for (const id of new Set([...Object.keys(a), ...Object.keys(b)]))
        assert.ok(Math.abs((a[id] ?? 0) - (b[id] ?? 0)) < 1e-7);
    }
  assert.ok(refined.vertices.some((v) => Object.keys(v.materialMix ?? {}).length === 2));
});
test("subdividing cell-only materials preserves non-grass fallback at every new vertex", () => {
  const grid = createTerrainGrid([0, 0, 100, 100], 100, 0, "ground_rocky");
  for (const vertex of grid.vertices) delete vertex.material;
  const refined = subdivideTerrainCells(grid, [grid.cells[0]!.id]);
  for (const triangle of terrainTriangles({ terrain: refined }))
    for (const mix of triangle.materialMixes!) assert.deepEqual(mix, { ground_rocky: 1 });
});

test("local subdivision retains distant and corner-only neighbors exactly", () => {
  const grid = createTerrainGrid([0, 0, 400, 300], 100);
  grid.cells[5]!.diagonal = 1;
  grid.vertices.forEach((vertex, i) => {
    vertex.position[2] = (i * 13) % 37;
    vertex.uv = [vertex.position[0] / 400, vertex.position[1] / 300];
  });
  const before = structuredClone(grid);
  const refined = subdivideTerrainCells(grid, [grid.cells[5]!.id]);
  const affected = new Set([1, 4, 5, 6, 9]);
  for (const [index, cell] of grid.cells.entries()) {
    if (affected.has(index)) {
      assert.ok(!refined.cells.some((next) => next.id === cell.id));
      continue;
    }
    assert.strictEqual(
      refined.cells.find((next) => next.id === cell.id),
      cell,
    );
    assert.equal(cell.vertices.length, 4);
  }
  assert.equal(
    refined.cells.filter((cell) => cell.id.startsWith(`${grid.cells[5]!.id}/`)).length,
    8,
  );
  for (const [index, vertex] of grid.vertices.entries())
    assert.strictEqual(refined.vertices[index], vertex);
  for (const vertex of refined.vertices) {
    assert.ok(Math.abs(vertex.uv![0] - vertex.position[0] / 400) < 1e-8);
    assert.ok(Math.abs(vertex.uv![1] - vertex.position[1] / 300) < 1e-8);
  }
  for (let y = 7; y < 300; y += 19)
    for (let x = 9; x < 400; x += 23)
      assert.ok(
        Math.abs(
          terrainHeightAt({ terrain: grid }, x, y)! - terrainHeightAt({ terrain: refined }, x, y)!,
        ) < 1e-8,
      );
  assert.deepEqual(grid, before);
});

test("empty subdivision is a no-op and unknown cell selections fail explicitly", () => {
  const grid = createTerrainGrid([0, 0, 200, 200], 100);
  assert.strictEqual(subdivideTerrainCells(grid, []), grid);
  const before = structuredClone(grid);
  assert.throws(
    () => subdivideTerrainCells(grid, [grid.cells[0]!.id, "missing"]),
    /unknown terrain cell missing/,
  );
  assert.deepEqual(grid, before);
});

test("anisotropic map-pixel row spacing survives saves and workspace growth", () => {
  const terrain = createTerrainGrid([0, 0, 200, 100], 100, 0, "grass_short", 50);
  const document = parseLevel3D(JSON.parse(JSON.stringify({ ...terrainFixture(), terrain })));
  assert.equal(document.terrain!.rowSpacing, 50);
  const grown = expandTerrainGrid(document.terrain!, [0, 0, 400, 200]);
  assert.equal(grown.rowSpacing, 50);
  validateTerrainGrid(grown);
  for (const vertex of grown.vertices) {
    assert.equal(vertex.position[0] % 100, 0);
    assert.equal(vertex.position[1] % 50, 0);
  }
  for (const rowSpacing of [0, -1, NaN, Infinity]) {
    assert.throws(() => createTerrainGrid([0, 0, 100, 100], 100, 0, "grass_short", rowSpacing));
    assert.throws(() => validateTerrainGrid({ ...terrain, rowSpacing }));
  }
});

test("road-only edits reuse evaluated terrain while river edits invalidate it", () => {
  const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
  const river = {
    id: "river",
    kind: "river" as const,
    width: 40,
    repeatLength: 100,
    closed: false,
    points: [
      [0, 0, 0],
      [100, 100, 0],
    ] as [number, number, number][],
  };
  const road = { ...river, id: "road", kind: "road" as const };
  const document = {
    camera,
    terrain: createTerrainGrid([0, 0, 200, 200], 100),
    splines: [river, road],
  };
  const before = terrainTriangles(document);
  const changed = { ...document, splines: [river, { ...road, width: 80 }] };
  assert.equal(terrainTriangles(changed), before);
  const preview = { ...changed, splines: [{ ...river, width: 60 }, road] };
  const after = terrainTriangles(preview);
  assert.notEqual(after, before);
  for (let i = 0; i < 10; i++) {
    assert.equal(terrainTriangles(document), before);
    assert.equal(terrainTriangles(preview), after);
  }
});
