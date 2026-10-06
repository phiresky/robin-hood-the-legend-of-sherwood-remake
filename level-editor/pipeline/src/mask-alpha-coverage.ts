import type { Vec3 } from "../../shared/src/scene.ts";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";

export interface MaskAlphaImage {
  width: number;
  height: number;
  alpha: Uint8Array;
}
interface Vertex {
  point: Vec3;
  uv: [number, number];
  alpha: number;
}

function clip(polygon: Vertex[], distance: (vertex: Vertex) => number): Vertex[] {
  const output: Vertex[] = [];
  for (const [i, a] of polygon.entries()) {
    const b = polygon[(i + 1) % polygon.length]!;
    const da = distance(a),
      db = distance(b);
    if (da >= 0) output.push(a);
    if (da < 0 !== db < 0) {
      const t = da / (da - db);
      output.push({
        point: a.point.map((value, axis) => value + t * (b.point[axis]! - value)) as Vec3,
        uv: a.uv.map((value, axis) => value + t * (b.uv[axis]! - value)) as [number, number],
        alpha: a.alpha + t * (b.alpha - a.alpha),
      });
    }
  }
  return output;
}

/** Intersect an actual mesh triangle with nearest-sampled base-level alpha.
 * UV clipping interpolates positions directly, including degenerate UV maps.
 * Filtering/mipmap silhouettes are not represented by these authoring surfaces. */
export function maskAlphaCoverage(
  points: MaskTriangle,
  uv: [number, number][],
  vertexAlpha: number[],
  cutoff: number,
  texture?: MaskAlphaImage,
  clampAxes: readonly [boolean, boolean] = [false, false],
): MaskTriangle[] {
  if (
    points.length !== 3 ||
    points.some((p) => p.length !== 3 || p.some((v) => !Number.isFinite(v))) ||
    uv.length !== 3 ||
    vertexAlpha.length !== 3 ||
    !Number.isFinite(cutoff) ||
    cutoff < 0 ||
    cutoff > 1 ||
    vertexAlpha.some((a) => !Number.isFinite(a) || a < 0 || a > 1) ||
    uv.some(
      (p) =>
        p.length !== 2 ||
        p.some((v, axis) => !Number.isFinite(v) || (!clampAxes[axis] && (v < 0 || v > 1))),
    )
  )
    throw new Error("Mask alpha recovery requires finite in-range UVs, alpha and cutoff");
  if (
    texture &&
    (!Number.isSafeInteger(texture.width) ||
      !Number.isSafeInteger(texture.height) ||
      texture.width <= 0 ||
      texture.height <= 0 ||
      texture.alpha.length !== texture.width * texture.height)
  )
    throw new Error("Invalid mask alpha texture dimensions");
  const original = points.map((point, i): Vertex => ({
    point,
    uv: uv[i]!,
    alpha: vertexAlpha[i]!,
  }));
  const triangles: MaskTriangle[] = [];
  const emit = (polygon: Vertex[], alpha: number) => {
    const visible = clip(polygon, (vertex) => vertex.alpha * alpha - cutoff);
    for (let i = 1; i + 1 < visible.length; i++) {
      const triangle: MaskTriangle = [visible[0]!.point, visible[i]!.point, visible[i + 1]!.point];
      const [a, b, c] = triangle;
      const ab = b.map((v, axis) => v - a[axis]!),
        ac = c.map((v, axis) => v - a[axis]!);
      const cross = [
        ab[1]! * ac[2]! - ab[2]! * ac[1]!,
        ab[2]! * ac[0]! - ab[0]! * ac[2]!,
        ab[0]! * ac[1]! - ab[1]! * ac[0]!,
      ];
      if (cross.some((value) => value !== 0)) triangles.push(triangle);
    }
  };
  if (!texture) {
    emit(original, 1);
    return triangles;
  }
  const { width, height, alpha } = texture;
  const pixel = (value: number, size: number) =>
    Math.max(0, Math.min(size - 1, Math.floor(value * size)));
  const left = pixel(Math.min(...uv.map((p) => p[0])), width);
  const right = pixel(Math.max(...uv.map((p) => p[0])), width) + 1;
  const top = pixel(Math.min(...uv.map((p) => p[1])), height);
  const bottom = pixel(Math.max(...uv.map((p) => p[1])), height) + 1;
  const maximumAlpha = Math.max(...vertexAlpha);
  // With uniform vertex alpha, accepted texels all produce the same solid
  // geometry. Keep raw alpha only when it changes an interpolated boundary.
  const constantAlpha = vertexAlpha.every((value) => value === vertexAlpha[0]);
  const coverage = (x: number, y: number): number => {
    const value = alpha[y * width + x]!;
    return constantAlpha ? ((value / 255) * maximumAlpha >= cutoff ? 255 : 0) : value;
  };
  type Rectangle = { left: number; right: number; top: number; bottom: number; alpha: number };
  const rectangles: Rectangle[] = [];
  let previous = new Map<string, Rectangle>();
  for (let y = top; y < bottom; y++) {
    const current = new Map<string, Rectangle>();
    let x = left;
    while (x < right) {
      const value = coverage(x, y);
      if ((value / 255) * maximumAlpha < cutoff) {
        x++;
        continue;
      }
      const start = x++;
      while (x < right && coverage(x, y) === value) x++;
      const key = `${start}:${x}:${value}`;
      let rectangle = previous.get(key);
      if (rectangle) rectangle.bottom++;
      else {
        rectangle = { left: start, right: x, top: y, bottom: y + 1, alpha: value / 255 };
        rectangles.push(rectangle);
      }
      current.set(key, rectangle);
    }
    previous = current;
  }
  for (const rectangle of rectangles) {
    // Extend edge texels to the sampler's clamped exterior. Clamping triangle
    // vertices instead would change interpolated UVs inside the texture.
    let polygon = original;
    if (!clampAxes[0] || rectangle.left !== 0)
      polygon = clip(polygon, (v) => v.uv[0] - rectangle.left / width);
    if (!clampAxes[0] || rectangle.right !== width)
      polygon = clip(polygon, (v) => rectangle.right / width - v.uv[0]);
    if (!clampAxes[1] || rectangle.top !== 0)
      polygon = clip(polygon, (v) => v.uv[1] - rectangle.top / height);
    if (!clampAxes[1] || rectangle.bottom !== height)
      polygon = clip(polygon, (v) => rectangle.bottom / height - v.uv[1]);
    emit(polygon, rectangle.alpha);
  }
  return triangles;
}
