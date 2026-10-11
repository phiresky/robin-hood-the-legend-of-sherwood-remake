import assert from "node:assert/strict";
import test from "node:test";
import { prepareSightCap } from "./prepare-sight-cap.ts";
import { heightPlane } from "./gameplay-plane.ts";
import type { SightObstacle } from "./level.ts";
import type { Vec3 } from "./scene.ts";

test("clipped planar solids retain their contour and heights with stable native cap vertices", () => {
  const points: SightObstacle["points"] = [
    [163.90489196777344, 558.7142364340918],
    [163.904891962602, 558.7142364340918],
    [163.86924458697567, 558.6564909205791],
    [164.12292478739542, 558.5005105778373],
    [164.12292479393236, 558.5005105777475],
  ].map(([x, y]) => ({ x: x!, y: y!, z_top: x! + y!, z_bottom: x! + y! - 2 }));
  const before = structuredClone(points);
  const prepared = prepareSightCap(points);
  assert.deepEqual(points, before);
  assert.equal(prepared.length, points.length);
  const offset = points.indexOf(prepared[0]!);
  assert.ok(offset > 0);
  assert.deepEqual(prepared, [...points.slice(offset), ...points.slice(0, offset)]);
  for (const field of ["z_top", "z_bottom"] as const) {
    const vertices = prepared
      .slice(0, 3)
      .map((point): Vec3 => [point.x, point.y, point[field]].map(Math.fround) as Vec3);
    assert.doesNotThrow(() => heightPlane(vertices));
  }
  assert.equal(prepareSightCap(prepared), prepared);
});

test("unusable or warped caps require authored repair", () => {
  const point = (x: number, y: number, z_top = 2) => ({ x, y, z_top, z_bottom: 0 });
  assert.throws(() => prepareSightCap([point(0, 0), point(1, 0), point(2, 0)]), /stable/);
  assert.throws(
    () => prepareSightCap([point(0, 0), point(0, 0), point(10, 0), point(10, 10), point(0, 10, 3)]),
    /planar/,
  );
});
