import { cellTriangles, type TerrainGrid } from "@rle/shared";

/** Cut across the isolated corner at a selection boundary, preserving valid XY topology. */
export function alignTerrainDiagonals(grid: TerrainGrid, ids: readonly string[]): TerrainGrid {
  const selected = new Set(ids);
  let changed = false;
  const cells = grid.cells.map((cell) => {
    if (cell.vertices.length !== 4) return cell;
    const mask = cell.vertices.map((i) => selected.has(grid.vertices[i]!.id));
    const count = mask.filter(Boolean).length;
    if (count !== 1 && count !== 3) return cell;
    const corner = mask.indexOf(count === 1);
    const diagonal: 0 | 1 = corner % 2 === 0 ? 1 : 0;
    if ((cell.diagonal ?? 0) === diagonal) return cell;
    const candidate = { ...cell, diagonal };
    // A deformed concave quad may only admit its existing diagonal.
    if (
      cellTriangles(grid, candidate).some((indices) => {
        const [a, b, c] = indices.map((i) => grid.vertices[i]!.position);
        return (b![0] - a![0]) * (c![1] - a![1]) - (b![1] - a![1]) * (c![0] - a![0]) <= 1e-5;
      })
    )
      return cell;
    changed = true;
    return candidate;
  });
  return changed ? { ...grid, cells } : grid;
}

/** Flatten one selection without changing its footprint or the surrounding ground. */
export function flattenTerrainVertices(grid: TerrainGrid, ids: readonly string[]): TerrainGrid {
  const selected = new Set(ids);
  const vertices = grid.vertices.filter((vertex) => selected.has(vertex.id));
  if (vertices.length !== selected.size)
    throw new Error("Selection contains an unknown terrain vertex");
  if (vertices.length < 2) return grid;
  const height = vertices.reduce((sum, vertex) => sum + vertex.position[2] / vertices.length, 0);
  const next: TerrainGrid = {
    ...grid,
    vertices: grid.vertices.map((vertex) =>
      selected.has(vertex.id)
        ? { ...vertex, position: [vertex.position[0], vertex.position[1], height] }
        : vertex,
    ),
  };
  return vertices.some((v) => v.position[2] !== height) ? alignTerrainDiagonals(next, ids) : next;
}
