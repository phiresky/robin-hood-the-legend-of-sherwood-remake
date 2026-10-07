import earcut, { flatten } from "earcut";
import clipping, { type MultiPolygon, type Polygon } from "polygon-clipping";
import { fixedPolygonBoolean } from "./fixed-polygon-boolean.ts";
import { Vector3 } from "three";
import { gameToScene, type MapCamera, type Vec3 } from "./scene.ts";
import type { Level3D } from "./level3d.ts";
import type { LevelSpline } from "./splines.ts";
import type { Point } from "./level.ts";
import type { GameplayAssetDescriptor, AssetWalkableSurface } from "./asset-gameplay.ts";
import { evaluateRiverChannels, riverChannelSource } from "./river-channel.ts";
import { sampleSpline, splineCurve, splineMaterialWeightsAt } from "./spline-sampling.ts";
import { roadTerrainPieces } from "./terrain-path-gameplay.ts";
import { terrainMaterial } from "./terrain-materials.ts";

export interface TerrainVertex {
  id: string;
  position: Vec3;
  material?: string;
  materialMix?: Record<string, number>;
  uv?: Point;
}
export interface TerrainCell {
  id: string;
  vertices: number[];
  material: string;
  walkable?: boolean;
  diagonal?: 0 | 1;
}
export interface TerrainGrid {
  version: 1;
  spacing: number;
  /** Row spacing in map pixels; omitted grids use spacing on both axes. */
  rowSpacing?: number;
  vertices: TerrainVertex[];
  cells: TerrainCell[];
  texture?: string;
}
export interface TerrainTriangle {
  id: string;
  cell: TerrainCell;
  indices: [number, number, number];
  points: [Vec3, Vec3, Vec3];
  uv?: [Point, Point, Point];
  materials?: [string, string, string];
  materialMixes?: [Record<string, number>, Record<string, number>, Record<string, number>];
  materialWeights?: [Vec3, Vec3, Vec3];
}
const cross = (a: Vec3, b: Vec3, c: Vec3) =>
  (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
export function cellTriangles(grid: TerrainGrid, cell: TerrainCell): [number, number, number][] {
  const v = cell.vertices;
  if (v.length === 3) return [[v[0]!, v[1]!, v[2]!]];
  if (v.length === 4)
    return cell.diagonal === 1
      ? [
          [v[0]!, v[1]!, v[3]!],
          [v[1]!, v[2]!, v[3]!],
        ]
      : [
          [v[0]!, v[1]!, v[2]!],
          [v[0]!, v[2]!, v[3]!],
        ];
  const flat = v.flatMap((i) => grid.vertices[i]!.position.slice(0, 2));
  const indices = earcut(flat);
  const out: [number, number, number][] = [];
  for (let i = 0; i < indices.length; i += 3)
    out.push([v[indices[i]!]!, v[indices[i + 1]!]!, v[indices[i + 2]!]!]);
  return out;
}
export function validateTerrainGrid(value: unknown): asserts value is TerrainGrid {
  const g = value as TerrainGrid;
  if (
    !g ||
    g.version !== 1 ||
    !Number.isFinite(g.spacing) ||
    (g.rowSpacing !== undefined && (!Number.isFinite(g.rowSpacing) || g.rowSpacing <= 0)) ||
    g.spacing <= 0 ||
    !Array.isArray(g.vertices) ||
    !Array.isArray(g.cells)
  )
    throw new Error("Terrain needs a valid connected grid");
  if (g.texture !== undefined && (typeof g.texture !== "string" || !g.texture))
    throw new Error("Terrain texture must be a nonempty URL");
  const ids = new Set<string>();
  for (const v of g.vertices) {
    if (
      !v ||
      typeof v.id !== "string" ||
      !v.id ||
      ids.has(v.id) ||
      !Array.isArray(v.position) ||
      v.position.length !== 3 ||
      !v.position.every(Number.isFinite)
    )
      throw new Error("Terrain vertices need unique IDs and finite XYZ positions");
    ids.add(v.id);
    if (v.material !== undefined && (typeof v.material !== "string" || !v.material))
      throw new Error("Terrain vertex material must be a nonempty ID");
    if (v.materialMix !== undefined) {
      if (!v.materialMix || typeof v.materialMix !== "object" || Array.isArray(v.materialMix))
        throw new Error("Invalid terrain material mixture");
      const values = Object.values(v.materialMix);
      if (
        !values.length ||
        Object.keys(v.materialMix).some((id) => !id) ||
        values.some((w) => !Number.isFinite(w) || w < 0) ||
        Math.abs(values.reduce((a, b) => a + b, 0) - 1) > 1e-6
      )
        throw new Error("Terrain material mixtures need normalized nonnegative weights");
    }
    if (v.uv && (v.uv.length !== 2 || !v.uv.every(Number.isFinite)))
      throw new Error("Terrain UV coordinates must be finite");
  }
  ids.clear();
  for (const c of g.cells) {
    if (
      !c ||
      typeof c.id !== "string" ||
      !c.id ||
      ids.has(c.id) ||
      typeof c.material !== "string" ||
      !c.material ||
      !Array.isArray(c.vertices) ||
      c.vertices.length < 3 ||
      new Set(c.vertices).size !== c.vertices.length ||
      c.vertices.some((i) => !Number.isInteger(i) || i < 0 || i >= g.vertices.length) ||
      (c.walkable !== undefined && typeof c.walkable !== "boolean") ||
      (c.diagonal !== undefined && c.diagonal !== 0 && c.diagonal !== 1)
    )
      throw new Error("Terrain cells need valid vertex indices and materials");
    ids.add(c.id);
    for (const t of cellTriangles(g, c))
      if (cross(...(t.map((i) => g.vertices[i]!.position) as [Vec3, Vec3, Vec3])) <= 1e-5)
        throw new Error("Terrain vertices cannot invert or collapse a cell");
  }
  const edges = new Map<string, { a: number; b: number; count: number }>();
  for (const cell of g.cells)
    for (let i = 0; i < cell.vertices.length; i++) {
      const a = cell.vertices[i]!,
        b = cell.vertices[(i + 1) % cell.vertices.length]!,
        key = a < b ? `${a}/${b}` : `${b}/${a}`,
        edge = edges.get(key);
      if (edge) edge.count++;
      else edges.set(key, { a, b, count: 1 });
    }
  const bins = new Map<string, { a: number; b: number }[]>(),
    spacing = Math.min(64, Math.max(32, g.spacing));
  for (const edge of edges.values())
    if (edge.count === 1) {
      const a = g.vertices[edge.a]!.position,
        b = g.vertices[edge.b]!.position,
        seen = new Set<object>();
      for (
        let x = Math.floor(Math.min(a[0], b[0]) / spacing);
        x <= Math.floor(Math.max(a[0], b[0]) / spacing);
        x++
      )
        for (
          let y = Math.floor(Math.min(a[1], b[1]) / spacing);
          y <= Math.floor(Math.max(a[1], b[1]) / spacing);
          y++
        ) {
          const key = `${x}/${y}`,
            items = bins.get(key) ?? [];
          for (const other of items) {
            if (seen.has(other)) continue;
            seen.add(other);
            if (
              edge.a === other.a ||
              edge.a === other.b ||
              edge.b === other.a ||
              edge.b === other.b
            )
              continue;
            const c = g.vertices[other.a]!.position,
              d = g.vertices[other.b]!.position;
            if (cross(a, b, c) * cross(a, b, d) < -1e-8 && cross(c, d, a) * cross(c, d, b) < -1e-8)
              throw new Error("Terrain boundary edges cannot cross");
          }
          items.push(edge);
          bins.set(key, items);
        }
    }
}
export function createTerrainGrid(
  bounds: [number, number, number, number],
  spacing = 128,
  height = 0,
  material = "grass_short",
  rowSpacing = spacing,
): TerrainGrid {
  const [x, y, w, h] = bounds;
  if (
    !bounds.every(Number.isFinite) ||
    w <= 0 ||
    h <= 0 ||
    !Number.isFinite(spacing) ||
    spacing <= 0 ||
    !Number.isFinite(height)
  )
    throw new Error("Terrain bounds and spacing must be positive and finite");
  if (!Number.isFinite(rowSpacing) || rowSpacing <= 0)
    throw new Error("Terrain row spacing must be positive and finite");
  const columns = Math.ceil(w / spacing),
    rows = Math.ceil(h / rowSpacing);
  if ((columns + 1) * (rows + 1) > 1000000)
    throw new Error("Terrain grid is too dense; increase spacing");
  const vertices: TerrainVertex[] = [],
    cells: TerrainCell[] = [];
  for (let r = 0; r <= rows; r++)
    for (let c = 0; c <= columns; c++)
      vertices.push({
        id: `v-${r}-${c}`,
        position: [x + Math.min(w, c * spacing), y + Math.min(h, r * rowSpacing), height],
        material,
      });
  for (let r = 0; r < rows; r++)
    for (let c = 0; c < columns; c++) {
      const i = r * (columns + 1) + c;
      cells.push({
        id: `cell-${r}-${c}`,
        vertices: [i, i + 1, i + columns + 2, i + columns + 1],
        material,
      });
    }
  return {
    version: 1,
    spacing,
    ...(rowSpacing !== spacing ? { rowSpacing } : {}),
    vertices,
    cells,
  };
}
type TerrainDocument = Pick<Level3D, "terrain"> & Partial<Pick<Level3D, "splines" | "camera">>;
const terrainCache = new WeakMap<
  TerrainGrid,
  {
    splines: Level3D["splines"];
    camera: Level3D["camera"] | undefined;
    triangles: TerrainTriangle[];
    buckets: Map<string, TerrainTriangle[]>;
    base: TerrainIndex;
    replacements: Map<TerrainTriangle, TerrainTriangle[]>;
  }[]
>();
const cellTriangleCache = new WeakMap<
  TerrainCell,
  {
    vertices: TerrainVertex[];
    textured: boolean;
    triangles: TerrainTriangle[];
  }
>();
type TerrainIndex = {
  triangles: TerrainTriangle[];
  buckets: Map<string, TerrainTriangle[]>;
  spacing: number;
};
const baseTerrainCache = new WeakMap<TerrainGrid, TerrainIndex>();
function baseTerrain(g: TerrainGrid) {
  const cached = baseTerrainCache.get(g);
  if (cached) return cached;
  const triangles = g.cells.flatMap((cell) => {
    const vertices = cell.vertices.map((i) => g.vertices[i]!);
    const cached = cellTriangleCache.get(cell);
    if (
      cached &&
      cached.textured === !!g.texture &&
      cached.vertices.length === vertices.length &&
      vertices.every((v, i) => v === cached.vertices[i])
    )
      return cached.triangles;
    const triangles = cellTriangles(g, cell).map((indices, i): TerrainTriangle => ({
      id: `${cell.id}/${i}`,
      cell,
      indices,
      points: indices.map((j) => g.vertices[j]!.position) as [Vec3, Vec3, Vec3],
      materialMixes: indices.map((j) =>
        terrainVertexMaterialMix(g, g.vertices[j]!, cell.material),
      ) as [Record<string, number>, Record<string, number>, Record<string, number>],
      materials: indices.map(
        (j) => g.vertices[j]!.material ?? (g.texture ? "$source" : cell.material),
      ) as [string, string, string],
      ...(indices.every((j) => g.vertices[j]!.uv)
        ? { uv: indices.map((j) => g.vertices[j]!.uv!) as [Point, Point, Point] }
        : {}),
    }));
    cellTriangleCache.set(cell, { vertices, textured: !!g.texture, triangles });
    return triangles;
  });
  const index = {
    triangles,
    spacing: Math.min(64, Math.max(32, g.spacing)),
    buckets: new Map<string, TerrainTriangle[]>(),
  };
  for (const triangle of triangles) {
    const b = triangleBounds(triangle);
    for (let x = Math.floor(b[0] / index.spacing); x <= Math.floor(b[2] / index.spacing); x++)
      for (let y = Math.floor(b[1] / index.spacing); y <= Math.floor(b[3] / index.spacing); y++) {
        const key = `${x}/${y}`;
        const bucket = index.buckets.get(key);
        if (bucket) bucket.push(triangle);
        else index.buckets.set(key, [triangle]);
      }
  }
  baseTerrainCache.set(g, index);
  return index;
}
function evaluatedTerrain(document: TerrainDocument) {
  const g = document.terrain;
  if (!g) return undefined;
  const recent = terrainCache.get(g) ?? [];
  for (const old of recent) {
    if (old.camera !== document.camera) continue;
    if (old.splines === document.splines) {
      if (recent[0] !== old) recent.reverse();
      return old;
    }
    const rivers = document.splines?.filter((path) => path.kind === "river") ?? [];
    const oldRivers = old.splines?.filter((path) => path.kind === "river") ?? [];
    // Roads and walls do not deform terrain; editing them keeps the channel mesh valid.
    if (rivers.length === oldRivers.length && rivers.every((river, i) => river === oldRivers[i])) {
      old.splines = document.splines;
      if (recent[0] !== old) recent.reverse();
      return old;
    }
  }
  const base = baseTerrain(g);
  let triangles = base.triangles;
  if (document.camera && document.splines?.some((p) => p.kind === "river"))
    triangles = evaluateRiverChannels(triangles, {
      camera: document.camera,
      splines: document.splines,
    });
  const replacements = new Map<TerrainTriangle, TerrainTriangle[]>();
  if (triangles !== base.triangles)
    for (const triangle of triangles) {
      const source = riverChannelSource(triangle);
      if (source === triangle) continue;
      const parts = replacements.get(source);
      if (parts) parts.push(triangle);
      else replacements.set(source, [triangle]);
    }
  const result = {
    splines: document.splines,
    camera: document.camera,
    triangles,
    base,
    replacements,
    buckets: new Map<string, TerrainTriangle[]>(),
  };
  // Placement following alternates queries against the committed and preview terrain.
  // Retain both so every attached object does not rebuild both channel meshes.
  terrainCache.set(g, [result, ...recent.slice(0, 1)]);
  return result;
}
type TerrainBounds = [number, number, number, number];
const triangleBoundsCache = new WeakMap<TerrainTriangle, TerrainBounds>();
function triangleBounds(triangle: TerrainTriangle): TerrainBounds {
  let box = triangleBoundsCache.get(triangle);
  if (!box) {
    const [a, b, c] = triangle.points;
    box = [
      Math.min(a[0], b[0], c[0]),
      Math.min(a[1], b[1], c[1]),
      Math.max(a[0], b[0], c[0]),
      Math.max(a[1], b[1], c[1]),
    ];
    triangleBoundsCache.set(triangle, box);
  }
  return box;
}
function boundsOverlap(a: TerrainBounds, b: TerrainBounds) {
  return a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1];
}
function terrainBucket(
  data: NonNullable<ReturnType<typeof evaluatedTerrain>>,
  x: number,
  y: number,
) {
  const key = `${x}/${y}`;
  const cached = data.buckets.get(key);
  if (cached) return cached;
  const source = data.base.buckets.get(key) ?? [];
  if (!source.some((triangle) => data.replacements.has(triangle))) return source;
  const spacing = data.base.spacing;
  const box: TerrainBounds = [x * spacing, y * spacing, (x + 1) * spacing, (y + 1) * spacing];
  const result: TerrainTriangle[] = [];
  for (const triangle of source)
    for (const part of data.replacements.get(triangle) ?? [triangle])
      if (boundsOverlap(triangleBounds(part), box)) result.push(part);
  data.buckets.set(key, result);
  return result;
}
/** Exact nearby geometry, sharing the control mesh index across channel edits. */
export function terrainTrianglesInBounds(document: TerrainDocument, box: TerrainBounds) {
  const data = evaluatedTerrain(document);
  if (!data) return [];
  const result = new Set<TerrainTriangle>(),
    spacing = data.base.spacing;
  for (let x = Math.floor(box[0] / spacing); x <= Math.floor(box[2] / spacing); x++)
    for (let y = Math.floor(box[1] / spacing); y <= Math.floor(box[3] / spacing); y++)
      for (const triangle of terrainBucket(data, x, y))
        if (boundsOverlap(triangleBounds(triangle), box)) result.add(triangle);
  return [...result];
}
export function terrainTriangles(document: TerrainDocument): TerrainTriangle[] {
  return evaluatedTerrain(document)?.triangles ?? [];
}
export function triangleHeightAt(
  points: [Vec3, Vec3, Vec3],
  x: number,
  y: number,
): number | undefined {
  const [a, b, c] = points,
    d = cross(a, b, c),
    p: Vec3 = [x, y, 0];
  const u = cross(p, b, c) / d,
    v = cross(a, p, c) / d,
    w = 1 - u - v;
  return u >= -1e-7 && v >= -1e-7 && w >= -1e-7 ? u * a[2] + v * b[2] + w * c[2] : undefined;
}
export function terrainHeightAt(
  document: TerrainDocument,
  x: number,
  y: number,
): number | undefined {
  const data = evaluatedTerrain(document);
  if (!data) return undefined;
  for (const t of terrainBucket(
    data,
    Math.floor(x / data.base.spacing),
    Math.floor(y / data.base.spacing),
  )) {
    const z = triangleHeightAt(t.points, x, y);
    if (z !== undefined) return z;
  }
  return undefined;
}
export function terrainVertexMaterialMix(
  grid: TerrainGrid,
  vertex: TerrainVertex,
  fallback = "grass_short",
): Record<string, number> {
  return vertex.materialMix ?? { [vertex.material ?? (grid.texture ? "$source" : fallback)]: 1 };
}
export function mixTerrainMaterials(
  a: Record<string, number>,
  b: Record<string, number>,
  fraction = 0.5,
): Record<string, number> {
  const result: Record<string, number> = {};
  for (const [id, w] of Object.entries(a)) result[id] = (result[id] ?? 0) + w * (1 - fraction);
  for (const [id, w] of Object.entries(b)) result[id] = (result[id] ?? 0) + w * fraction;
  return result;
}
/** Refine selected cells and shared edges while retaining unrelated cells unchanged. */
export function subdivideTerrainCells(grid: TerrainGrid, selected: Iterable<string>): TerrainGrid {
  const wanted = new Set(selected);
  if (!wanted.size) return grid;
  const known = new Set(grid.cells.map((cell) => cell.id));
  for (const id of wanted)
    if (!known.has(id)) throw new Error(`Cannot subdivide unknown terrain cell ${id}`);
  const vertices = [...grid.vertices],
    midpoints = new Map<string, number>();
  const key = (a: number, b: number) => (a < b ? `${a}/${b}` : `${b}/${a}`);
  const midpoint = (a: number, b: number) => {
    const k = key(a, b),
      old = midpoints.get(k);
    if (old !== undefined) return old;
    const va = vertices[a]!,
      vb = vertices[b]!,
      index = vertices.length;
    vertices.push({
      id: `mid-${va.id}-${vb.id}`,
      materialMix:
        !grid.texture && !va.material && !vb.material && !va.materialMix && !vb.materialMix
          ? undefined
          : mixTerrainMaterials(
              terrainVertexMaterialMix(grid, va),
              terrainVertexMaterialMix(grid, vb),
            ),
      position: va.position.map((p, i) => (p + vb.position[i]!) / 2) as Vec3,
      ...(va.uv && vb.uv ? { uv: va.uv.map((p, i) => (p + vb.uv![i]!) / 2) as Point } : {}),
    });
    midpoints.set(k, index);
    return index;
  };
  const tris = terrainTriangles({ terrain: grid });
  for (const t of tris)
    if (wanted.has(t.cell.id))
      for (let i = 0; i < 3; i++) midpoint(t.indices[i]!, t.indices[(i + 1) % 3]!);
  const affected = new Set(wanted);
  const byCell = new Map<string, TerrainTriangle[]>();
  for (const t of tris) {
    const members = byCell.get(t.cell.id) ?? [];
    members.push(t);
    byCell.set(t.cell.id, members);
    if (t.indices.some((a, i) => midpoints.has(key(a, t.indices[(i + 1) % 3]!))))
      affected.add(t.cell.id);
  }
  const cells: TerrainCell[] = [];
  for (const cell of grid.cells) {
    if (!affected.has(cell.id)) {
      cells.push(cell);
      continue;
    }
    // Preserve the existing triangle planes only where new shared-edge vertices
    // require refinement. Unrelated polygons keep their identity and topology.
    for (const t of byCell.get(cell.id)!) {
      let pieces: number[][] = [t.indices];
      for (let e = 0; e < 3; e++) {
        const a = t.indices[e]!,
          b = t.indices[(e + 1) % 3]!,
          m = midpoints.get(key(a, b));
        if (m === undefined) continue;
        pieces = pieces.flatMap((p) => {
          const j = p.findIndex(
            (v, i) => (v === a && p[(i + 1) % 3] === b) || (v === b && p[(i + 1) % 3] === a),
          );
          if (j < 0) return [p];
          const u = p[j]!,
            v = p[(j + 1) % 3]!,
            w = p[(j + 2) % 3]!;
          return [
            [u, m, w],
            [m, v, w],
          ];
        });
      }
      pieces.forEach((p, i) =>
        cells.push({ ...t.cell, id: `${t.id}/${i}`, vertices: p, diagonal: undefined }),
      );
    }
  }
  const result = { ...grid, vertices, cells };
  validateTerrainGrid(result);
  return result;
}
/** Bounds are an editing guide; expansion adds ground and never removes existing vertices. */
export function expandTerrainGrid(
  grid: TerrainGrid,
  bounds: [number, number, number, number],
): TerrainGrid {
  const cover: MultiPolygon = grid.cells.map((c) => [
    c.vertices.map((i) => grid.vertices[i]!.position.slice(0, 2) as Point),
  ]);
  const base = createTerrainGrid(
    bounds,
    grid.spacing,
    0,
    grid.cells[0]?.material ?? "grass_short",
    grid.rowSpacing ?? grid.spacing,
  );
  const vertices = [...grid.vertices],
    cells = [...grid.cells];
  const lookup = new Map(vertices.map((v, i) => [`${v.position[0]},${v.position[1]}`, i]));
  for (const cell of base.cells) {
    const poly = cell.vertices.map((i) => base.vertices[i]!.position.slice(0, 2) as Point);
    const remaining = clipping.difference([poly], cover);
    for (const shape of remaining) {
      const f = flatten(shape),
        tri = earcut(f.vertices, f.holes, f.dimensions),
        ids: number[] = [];
      for (let i = 0; i < f.vertices.length; i += 2) {
        const x = f.vertices[i]!,
          y = f.vertices[i + 1]!,
          k = `${x},${y}`;
        let id = lookup.get(k);
        if (id === undefined) {
          id = vertices.length;
          vertices.push({
            id: `expanded-${id}`,
            material: cell.material,
            position: [x, y, terrainHeightAt({ terrain: grid }, x, y) ?? 0],
          });
          lookup.set(k, id);
        }
        ids.push(id);
      }
      for (let i = 0; i < tri.length; i += 3) {
        let v = tri.slice(i, i + 3).map((j) => ids[j]!);
        if (cross(...(v.map((j) => vertices[j]!.position) as [Vec3, Vec3, Vec3])) < 0) v.reverse();
        cells.push({ id: `expanded-cell-${cells.length}`, vertices: v, material: cell.material });
      }
    }
  }
  // New boundary vertices must also split the old neighboring triangles.
  for (let index = 0; index < grid.cells.length; index++) {
    const original = grid.cells[index]!;
    let pieces = cellTriangles(grid, original).map((t) => [...t]);
    let changed = false;
    for (let n = grid.vertices.length; n < vertices.length; n++) {
      const p = vertices[n]!.position;
      pieces = pieces.flatMap((t) => {
        for (let e = 0; e < 3; e++) {
          const a = vertices[t[e]!]!.position,
            b = vertices[t[(e + 1) % 3]!]!.position,
            dx = b[0] - a[0],
            dy = b[1] - a[1],
            den = dx * dx + dy * dy,
            u = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / den;
          if (u <= 1e-7 || u >= 1 - 1e-7 || Math.abs(cross(a, b, p)) > 1e-6) continue;
          changed = true;
          vertices[n] = {
            ...vertices[n]!,
            material: undefined,
            ...(vertices[t[e]!]!.uv && vertices[t[(e + 1) % 3]!]!.uv
              ? {
                  uv: vertices[t[e]!]!.uv!.map(
                    (v, i) => v + (vertices[t[(e + 1) % 3]!]!.uv![i]! - v) * u,
                  ) as Point,
                }
              : {}),
            materialMix: mixTerrainMaterials(
              terrainVertexMaterialMix(grid, vertices[t[e]!]!, original.material),
              terrainVertexMaterialMix(grid, vertices[t[(e + 1) % 3]!]!, original.material),
              u,
            ),
            position: [p[0], p[1], a[2] + u * (b[2] - a[2])],
          };
          return [
            [t[e]!, n, t[(e + 2) % 3]!],
            [n, t[(e + 1) % 3]!, t[(e + 2) % 3]!],
          ];
        }
        return [t];
      });
    }
    if (changed) {
      cells[index] = { ...original, vertices: pieces[0]!, diagonal: undefined };
      pieces.slice(1).forEach((p, i) =>
        cells.push({
          ...original,
          id: `${original.id}/boundary-${i}`,
          vertices: p,
          diagonal: undefined,
        }),
      );
    }
  }
  const result = { ...grid, vertices, cells };
  validateTerrainGrid(result);
  return result;
}
/** Shared with the visible strip so river boundaries match export and placement. */
export const terrainSplineCurve = splineCurve;
export function splineFootprint(path: LevelSpline, camera: MapCamera): MultiPolygon {
  const sin = Math.sin((camera.elevation_deg * Math.PI) / 180);
  const pairs = sampleSpline(path, camera).map((s) => {
    const n = new Vector3(-s.tangent.y, s.tangent.x, 0)
      .normalize()
      .multiplyScalar((s.width * s.lateralScale) / 2);
    return [
      [s.position.x - n.x, -(s.position.y - n.y) * sin],
      [s.position.x + n.x, -(s.position.y + n.y) * sin],
    ] as [Point, Point];
  });
  const strips: Polygon[] = pairs
    .slice(1)
    .map((p, i) => [[pairs[i]![0], pairs[i]![1], p[1], p[0]]]);
  return strips.length ? clipping.union(strips[0]!, ...strips.slice(1)) : [];
}

