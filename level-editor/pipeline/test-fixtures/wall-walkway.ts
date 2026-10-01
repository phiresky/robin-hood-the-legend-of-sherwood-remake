import { authorWallWalkway } from "../src/author-wall-walkway.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { wallSplineFixture } from "../../shared/test-fixtures/wall-spline.ts";

export function walkwayFixture(offset = 0) {
  const triangles: [Vec3, Vec3, Vec3][] = [];
  function cap(x0: number, x1: number, y0: number, y1: number, z: number) {
    const a: Vec3 = [x0, y0, z],
      b: Vec3 = [x1, y0, z],
      c: Vec3 = [x1, y1, z],
      d: Vec3 = [x0, y1, z];
    triangles.push([a, b, c], [a, c, d]);
  }
  cap(-50, 50, -10, 10, 0);
  // Many tiny mesh faces must not turn into a matching amount of gameplay metadata.
  for (let i = 0; i < 64; i++) cap(-50 + (100 * i) / 64, -50 + (100 * (i + 1)) / 64, -10, 10, 40);
  cap(-50, 50, -10, -3, 46);
  cap(-30, -10, -10, -3, 60);
  const f = wallSplineFixture();
  f.document.splines![0]!.width = 80;
  const shift = (p: Vec3): Vec3 => [p[0] + offset, p[1], p[2]];
  f.asset.gameplay = authorWallWalkway(
    triangles.map(([a, b, c]) => [shift(a), shift(b), shift(c)]),
    "body",
    { material: 2, opaque: true, walkwayHeight: 40 },
  );
  return f;
}
