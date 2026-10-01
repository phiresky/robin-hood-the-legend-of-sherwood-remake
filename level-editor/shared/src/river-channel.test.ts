import test from "node:test";
import assert from "node:assert/strict";
import {
  createTerrainGrid,
  terrainTriangles,
  terrainHeightAt,
  triangleHeightAt,
} from "./authored-terrain.ts";
import { evaluateRiverChannels } from "./river-channel.ts";
import type { LevelSpline } from "./splines.ts";
import type { TerrainTriangle } from "./authored-terrain.ts";
const camera = { kind: "oblique-orthographic" as const, elevation_deg: 35 };
const river: LevelSpline = {
  id: "river",
  name: "River",
  kind: "river",
  points: [
    [20, 50, 0],
    [180, 50, 0],
  ],
  width: 12,
  closed: false,
  repeatLength: 32,
  channel: { enabled: true, bedDepth: 8, bankSlope: 1 },
};
const grid = createTerrainGrid([0, 0, 200, 100], 200);
const base = terrainTriangles({ terrain: grid });
const at = (triangles: TerrainTriangle[], x: number, y: number) =>
  triangles.map((t) => triangleHeightAt(t.points, x, y)).find((h) => h !== undefined);
const area = (triangles: TerrainTriangle[]) =>
  triangles.reduce(
    (sum, { points: [a, b, c] }) =>
      sum + Math.abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2,
    0,
  );
test("narrow river cuts a coarse cell without modifying its control mesh", () => {
  const saved = JSON.stringify(grid),
    result = evaluateRiverChannels(base, { camera, splines: [river] });
  assert.equal(JSON.stringify(grid), saved);
  assert.ok(result.length > base.length);
  assert.ok(Math.abs(at(result, 100, 50)! + 8) < 1e-6);
  assert.equal(at(result, 100, 10), 0);
  assert.ok(Math.abs(area(result) - 20000) < 1e-5);
  for (const t of result) {
    assert.ok(t.points.flat().every(Number.isFinite));
    const [a, b, c] = t.points;
    assert.ok((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) > 0);
  }
});
test("moving disabling and deleting a river restore the original ground", () => {
  assert.strictEqual(evaluateRiverChannels(base, { camera, splines: [] }), base);
  assert.strictEqual(
    evaluateRiverChannels(base, {
      camera,
      splines: [{ ...river, channel: { ...river.channel!, enabled: false } }],
    }),
    base,
  );
  const moved = {
    ...river,
    points: river.points.map(([x, y, z]) => [x, y + 30, z] as [number, number, number]),
  };
  assert.equal(at(evaluateRiverChannels(base, { camera, splines: [moved] }), 100, 50), 0);
  assert.ok(
    Math.abs(at(evaluateRiverChannels(base, { camera, splines: [moved] }), 100, 80)! + 8) < 1e-6,
  );
});
test("crossing channels use the lower bed independently of river ordering", () => {
  const second: LevelSpline = {
    ...river,
    id: "second",
    points: [
      [100, 10, -3],
      [100, 90, -3],
    ],
  };
  const a = evaluateRiverChannels(base, { camera, splines: [river, second] }),
    b = evaluateRiverChannels(base, { camera, splines: [second, river] });
  for (const [x, y] of [
    [100, 50],
    [100, 48],
    [80, 50],
    [100, 30],
    [150, 10],
  ])
    assert.ok(Math.abs(at(a, x!, y!)! - at(b, x!, y!)!) < 1e-6);
  assert.ok(Math.abs(at(a, 100, 50)! + 11) < 1e-6);
  assert.ok(Math.abs(area(a) - 20000) < 1e-5);
});

test("bank contours preserve sloped ground and interpolate painted UVs", () => {
  const slope = base.map((t) => ({
    ...t,
    points: t.points.map(
      ([x, y]) => [x, y, x / 10] as [number, number, number],
    ) as TerrainTriangle["points"],
    uv: t.points.map(([x, y]) => [x / 200, y / 100] as [number, number]) as [
      [number, number],
      [number, number],
      [number, number],
    ],
  }));
  const result = evaluateRiverChannels(slope, { camera, splines: [river] });
  assert.ok(Math.abs(at(result, 100, 10)! - 10) < 1e-6);
  assert.ok(Math.abs(at(result, 100, 50)! + 8) < 1e-6);
  for (const triangle of result)
    triangle.points.forEach(([x, y], i) => {
      assert.ok(Math.abs(triangle.uv![i]![0] - x / 200) < 1e-6);
      assert.ok(Math.abs(triangle.uv![i]![1] - y / 100) < 1e-6);
    });
  assert.ok(Math.abs(area(result) - 20000) < 1e-5);
});

test("curved variable-width channels retain continuous ground coverage", () => {
  const curved: LevelSpline = {
    ...river,
    points: [
      [20, 30, 0],
      [100, 65, -3],
      [180, 30, 0],
    ],
    pointWidths: [10, 30, 10],
  };
  const result = evaluateRiverChannels(base, { camera, splines: [curved] });
  assert.ok(Math.abs(area(result) - 20000) < 1e-4);
  for (let x = 1; x < 200; x += 13)
    for (let y = 1; y < 100; y += 9) {
      const values = result
        .map((t) => triangleHeightAt(t.points, x, y))
        .filter((z) => z !== undefined);
      assert.ok(values.length > 0, `gap at ${x},${y}`);
      assert.ok(Math.max(...values) - Math.min(...values) < 1e-5, `overlap at ${x},${y}`);
    }
});