export function terrainGameplay(document: Level3D): GameplayAssetDescriptor | undefined {
  if (!document.terrain) return undefined;
  validateTerrainGrid(document.terrain);
  const surfaces: AssetWalkableSurface[] = [],
    movementBlockers: AssetWalkableSurface[] = [];
  const rivers = (document.splines ?? []).filter((p) => p.kind === "river");
  const footprints = rivers
    .flatMap((p) => splineFootprint(p, document.camera))
    .map((polygon) => {
      const points = polygon[0]!;
      return {
        polygon,
        minX: Math.min(...points.map((p) => p[0])),
        maxX: Math.max(...points.map((p) => p[0])),
        minY: Math.min(...points.map((p) => p[1])),
        maxY: Math.max(...points.map((p) => p[1])),
      };
    });
  const add = (id: string, points: Vec3[], materialId: string, blocked: boolean) => {
    const projected = points.map(([x, y, z]) => [x, y - z, z] as Vec3);
    if (Math.abs(cross(projected[0]!, projected[1]!, projected[2]!)) < 1e-5) return;
    // Generated subpixel channel slivers have no representable navigation area.
    const quantized = projected.map(([x, y, z]) => [Math.round(x), Math.round(y), z] as Vec3);
    if (Math.abs(cross(quantized[0]!, quantized[1]!, quantized[2]!)) < 1) return;
    const material = terrainMaterial(materialId, document.customMaterials);
    const surface: AssetWalkableSurface = {
      id,
      node: "$root",
      polygon: points.map((p) => [p[0], p[1]]),
      height: points.map((p) => p[2]),
      navigationRegion: "terrain",
      // Keep terrain and placed floor seams in the same continuous frame until
      // their collision cuts and navigation unions have been assembled.
      preserveMovementPrecision: true,
      projectionMaterials: { defaultMaterial: material.gameplayMaterial, regions: [] },
    };
    surfaces.push(surface);
    if (blocked)
      movementBlockers.push({
        ...surface,
        id: `blocked/${id}`,
        navigationRegion: undefined,
        projectionMaterials: undefined,
      });
  };
  for (const triangle of roadTerrainPieces(document, terrainTriangles(document))) {
    const weights = new Map<string, number>();
    (
      triangle.materialMixes ??
      (
        triangle.materials ?? [
          triangle.cell.material,
          triangle.cell.material,
          triangle.cell.material,
        ]
      ).map((id) => ({ [id]: 1 }))
    ).forEach((mix, i) => {
      const weight = triangle.materialWeights
        ? triangle.materialWeights.reduce((sum, w) => sum + w[i]!, 0) / 3
        : 1 / 3;
      for (const [id, w] of Object.entries(mix)) {
        const resolved = id === "$source" ? triangle.cell.material : id;
        weights.set(resolved, (weights.get(resolved) ?? 0) + weight * w);
      }
    });
    const materialId = [...weights].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]![0],
      material = terrainMaterial(materialId, document.customMaterials);
    const [a, b, c] = triangle.points.map(
      (p) => new Vector3(...gameToScene(document.camera, ...p)),
    );
    const normal = b!.clone().sub(a!).cross(c!.clone().sub(a!)).normalize();
    const steep = Math.abs(normal.z) < Math.SQRT1_2 - 1e-7;
    const blocked =
      triangle.cell.walkable === false ||
      (triangle.cell.walkable !== true && (material.gameplayMaterial === 5 || steep));
    const minX = Math.min(...triangle.points.map((p) => p[0])),
      maxX = Math.max(...triangle.points.map((p) => p[0]));
    const minY = Math.min(...triangle.points.map((p) => p[1])),
      maxY = Math.max(...triangle.points.map((p) => p[1]));
    const intersecting = footprints.filter(
      (p) => p.minX < maxX && p.maxX > minX && p.minY < maxY && p.maxY > minY,
    );
    if (!intersecting.length) {
      add(triangle.id, triangle.points, materialId, blocked);
      continue;
    }
    const ring = triangle.points.map((p) => [p[0], p[1]] as Point),
      parts = fixedPolygonBoolean(
        "difference",
        [ring],
        intersecting.map((p) => p.polygon),
      );
    const [planeA, planeB, planeC] = triangle.points;
    const planeArea = cross(planeA, planeB, planeC);
    for (const [partIndex, part] of parts.entries()) {
      const flat = flatten(part),
        indices = earcut(flat.vertices, flat.holes, flat.dimensions);
      for (let i = 0; i < indices.length; i += 3) {
        const points = indices.slice(i, i + 3).map((j) => {
          const x = flat.vertices[j * 2]!,
            y = flat.vertices[j * 2 + 1]!;
          // Fixed-point clipping may move an edge by a fraction of a pixel;
          // evaluate its affine plane without a point-inside rejection.
          const p: Vec3 = [x, y, 0];
          const u = cross(p, planeB, planeC) / planeArea,
            v = cross(planeA, p, planeC) / planeArea;
          return [x, y, u * planeA[2] + v * planeB[2] + (1 - u - v) * planeC[2]] as Vec3;
        });
        add(`${triangle.id}/cut-${partIndex}-${i}`, points, materialId, blocked);
      }
    }
  }
  const sin = Math.sin((document.camera.elevation_deg * Math.PI) / 180),
    cos = Math.cos((document.camera.elevation_deg * Math.PI) / 180);
  for (const path of rivers) {
    const samples = sampleSpline(path, document.camera);
    const pairs = samples.map((s) => {
      const n = new Vector3(-s.tangent.y, s.tangent.x, 0)
        .normalize()
        .multiplyScalar((s.width * s.lateralScale) / 2);
      return [
        [s.position.x - n.x, -(s.position.y - n.y) * sin, s.position.z * cos],
        [s.position.x + n.x, -(s.position.y + n.y) * sin, s.position.z * cos],
      ] as [Vec3, Vec3];
    });
    for (let i = 1; i < pairs.length; i++) {
      const s = samples[i]!,
        id =
          Object.entries(
            splineMaterialWeightsAt(
              path,
              (s.section + s.fraction) /
                (path.closed ? path.points.length : path.points.length - 1),
            ),
          ).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]?.[0] ?? "water_still";
      const blocked = id !== "water_ford";
      const a = pairs[i - 1]!,
        b = pairs[i]!;
      add(`river/${path.id}/${i}/0`, [a[0], a[1], b[0]], id, blocked);
      add(`river/${path.id}/${i}/1`, [a[1], b[1], b[0]], id, blocked);
    }
  }
  return {
    version: 1,
    kind: "projection-mapped-asset",
    id: "authored-terrain",
    name: "Authored terrain",
    source_map: document.map,
    model: "generated",
    editor_usage: "map-background",
    parts: [],
    gameplay: {
      version: 1,
      collision: "none",
      doors: [],
      surfaces,
      movementBlockers,
      materials: surfaces
        .filter((s) => s.projectionMaterials?.defaultMaterial === 5)
        .map((s) => ({
          id: `material/${s.id}`,
          node: "$root",
          polygon: s.polygon.map(
            ([x, y], i) => [x, y, typeof s.height === "number" ? s.height : s.height[i]!] as Vec3,
          ),
          material: 5,
          ground: true,
          obstacles: [],
        })),
    },
  };
}
