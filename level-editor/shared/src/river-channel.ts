import type { TerrainTriangle } from "./authored-terrain.ts";
import type { Level3D } from "./level3d.ts";
import type { Vec3 } from "./scene.ts";
import { sampleSpline, splineMaterialWeightsAt } from "./spline-sampling.ts";

type XY = [number, number];
type Plane = [number, number, number];
type Cut = { points: [Vec3, Vec3, Vec3]; plane: Plane; bounds: number[] };
const EPS = 1e-8;
const cross = (a: XY, b: XY, p: XY) =>
  (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
const xy = (p: Vec3): XY => [p[0], p[1]];
const area = (p: XY[]) =>
  p.reduce(
    (a, v, i) => a + v[0] * p[(i + 1) % p.length]![1] - v[1] * p[(i + 1) % p.length]![0],
    0,
  ) / 2;
const height = (p: Plane, v: XY) => p[0] * v[0] + p[1] * v[1] + p[2];
function plane([a, b, c]: [Vec3, Vec3, Vec3]): Plane {
  const d = cross(xy(a), xy(b), xy(c));
  const x = ((b[2] - a[2]) * (c[1] - a[1]) - (c[2] - a[2]) * (b[1] - a[1])) / d;
  const y = ((b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])) / d;
  return [x, y, a[2] - x * a[0] - y * a[1]];
}
function clip(poly: XY[], distance: (p: XY) => number, positive: boolean): XY[] {
  const out: XY[] = [];
  for (let i = 0; i < poly.length; i++) {
    const a = poly[i]!,
      b = poly[(i + 1) % poly.length]!,
      da = distance(a) * (positive ? 1 : -1),
      db = distance(b) * (positive ? 1 : -1);
    if (da >= 0) out.push(a);
    if ((da < 0 && db > 0) || (da > 0 && db < 0)) {
      const t = da / (da - db);
      out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
    }
  }
  return out.filter((p, i) => {
    const q = out[(i + out.length - 1) % out.length]!;
    return Math.hypot(p[0] - q[0], p[1] - q[1]) > EPS;
  });
}
function useful(p: XY[]) {
  return p.length >= 3 && Math.abs(area(p)) > EPS;
}
function bounds(p: XY[]) {
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  for (const [x, y] of p) {
    minX = Math.min(minX, x);
    minY = Math.min(minY, y);
    maxX = Math.max(maxX, x);
    maxY = Math.max(maxY, y);
  }
  return [minX, minY, maxX, maxY];
}
function channelPiece(poly: XY[], plane: Plane) {
  return { poly, plane, bounds: bounds(poly) };
}

function overlaps(a: number[], b: number[]) {
  return a[0]! <= b[2]! && a[2]! >= b[0]! && a[1]! <= b[3]! && a[3]! >= b[1]!;
}

/** Build a piecewise planar excavation volume. Width is the waterline width;
 * slope is scene-space rise/run, and depth uses the same game pixels as Z. */