test("shared terrain sampling evaluates channels and invalidates on river changes", () => {
  const document = { terrain: grid, camera, splines: [river] };
  assert.ok(Math.abs(terrainHeightAt(document, 100, 50)! + 8) < 1e-6);
  assert.strictEqual(terrainTriangles(document), terrainTriangles(document));
  assert.equal(terrainHeightAt({ ...document, splines: [] }, 100, 50), 0);
  const changed = { ...river, channel: { ...river.channel!, bedDepth: 12 } };
  assert.ok(Math.abs(terrainHeightAt({ ...document, splines: [changed] }, 100, 50)! + 12) < 1e-6);
});

test("channel splits preserve material blends including carried weights", () => {
  const materials: [string, string, string] = ["grass_short", "ground_sand", "ground_moss"];
  const weighted = base.map((t) => ({
    ...t,
    materials,
    materialWeights: t.points.map(([x, y]) => [
      x / 400,
      y / 200,
      1 - x / 400 - y / 200,
    ]) as TerrainTriangle["materialWeights"],
  }));
  const result = evaluateRiverChannels(weighted, { camera, splines: [river] });
  for (const t of result)
    t.points.forEach(([x, y], i) => {
      assert.deepEqual(t.materials, materials);
      assert.ok(Math.abs(t.materialWeights![i]![0] - x / 400) < 1e-7);
      assert.ok(Math.abs(t.materialWeights![i]![1] - y / 200) < 1e-7);
      assert.ok(Math.abs(t.materialWeights![i]![2] - (1 - x / 400 - y / 200)) < 1e-7);
    });
});

test("long straight channels avoid redundant sampled section boundaries", () => {
  const longBase = terrainTriangles({ terrain: createTerrainGrid([0, 0, 2000, 1000], 250) });
  const longRiver: LevelSpline = {
    ...river,
    points: [
      [100, 500, 0],
      [1900, 500, 0],
    ],
    width: 100,
  };
  const result = evaluateRiverChannels(longBase, { camera, splines: [longRiver] });
  assert.ok(result.length < 1000, `Unexpected straight channel fragmentation: ${result.length}`);
  assert.ok(Math.abs(area(result) - 2000000) < 1e-4);
});

test("ford channel has a shallow bed and traversable banks while other reaches retain depth", () => {
  const ford: LevelSpline = {
    ...river,
    points: [
      [100, 0, 0],
      [100, 30, 0],
      [100, 70, 0],
      [100, 100, 0],
    ],
    width: 24,
    channel: { enabled: true, bedDepth: 24, bankSlope: 1 },
    pointMaterials: ["water_still", "water_ford", "water_ford", "water_still"],
  };
  const result = evaluateRiverChannels(base, { camera, splines: [ford] });
  const cos = Math.cos((camera.elevation_deg * Math.PI) / 180);
  assert.ok(Math.abs(at(result, 100, 50)! + 12 * cos * 0.75 * 0.5) < 1e-6);
  for (let x = 75; x < 125; x += 0.5) {
    const a = at(result, x, 50)!,
      b = at(result, x + 0.5, 50)!;
    assert.ok(Math.abs(b - a) / cos / 0.5 <= 0.750001, `Steep ford bank at ${x}`);
  }
  assert.ok(at(result, 100, 5)! < -12, "ordinary river reaches stay deeper");
});

test("local edits reuse unchanged channel triangles and match a fresh evaluation", () => {
  const grid = createTerrainGrid([0, 0, 200, 100], 25);
  const splines = [river];
  const original = terrainTriangles({ terrain: grid });
  const first = evaluateRiverChannels(original, { camera, splines });
  const moved = {
    ...grid,
    vertices: grid.vertices.map((v) =>
      v.position[0] === 100 && v.position[1] === 50
        ? { ...v, position: [100, 50, -4] as [number, number, number] }
        : v,
    ),
  };
  const nextBase = terrainTriangles({ terrain: moved });
  const next = evaluateRiverChannels(nextBase, { camera, splines });
  assert.ok(
    next.some((t) => first.includes(t)),
    "Unchanged channel tessellation is reused",
  );
  assert.deepEqual(next, evaluateRiverChannels(nextBase, { camera, splines: [...splines] }));
  const taller = nextBase.map((t) => ({
    ...t,
    points: t.points.map(
      ([x, y, z]) => [x, y, z + 100] as [number, number, number],
    ) as TerrainTriangle["points"],
  }));
  assert.deepEqual(
    evaluateRiverChannels(taller, { camera, splines }),
    evaluateRiverChannels(taller, { camera, splines: [...splines] }),
  );
  const repainted = nextBase.map((t) => ({
    ...t,
    materials: ["path_dirt", "path_dirt", "path_dirt"] as [string, string, string],
  }));
  assert.deepEqual(
    evaluateRiverChannels(repainted, { camera, splines }),
    evaluateRiverChannels(repainted, { camera, splines: [...splines] }),
  );
});
