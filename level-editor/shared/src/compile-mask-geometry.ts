import type { Mask, Point } from "./level.ts";
import type { Vec3 } from "./scene.ts";
import { encodeMaskBitmap } from "./encode-mask-bitmap.ts";
import { decodeMaskAlphaCoverage, type MaskAlphaCoverage } from "./mask-alpha-sampler.ts";

export type MaskTriangle = [Vec3, Vec3, Vec3];

/** Front envelope of an authored boundary. Recomputed after placement,
 * so rotating a concave footprint does not leave a backwards masking polyline. */
export function maskBoundaryPolyline(boundary: Point[], closed = true): Point[] {
  if (!closed && boundary.length >= 2) {
    const rounded = boundary.map(([x, y]): Point => [Math.round(x), Math.round(y)]);
    const monotone = (points: Point[]) => points.every((p, i) => !i || p[0] >= points[i - 1]![0]);
    if (rounded[0]![0] !== rounded.at(-1)![0]) {
      // Retain authored vertical endpoints as well as interior steps. They
      // carry bounds even when no nonvertical envelope segment ends there.
      if (monotone(rounded)) return rounded;
      const reversed = [...rounded].reverse();
      if (monotone(reversed)) return reversed;
    }
  }
  const edges = (closed ? boundary : boundary.slice(0, -1))
    .map((a, i) => [a, boundary[(i + 1) % boundary.length]!] as const)
    .filter(([a, b]) => a[0] !== b[0]);
  const at = ([a, b]: (typeof edges)[number], x: number) =>
    a[1] + ((b[1] - a[1]) * (x - a[0])) / (b[0] - a[0]);
  const xs = new Set(boundary.map(([x]) => x));
  for (let i = 0; i < edges.length; i++)
    for (let j = i + 1; j < edges.length; j++) {
      const a = edges[i]!,
        b = edges[j]!;
      const left = Math.max(Math.min(a[0][0], a[1][0]), Math.min(b[0][0], b[1][0]));
      const right = Math.min(Math.max(a[0][0], a[1][0]), Math.max(b[0][0], b[1][0]));
      if (right <= left) continue;
      const dl = at(a, left) - at(b, left),
        dr = at(a, right) - at(b, right);
      if (dl * dr < 0) xs.add(left + ((right - left) * dl) / (dl - dr));
    }
  const positions = [...xs].sort((a, b) => a - b);
  const result: Point[] = [];
  for (let i = 1; i < positions.length; i++) {
    const left = positions[i - 1]!,
      right = positions[i]!,
      middle = (left + right) / 2;
    const crossing = edges.filter(
      ([a, b]) => middle > Math.min(a[0], b[0]) && middle < Math.max(a[0], b[0]),
    );
    const front = crossing.sort((a, b) => at(b, middle) - at(a, middle))[0];
    if (!front) throw new Error("Mask boundary has no front envelope");
    for (const x of [left, right]) {
      const point: Point = [Math.round(x), Math.round(at(front, x))],
        last = result.at(-1);
      // Preserve vertical steps with repeated X values, instead of replacing a
      // concave notch with a sloped line that masks the wrong side of the notch.
      if (!last || last[0] !== point[0] || last[1] !== point[1]) result.push(point);
    }
  }
  if (result.length < 2 || result[0]![0] === result.at(-1)![0])
    throw new Error("Mask boundary collapses after placement");
  return result;
}

/** Rasterize placed 3D coverage in bounded tiles. Bitmap holes remain empty
 * unless covered by another triangle. Winding matters only for authored one-sided coverage. */
export function rasterizeMaskGeometry(
  triangles: MaskTriangle[],
  rules: Pick<
    Mask,
    "layer" | "mask_type" | "character_polyline" | "projectile_polyline" | "obstacle_indices"
  >,
  cullBackfaces = false,
  alphaCoverage?: MaskAlphaCoverage,
): Mask[] {
  if (!triangles.length) throw new Error("Mask coverage requires triangles");
  const acceptsAlpha = alphaCoverage && decodeMaskAlphaCoverage(alphaCoverage, triangles.length);
  // Scene/game matrix roundoff must not add a whole empty border row or column.
  const snap = (n: number) => (Math.abs(n - Math.round(n)) < 1e-7 ? Math.round(n) : n);
  const projected = triangles.map((triangle) =>
    triangle.map(([x, y, z]): Point => [snap(x), snap(y - z)]),
  );
  const box = [Infinity, Infinity, -Infinity, -Infinity];
  for (const triangle of projected)
    for (const [x, y] of triangle) {
      if (!Number.isFinite(x) || !Number.isFinite(y))
        throw new Error("Mask coverage must have finite coordinates");
      box[0] = Math.min(box[0]!, x);
      box[1] = Math.min(box[1]!, y);
      box[2] = Math.max(box[2]!, x);
      box[3] = Math.max(box[3]!, y);
    }
  const minX = Math.floor(box[0]!),
    minY = Math.floor(box[1]!),
    maxX = Math.max(minX + 1, Math.ceil(box[2]!)),
    maxY = Math.max(minY + 1, Math.ceil(box[3]!));
  if (Math.min(minX, minY) < -32768 || Math.max(maxX, maxY) > 32767)
    throw new Error("Mask coverage exceeds signed 16-bit coordinates");
  if ((maxX - minX) * (maxY - minY) > 64 * 1024 * 1024)
    throw new Error("Mask coverage exceeds 64 megapixels");
  const masks: Mask[] = [];
  const cross = (a: Point, b: Point, x: number, y: number) =>
    (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
  for (let top = minY; top < maxY; top += 1024)
    for (let left = minX; left < maxX; left += 1024) {
      const width = Math.min(1024, maxX - left),
        height = Math.min(1024, maxY - top);
      const pixels = new Uint8Array(width * height);
      for (const [index, triangle] of projected.entries()) {
        const a = triangle[0]!,
          b = triangle[1]!,
          c = triangle[2]!;
        const area = cross(a, b, c[0], c[1]),
          sign = Math.sign(area);
        // Projected map Y points downward: front-facing mesh winding is negative.
        if (
          Math.abs(area) < 1e-10 ||
          (cullBackfaces && area > 0 && !alphaCoverage?.triangles[index]?.doubleSided)
        )
          continue;
        const x1 = Math.max(left, Math.floor(Math.min(a[0], b[0], c[0]))),
          x2 = Math.min(left + width, Math.ceil(Math.max(a[0], b[0], c[0]))),
          y1 = Math.max(top, Math.floor(Math.min(a[1], b[1], c[1]))),
          y2 = Math.min(top + height, Math.ceil(Math.max(a[1], b[1], c[1])));
        if (x2 <= x1 || y2 <= y1) continue;
        for (let y = y1; y < y2; y++)
          for (let x = x1; x < x2; x++) {
            if (
              cross(a, b, x + 0.5, y + 0.5) * sign >= -1e-8 &&
              cross(b, c, x + 0.5, y + 0.5) * sign >= -1e-8 &&
              cross(c, a, x + 0.5, y + 0.5) * sign >= -1e-8 &&
              (!acceptsAlpha ||
                acceptsAlpha(
                  index,
                  cross(b, c, x + 0.5, y + 0.5) / area,
                  cross(c, a, x + 0.5, y + 0.5) / area,
                  cross(a, b, x + 0.5, y + 0.5) / area,
                ))
            )
              pixels[(y - top) * width + x - left] = 1;
          }
      }
      masks.push({
        ...rules,
        box_top_left: [left, top],
        box_size: [width, height],
        mask_data: encodeMaskBitmap(pixels, width, height),
      });
    }
  return masks;
}
