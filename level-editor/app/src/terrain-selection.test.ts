import test from "node:test";
import assert from "node:assert/strict";
import { createTerrainGrid, cellTriangles, validateTerrainGrid } from "@rle/shared";
import { alignTerrainDiagonals, flattenTerrainVertices } from "./terrain-selection.ts";

test("flatten averages only selected heights and preserves their XY and all unselected data", () => {
  const grid = createTerrainGrid([0, 0, 200, 100], 100, 90);
  grid.vertices[0]!.position[2] = -20;
  grid.vertices[1]!.position[2] = 10;
  grid.vertices[3]!.position[2] = 70;
  const ids = [0, 1, 3].map((index) => grid.vertices[index]!.id);
  const flattened = flattenTerrainVertices(grid, [...ids, ids[0]!]);
  validateTerrainGrid(flattened);
  for (let i = 0; i < grid.vertices.length; i++) {
    const before = grid.vertices[i]!,
      after = flattened.vertices[i]!;
    if (ids.includes(before.id)) {
      assert.deepEqual(after.position, [before.position[0], before.position[1], 20]);
      assert.equal(after.material, before.material);
    } else assert.strictEqual(after, before);
  }
  assert.deepEqual(
    [grid.vertices[0]!.position[2], grid.vertices[1]!.position[2], grid.vertices[3]!.position[2]],
    [-20, 10, 70],
  );
});

test("flatten empty and single selections is unchanged and invalid selections fail explicitly", () => {
  const grid = createTerrainGrid([0, 0, 100, 100], 100);
  assert.strictEqual(flattenTerrainVertices(grid, []), grid);
  assert.strictEqual(flattenTerrainVertices(grid, [grid.vertices[0]!.id]), grid);
  assert.throws(() => flattenTerrainVertices(grid, ["missing"]), /unknown terrain vertex/);
});

test("selection boundary diagonals cut across each isolated corner and its complement", () => {
  const grid = createTerrainGrid([0, 0, 100, 100], 100);
  const cell = grid.cells[0]!;
  for (let corner = 0; corner < 4; corner++) {
    for (const complement of [false, true]) {
      const selected = cell.vertices
        .filter((_, i) => (i === corner) !== complement)
        .map((i) => grid.vertices[i]!.id);
      const next = alignTerrainDiagonals(grid, selected);
      validateTerrainGrid(next);
      const triangles = cellTriangles(next, next.cells[0]!);
      const diagonal = triangles[0].filter((i) => triangles[1].includes(i));
      assert.ok(!diagonal.includes(cell.vertices[corner]!));
      assert.deepEqual(
        new Set(diagonal),
        new Set([cell.vertices[(corner + 1) % 4], cell.vertices[(corner + 3) % 4]]),
      );
    }
  }
  assert.strictEqual(alignTerrainDiagonals(grid, []), grid);
  assert.strictEqual(
    alignTerrainDiagonals(
      grid,
      grid.vertices.map((v) => v.id),
    ),
    grid,
  );
  assert.strictEqual(
    alignTerrainDiagonals(
      grid,
      cell.vertices.slice(0, 2).map((i) => grid.vertices[i]!.id),
    ),
    grid,
  );
});

test("concave cells retain the only valid diagonal", () => {
  const grid = createTerrainGrid([0, 0, 100, 100], 100);
  const cell = grid.cells[0]!;
  grid.vertices[cell.vertices[2]!]!.position = [20, 20, 0];
  validateTerrainGrid(grid);
  assert.strictEqual(alignTerrainDiagonals(grid, [grid.vertices[cell.vertices[0]!]!.id]), grid);
});
