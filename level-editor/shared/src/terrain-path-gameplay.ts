import earcut, { flatten } from "earcut";
import type { MultiPolygon, Polygon } from "polygon-clipping";
import { fixedClipping as clipping } from "./fixed-polygon-boolean.ts";
import type { TerrainTriangle } from "./authored-terrain.ts";
import type { Level3D } from "./level3d.ts";
import type { Vec3 } from "./scene.ts";
import { sampleSpline, splineMaterialWeightsAt } from "./spline-sampling.ts";

type Bounds = [number, number, number, number];
const bounds = (points: number[][]): Bounds => [
  Math.min(...points.map((p) => p[0]!)),
  Math.min(...points.map((p) => p[1]!)),
  Math.max(...points.map((p) => p[0]!)),
  Math.max(...points.map((p) => p[1]!)),
];
const overlaps = (a: Bounds, b: Bounds) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
const cross = (a: number[], b: number[], c: number[]) =>
  (b[0]! - a[0]!) * (c[1]! - a[1]!) - (b[1]! - a[1]!) * (c[0]! - a[0]!);
function height(points: TerrainTriangle["points"], x: number, y: number) {
  const [a, b, c] = points,
    p = [x, y],
    area = cross(a, b, c);
  const u = cross(p, b, c) / area,
    v = cross(a, p, c) / area;
  return u * a[2] + v * b[2] + (1 - u - v) * c[2];
}

/** Gameplay-only pieces: roads replace surface material while retaining ground planes. */
export function roadTerrainPieces(
  document: Pick<Level3D, "splines" | "camera">,
  baseTriangles: TerrainTriangle[],
): TerrainTriangle[] {
  const sin = Math.sin((document.camera.elevation_deg * Math.PI) / 180);
  const strips: { polygon: Polygon; bounds: Bounds; material: string }[] = [];
  for (const path of document.splines ?? []) {
    if (path.kind !== "road" || path.points.length < 2) continue;
    const samples = sampleSpline(path, document.camera);
    const pairs = samples.map((s) => {
      const length = Math.hypot(s.tangent.x, s.tangent.y);
      const dx = length ? ((-s.tangent.y / length) * s.width) / 2 : 0;
      const dy = length ? ((s.tangent.x / length) * s.width) / 2 : 0;
      return [
        [s.position.x - dx, -(s.position.y - dy) * sin],
        [s.position.x + dx, -(s.position.y + dy) * sin],
      ] as [number[], number[]];
    });
    const sections = path.closed ? path.points.length : path.points.length - 1;
    for (let i = 1; i < pairs.length; i++) {
      const a = samples[i - 1]!,
        b = samples[i]!;
      const parameter = (a.section + a.fraction + (b.section + b.fraction)) / (2 * sections);
      const weights = Object.entries(splineMaterialWeightsAt(path, parameter));
      const material = weights.reduce((best, entry) => (entry[1] > best[1] ? entry : best))[0];
      const ring = [pairs[i - 1]![0], pairs[i - 1]![1], pairs[i]![1], pairs[i]![0]] as [
        number,
        number,
      ][];
      strips.push({ polygon: [ring], bounds: bounds(ring), material });
    }
  }
  if (!strips.length) return baseTriangles;
  return baseTriangles.flatMap((triangle) => {
    const ring = triangle.points.map((p) => [p[0], p[1]] as [number, number]);
    const triangleBounds = bounds(ring);
    const candidates = strips.filter((strip) => overlaps(triangleBounds, strip.bounds));
    if (!candidates.length) return [triangle];
    let pieces: { polygon: Polygon; material?: string }[] = [{ polygon: [ring] }];
    let changed = false;
    for (const strip of candidates) {
      pieces = pieces.flatMap((piece) => {
        if (piece.material === strip.material || !overlaps(bounds(piece.polygon[0]!), strip.bounds))
          return [piece];
        const covered: MultiPolygon = clipping.intersection(piece.polygon, strip.polygon);
        if (!covered.length) return [piece];
        changed = true;
        return [
          ...clipping
            .difference(piece.polygon, strip.polygon)
            .map((polygon) => ({ polygon, material: piece.material })),
          ...covered.map((polygon) => ({ polygon, material: strip.material })),
        ];
      });
    }
    if (!changed) return [triangle];
    return pieces.flatMap((piece, part) => {
      const flat = flatten(piece.polygon),
        indices = earcut(flat.vertices, flat.holes, flat.dimensions);
      const out: TerrainTriangle[] = [];
      for (let i = 0; i < indices.length; i += 3) {
        const points = indices.slice(i, i + 3).map((index) => {
          const x = flat.vertices[index * 2]!,
            y = flat.vertices[index * 2 + 1]!;
          return [x, y, height(triangle.points, x, y)] as Vec3;
        }) as TerrainTriangle["points"];
        if (Math.abs(cross(...points)) < 1e-8) continue;
        if (cross(...points) < 0) [points[1], points[2]] = [points[2], points[1]];
        out.push({
          ...triangle,
          id: `${triangle.id}/road-${part}-${i}`,
          points,
          // Generated points do not reference the document's vertex buffer.
          indices: [-1, -1, -1],
          uv: undefined,
          materialWeights: piece.material
            ? undefined
            : (points.map((point) => {
                const [a, b, c] = triangle.points,
                  area = cross(a, b, c);
                const u = cross(point, b, c) / area,
                  v = cross(a, point, c) / area;
                const barycentric: Vec3 = [u, v, 1 - u - v];
                if (!triangle.materialWeights) return barycentric;
                return [0, 1, 2].map((component) =>
                  triangle.materialWeights!.reduce(
                    (sum, weights, index) => sum + weights[component]! * barycentric[index]!,
                    0,
                  ),
                ) as Vec3;
              }) as [Vec3, Vec3, Vec3]),
          ...(piece.material
            ? {
                cell: { ...triangle.cell, material: piece.material },
                materialMixes: [
                  { [piece.material]: 1 },
                  { [piece.material]: 1 },
                  { [piece.material]: 1 },
                ],
                materials: [piece.material, piece.material, piece.material] as [
                  string,
                  string,
                  string,
                ],
              }
            : {}),
        });
      }
      return out;
    });
  });
}