function channelCuts(document: Pick<Level3D, "splines" | "camera">, maxHeight: number): Cut[] {
  const cuts: Cut[] = [],
    angle = (document.camera.elevation_deg * Math.PI) / 180,
    sin = Math.sin(angle),
    cos = Math.cos(angle);
  const add = (points: [Vec3, Vec3, Vec3]) => {
    const a = cross(xy(points[0]), xy(points[1]), xy(points[2]));
    if (Math.abs(a) < EPS) return;
    if (a < 0) points.reverse();
    cuts.push({ points, plane: plane(points), bounds: bounds(points.map(xy)) });
  };
  for (const river of document.splines ?? []) {
    if (river.kind !== "river" || river.channel?.enabled === false || river.points.length < 2)
      continue;
    const depth = river.channel?.bedDepth ?? 24,
      slope = river.channel?.bankSlope ?? 1;
    if (depth <= 0 || slope <= 0) continue;
    const samples = sampleSpline(river, document.camera);
    const rawProfiles = samples.map((s) => {
      const p = s.position,
        half = s.width / 2;
      const parameter =
        (s.section + s.fraction) / (river.closed ? river.points.length : river.points.length - 1);
      const ford = Math.max(
        0,
        Math.min(1, splineMaterialWeightsAt(river, parameter).water_ford ?? 0),
      );
      // Fords are shallow crossings: retain a bed and banks gentle enough to
      // walk across rather than only changing the water's visual appearance.
      const fordSlope = Math.min(slope, 0.75);
      const fordDepth = Math.min(depth, half * cos * fordSlope * 0.5);
      const localDepth = depth + (fordDepth - depth) * ford;
      const localSlope = slope + (fordSlope - slope) * ford;
      const bed = p.z - localDepth / cos;
      // A narrow non-ford channel becomes V-shaped when its full depth cannot fit.
      const inner = Math.max(0, half - localDepth / cos / localSlope);
      const actualSlope = localDepth / cos / (half - inner);
      const top = Math.max(maxHeight / cos + 1, p.z),
        outer = inner + (top - bed) / actualSlope;
      const len = Math.hypot(s.tangent.x, s.tangent.y),
        nx = -s.tangent.y / len,
        ny = s.tangent.x / len;
      const point = (offset: number, z: number): Vec3 => [
        p.x + nx * offset,
        -(p.y + ny * offset) * sin,
        z * cos,
      ];
      return {
        p,
        inner,
        outer,
        bed,
        top,
        points: [point(-outer, top), point(-inner, bed), point(inner, bed), point(outer, top)],
      };
    });
    // Remove redundant samples on straight constant-profile reaches. Keeping
    // every sample would introduce hundreds of invisible gameplay boundaries.
    const profiles: typeof rawProfiles = [];
    for (const profile of rawProfiles) {
      profiles.push(profile);
      while (profiles.length >= 3) {
        const a = profiles.at(-3)!,
          b = profiles.at(-2)!,
          c = profiles.at(-1)!;
        const span = c.p.clone().sub(a.p),
          denom = span.lengthSq();
        if (denom < EPS) break;
        const t = b.p.clone().sub(a.p).dot(span) / denom;
        if (
          t <= 0 ||
          t >= 1 ||
          !b.points.every((p, i) =>
            p.every(
              (value, k) =>
                Math.abs(value - (a.points[i]![k]! + (c.points[i]![k]! - a.points[i]![k]!) * t)) <
                1e-7,
            ),
          )
        )
          break;
        profiles.splice(profiles.length - 2, 1);
      }
    }
    for (let i = 1; i < profiles.length; i++)
      for (let j = 0; j < 3; j++) {
        const a = profiles[i - 1]!.points,
          b = profiles[i]!.points;
        add([a[j]!, a[j + 1]!, b[j + 1]!]);
        add([a[j]!, b[j + 1]!, b[j]!]);
      }
    // Rounded endcaps keep open rivers continuous with unmodified ground.
    if (!river.closed)
      for (const profile of [profiles[0]!, profiles.at(-1)!]) {
        const { p, inner, outer, bed, top } = profile;
        const ring = (r: number, z: number, t: number): Vec3 => [
          p.x + Math.cos(t) * r,
          -(p.y + Math.sin(t) * r) * sin,
          z * cos,
        ];
        for (let i = 0; i < 16; i++) {
          const a = (i * Math.PI) / 8,
            b = ((i + 1) * Math.PI) / 8,
            center: Vec3 = [p.x, -p.y * sin, bed * cos];
          const ia = ring(inner, bed, a),
            ib = ring(inner, bed, b),
            oa = ring(outer, top, a),
            ob = ring(outer, top, b);
          add([center, ia, ib]);
          add([ia, oa, ob]);
          add([ia, ob, ib]);
        }
      }
  }
  return cuts;
}

// Documents are immutable. Keep only the latest evaluation per river-array identity;
// unchanged triangles can retain their exact channel tessellation across local edits.
const evaluations = new WeakMap<
  NonNullable<Level3D["splines"]>,
  {
    signature: string;
    cuts: Cut[];
    triangles: Map<string, { signature: string; result: TerrainTriangle[] }>;
  }
>();

/** Non-destructive lower envelope of the base mesh and all river excavations.
 * Splits at bank/bed contours and plane crossings even on very coarse grids.
 * Overlapping channels use the lower bed, independently of river ordering. */
