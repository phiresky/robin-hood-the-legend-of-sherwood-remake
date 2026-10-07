import assert from "node:assert/strict";
import test from "node:test";
import earcut from "earcut";
import { meshCappedVolumes } from "./mesh-capped-volumes.ts";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { Point } from "../../shared/src/level.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

test("rejects a closed shell whose cap crosses through its floor", () => {
  const mesh = extrude([
    [0, 0],
    [3, 0],
    [3, 3],
    [0, 3],
  ]);
  for (const point of new Set(mesh.flat()))
    if (point[0] === 3 && point[1] === 3 && point[2] === 1) point[2] = -1;
  assert.throws(() => meshCappedVolumes(mesh), /intersecting cap faces/);
});

function extrude(ring: Point[], slope = 0): MaskTriangle[] {
  const low = ring.map(([x, y]): Vec3 => [x, y, slope * x]);
  const high = ring.map(([x, y]): Vec3 => [x, y, 1 + slope * x]);
  const indices = earcut(ring.flat());
  const triangles: MaskTriangle[] = [];
  for (let i = 0; i < indices.length; i += 3) {
    const a = indices[i]!,
      b = indices[i + 1]!,
      c = indices[i + 2]!;
    triangles.push([low[c]!, low[b]!, low[a]!], [high[a]!, high[b]!, high[c]!]);
  }
  for (let i = 0; i < ring.length; i++) {
    const j = (i + 1) % ring.length;
    triangles.push([low[i]!, low[j]!, high[j]!], [low[i]!, high[j]!, high[i]!]);
  }
  return triangles;
}

test("capped volumes preserve sloped floors and roofs under reversed winding and rotation", () => {
  const mesh = extrude(
    [
      [0, 0],
      [3, 0],
      [3, 2],
      [0, 2],
    ],
    0.75,
  );
  for (const reverse of [false, true]) {
    const angle = 0.61,
      c = Math.cos(angle),
      s = Math.sin(angle);
    const placed = mesh.map((triangle): MaskTriangle => {
      const points = triangle.map(([x, y, z]): Vec3 => [
        c * x - s * y + 120,
        s * x + c * y - 80,
        z + 40,
      ]);
      return reverse ? [points[2]!, points[1]!, points[0]!] : [points[0]!, points[1]!, points[2]!];
    });
    const volumes = meshCappedVolumes(placed);
    assert.ok(volumes.length > 0);
    for (const point of volumes.flat()) {
      const x = c * (point.x - 120) + s * (point.y + 80);
      assert.ok(Math.abs(point.z_bottom - (40 + 0.75 * x)) < 1e-8);
      assert.ok(Math.abs(point.z_top - (41 + 0.75 * x)) < 1e-8);
    }
  }
});

test("concave footprint retains its empty corner instead of filling its convex hull", () => {
  const volumes = meshCappedVolumes(
    extrude([
      [0, 0],
      [3, 0],
      [3, 1],
      [1, 1],
      [1, 3],
      [0, 3],
    ]),
  );
  for (const polygon of volumes) {
    const x = polygon.reduce((sum, p) => sum + p.x, 0) / polygon.length;
    const y = polygon.reduce((sum, p) => sum + p.y, 0) / polygon.length;
    assert.ok(x <= 1 || y <= 1);
  }
  const area = volumes.reduce(
    (sum, polygon) =>
      sum +
      polygon.reduce((a, p, i) => {
        const q = polygon[(i + 1) % polygon.length]!;
        return a + p.x * q.y - q.x * p.y;
      }, 0) /
        2,
    0,
  );
  assert.ok(Math.abs(area - 5) < 1e-8);
});

test("preserves multiple vertical intervals and requires separate shells to be processed independently", () => {
  const cShape = extrude([
    [0, 0],
    [3, 0],
    [3, 1],
    [1, 1],
    [1, 2],
    [3, 2],
    [3, 3],
    [0, 3],
  ]);
  const sideways = cShape.map((triangle): MaskTriangle => {
    const p = triangle.map(([x, y, z]): Vec3 => [x, z, y]);
    return [p[0]!, p[1]!, p[2]!];
  });
  const volumes = meshCappedVolumes(sideways);
  assert.ok(volumes.length > 0);
  for (const polygon of volumes) {
    const x = polygon.reduce((sum, p) => sum + p.x, 0) / polygon.length;
    const bottom = polygon.reduce((sum, p) => sum + p.z_bottom, 0) / polygon.length;
    const top = polygon.reduce((sum, p) => sum + p.z_top, 0) / polygon.length;
    assert.ok(x <= 1 || top <= 1 || bottom >= 2, "air between the rails must remain empty");
  }
  const box = extrude([
    [0, 0],
    [1, 0],
    [1, 1],
    [0, 1],
  ]);
  const moved = box.map((triangle): MaskTriangle => {
    const p = triangle.map(([x, y, z]): Vec3 => [x + 5, y, z]);
    return [p[0]!, p[1]!, p[2]!];
  });
  assert.throws(() => meshCappedVolumes([...box, ...moved]), /one closed shell/);
});
