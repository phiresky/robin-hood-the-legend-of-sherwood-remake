import assert from "node:assert/strict";
import test from "node:test";
import { closedMeshComponents } from "./closed-mesh-components.ts";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

function tetra(offset: Vec3 = [0, 0, 0]): MaskTriangle[] {
  const points: Vec3[] = [
    [0, 0, 0],
    [1, 0, 0],
    [0, 1, 0],
    [0, 0, 1],
  ];
  const faces = [
    [0, 2, 1],
    [0, 1, 3],
    [0, 3, 2],
    [1, 2, 3],
  ];
  return faces.map(
    ([a, b, c]) =>
      [a!, b!, c!].map((id) => {
        const p = points[id]!;
        return [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]];
      }) as MaskTriangle,
  );
}

test("seam-duplicated closed pieces remain separate across visible gaps and point contacts", () => {
  assert.deepEqual(
    closedMeshComponents([...tetra(), ...tetra([3, 0, 0])]).map((c) => c.length),
    [4, 4],
  );
  assert.deepEqual(
    closedMeshComponents([...tetra(), ...tetra([1, 0, 0])]).map((c) => c.length),
    [4, 4],
  );
});

test("welds nearby seam vertices across spatial bucket boundaries", () => {
  const mesh = tetra();
  mesh[0]![0][0] -= 1e-7;
  assert.equal(closedMeshComponents(mesh).length, 1);
});

test("rejects open, nonmanifold, reversed and degenerate surfaces instead of inventing solids", () => {
  const mesh = tetra();
  assert.throws(() => closedMeshComponents(mesh.slice(1)), /incident faces/);
  assert.throws(() => closedMeshComponents([...mesh, mesh[0]!]), /incident faces/);
  const reversed = tetra();
  reversed[0] = [reversed[0]![2], reversed[0]![1], reversed[0]![0]];
  assert.throws(() => closedMeshComponents(reversed), /winding/);
  assert.throws(
    () =>
      closedMeshComponents([
        [
          [0, 0, 0],
          [1, 0, 0],
          [2, 0, 0],
        ],
      ]),
    /Degenerate/,
  );
  assert.throws(() => closedMeshComponents([]), /requires triangles/);
  assert.throws(() => closedMeshComponents(mesh, NaN), /tolerance/);
});