export function evaluateRiverChannels(
  base: TerrainTriangle[],
  document: Pick<Level3D, "splines" | "camera">,
): TerrainTriangle[] {
  if (
    !base.length ||
    !document.splines?.some((p) => p.kind === "river" && p.channel?.enabled !== false)
  )
    return base;
  const maxHeight = base.reduce((max, t) => Math.max(max, ...t.points.map((p) => p[2])), -Infinity);
  const signature = JSON.stringify([document.camera, document.splines, maxHeight]);
  let evaluation = evaluations.get(document.splines);
  if (!evaluation || evaluation.signature !== signature) {
    evaluation = { signature, cuts: channelCuts(document, maxHeight), triangles: new Map() };
    evaluations.set(document.splines, evaluation);
  }
  const cuts = evaluation.cuts;
  if (!cuts.length) return base;
  const result: TerrainTriangle[] = [];
  const current = new Map<string, { signature: string; result: TerrainTriangle[] }>();
  for (const source of base) {
    const cached = evaluation.triangles.get(source.id);
    const sourceSignature = JSON.stringify(source);
    if (cached?.signature === sourceSignature) {
      current.set(source.id, cached);
      for (const triangle of cached.result) result.push(triangle);
      continue;
    }
    const start = result.length;
    const sourceBounds = bounds(source.points.map(xy));
    const nearby = cuts.filter((cut) => overlaps(sourceBounds, cut.bounds));
    if (!nearby.length) {
      result.push(source);
      current.set(source.id, { signature: sourceSignature, result: [source] });
      continue;
    }
    let pieces = [channelPiece(source.points.map(xy), plane(source.points))];
    for (const cut of nearby) {
      const outside: typeof pieces = [];
      for (const piece of pieces) {
        if (
          !overlaps(piece.bounds, cut.bounds) ||
          piece.poly.every((p) => height(piece.plane, p) <= height(cut.plane, p) + EPS)
        ) {
          outside.push(piece);
          continue;
        }
        let inside = piece.poly;
        for (let i = 0; i < 3 && useful(inside); i++) {
          const a = xy(cut.points[i]!),
            b = xy(cut.points[(i + 1) % 3]!);
          const distance = (p: XY) => cross(a, b, p);
          const part = clip(inside, distance, false);
          if (useful(part)) outside.push(channelPiece(part, piece.plane));
          inside = clip(inside, distance, true);
        }
        if (useful(inside)) {
          const delta = (p: XY) => height(piece.plane, p) - height(cut.plane, p);
          if (inside.every((p) => delta(p) <= EPS)) outside.push(channelPiece(inside, piece.plane));
          else if (inside.every((p) => delta(p) >= -EPS))
            outside.push(channelPiece(inside, cut.plane));
          else {
            const lower = clip(inside, delta, true),
              upper = clip(inside, delta, false);
            if (useful(lower)) outside.push(channelPiece(lower, cut.plane));
            if (useful(upper)) outside.push(channelPiece(upper, piece.plane));
          }
        }
      }
      pieces = outside;
    }
    let index = 0;
    for (const piece of pieces)
      for (let i = 1; i < piece.poly.length - 1; i++) {
        const points = [piece.poly[0]!, piece.poly[i]!, piece.poly[i + 1]!].map(
          (p) => [p[0], p[1], height(piece.plane, p)] as Vec3,
        ) as [Vec3, Vec3, Vec3];
        if (Math.abs(cross(...(points.map(xy) as [XY, XY, XY]))) < EPS) continue;
        const barycentric = points.map((p) => {
          const [a, b, c] = source.points.map(xy) as [XY, XY, XY],
            d = cross(a, b, c);
          const u = cross(xy(p), b, c) / d,
            v = cross(a, xy(p), c) / d;
          return [u, v, 1 - u - v] as Vec3;
        });
        const interpolate = (values: number[][], dimensions: number) =>
          barycentric.map((weights) =>
            Array.from({ length: dimensions }, (_, k) =>
              weights.reduce((sum, w, i) => sum + w * values[i]![k]!, 0),
            ),
          );
        const uv = source.uv ? (interpolate(source.uv, 2) as [XY, XY, XY]) : undefined;
        const materialWeights = source.materials
          ? (interpolate(
              source.materialWeights ?? [
                [1, 0, 0],
                [0, 1, 0],
                [0, 0, 1],
              ],
              3,
            ) as [Vec3, Vec3, Vec3])
          : undefined;
        result.push({
          ...source,
          id: `${source.id}/channel-${index++}`,
          points,
          ...(uv ? { uv } : {}),
          ...(materialWeights ? { materialWeights } : {}),
        });
      }
    current.set(source.id, { signature: sourceSignature, result: result.slice(start) });
  }
  evaluation.triangles = current;
  return result;
}
