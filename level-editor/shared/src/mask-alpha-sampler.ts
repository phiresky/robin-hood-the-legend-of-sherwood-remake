import type { Point } from "./level.ts";

export type MaskTextureWrap = "clamp" | "repeat" | "mirror";
export interface MaskTriangleAlpha {
  uv: [Point, Point, Point];
  alpha: [number, number, number];
  cutoff: number;
  texture?: number;
  wrap: [MaskTextureWrap, MaskTextureWrap];
  doubleSided?: boolean;
}
/** Original mesh triangles retain UVs instead of expanding alpha texels into geometry. */
export interface MaskAlphaCoverage {
  textures: { width: number; height: number; alphaBase64: string }[];
  triangles: MaskTriangleAlpha[];
}

export function decodeMaskAlphaCoverage(coverage: MaskAlphaCoverage, triangleCount: number) {
  if (
    !Array.isArray(coverage.textures) ||
    !Array.isArray(coverage.triangles) ||
    coverage.triangles.length !== triangleCount
  )
    throw new Error("Mask alpha coverage must match its mesh triangles");
  let total = 0;
  const textures = coverage.textures.map((texture) => {
    const size = texture.width * texture.height;
    total += size;
    if (
      !Number.isSafeInteger(texture.width) ||
      !Number.isSafeInteger(texture.height) ||
      texture.width <= 0 ||
      texture.height <= 0 ||
      total > 64 * 1024 * 1024 ||
      typeof texture.alphaBase64 !== "string" ||
      texture.alphaBase64.length !== 4 * Math.ceil(size / 3) ||
      !/^[A-Za-z0-9+/]*={0,2}$/.test(texture.alphaBase64)
    )
      throw new Error("Invalid mask alpha texture");
    const decoded = atob(texture.alphaBase64);
    if (decoded.length !== size) throw new Error("Mask alpha texture size mismatch");
    return {
      width: texture.width,
      height: texture.height,
      alpha: Uint8Array.from(decoded, (character) => character.charCodeAt(0)),
    };
  });
  for (const triangle of coverage.triangles) {
    if (
      !Array.isArray(triangle.uv) ||
      triangle.uv.length !== 3 ||
      !triangle.uv.every((p) => Array.isArray(p) && p.length === 2 && p.every(Number.isFinite)) ||
      !Array.isArray(triangle.alpha) ||
      triangle.alpha.length !== 3 ||
      !triangle.alpha.every((a) => Number.isFinite(a) && a >= 0 && a <= 1) ||
      !Number.isFinite(triangle.cutoff) ||
      triangle.cutoff < 0 ||
      triangle.cutoff > 1 ||
      !Array.isArray(triangle.wrap) ||
      triangle.wrap.length !== 2 ||
      !triangle.wrap.every((w) => ["clamp", "repeat", "mirror"].includes(w)) ||
      (triangle.doubleSided !== undefined && typeof triangle.doubleSided !== "boolean") ||
      (triangle.texture !== undefined &&
        (!Number.isSafeInteger(triangle.texture) ||
          triangle.texture < 0 ||
          triangle.texture >= textures.length))
    )
      throw new Error("Invalid mask triangle alpha sampling");
  }
  const wrap = (value: number, mode: MaskTextureWrap) => {
    if (mode === "clamp") return Math.max(0, Math.min(1, value));
    if (mode === "repeat") return value - Math.floor(value);
    const repeat = value - Math.floor(value / 2) * 2;
    return repeat <= 1 ? repeat : 2 - repeat;
  };
  return (index: number, a: number, b: number, c: number) => {
    const rule = coverage.triangles[index]!;
    let alpha = rule.alpha[0] * a + rule.alpha[1] * b + rule.alpha[2] * c;
    if (rule.texture !== undefined) {
      const texture = textures[rule.texture]!;
      const u = wrap(rule.uv[0][0] * a + rule.uv[1][0] * b + rule.uv[2][0] * c, rule.wrap[0]);
      const v = wrap(rule.uv[0][1] * a + rule.uv[1][1] * b + rule.uv[2][1] * c, rule.wrap[1]);
      const x = Math.min(texture.width - 1, Math.floor(u * texture.width));
      const y = Math.min(texture.height - 1, Math.floor(v * texture.height));
      alpha *= texture.alpha[y * texture.width + x]! / 255;
    }
    return alpha >= rule.cutoff;
  };
}
