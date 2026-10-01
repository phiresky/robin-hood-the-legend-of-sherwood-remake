import { terrainTriangles, type Level3D, type Vec3 } from "@rle/shared";

/** Intersect evaluated terrain (including spline shaping) at regular game-height intervals. */
export function terrainContours(document: Level3D, interval = 32): [Vec3, Vec3][] {
  if (!Number.isFinite(interval) || interval <= 0)
    throw new Error("Contour interval must be positive and finite");
  const segments = new Map<string, [Vec3, Vec3]>();
  const key = (p: Vec3) => p.map((v) => Math.round(v * 1e6)).join(",");
  for (const { points } of terrainTriangles(document)) {
    const min = Math.min(...points.map((p) => p[2]));
    const max = Math.max(...points.map((p) => p[2]));
    // Flat faces have no contour direction; adjacent slopes supply plateau edges.
    if (min === max) continue;
    for (let level = Math.ceil(min / interval); level <= Math.floor(max / interval); level++) {
      const height = level * interval;
      const hits = new Map<string, Vec3>();
      for (let i = 0; i < 3; i++) {
        const a = points[i]!,
          b = points[(i + 1) % 3]!;
        if (a[2] === height) hits.set(key(a), a);
        if ((a[2] < height && b[2] > height) || (a[2] > height && b[2] < height)) {
          const t = (height - a[2]) / (b[2] - a[2]);
          const p: Vec3 = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), height];
          hits.set(key(p), p);
        }
      }
      if (hits.size !== 2) continue;
      const [a, b] = [...hits.values()] as [Vec3, Vec3];
      segments.set([key(a), key(b)].sort().join(";"), [a, b]);
    }
  }
  return [...segments.values()];
}
