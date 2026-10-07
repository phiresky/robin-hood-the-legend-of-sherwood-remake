import assert from "node:assert/strict";
import test from "node:test";
import { MeshoptSimplifier } from "meshoptimizer";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { simplifyPhysicalShell } from "./simplify-physical-shell.ts";
import { closedMeshComponents } from "./closed-mesh-components.ts";

function tetra(): MaskTriangle[] {
  const a: Vec3 = [0, 0, 0],
    b: Vec3 = [3, 0, 0],
    c: Vec3 = [0, 3, 0],
    d: Vec3 = [0, 0, 3];
  return [
    [a, c, b],
    [a, b, d],
    [a, d, c],
    [b, c, d],
  ];
}

test("removes coplanar tessellation while retaining a closed physical shell", async () => {
  const mesh = tetra().flatMap(([a, b, c]): MaskTriangle[] => {
    const p: Vec3 = [(a[0] + b[0] + c[0]) / 3, (a[1] + b[1] + c[1]) / 3, (a[2] + b[2] + c[2]) / 3];
    return [
      [a, b, p],
      [b, c, p],
      [c, a, p],
    ];
  });
  const result = await simplifyPhysicalShell(mesh, 0.01);
  assert.ok(result.triangles.length < mesh.length);
  assert.equal(result.sourceTriangles, 12);
  assert.ok(result.error <= 0.01);
  assert.equal(closedMeshComponents(result.triangles).length, 1);
  assert.deepEqual(
    new Set(result.triangles.flat().map((p) => JSON.stringify(p))),
    new Set(
      tetra()
        .flat()
        .map((p) => JSON.stringify(p)),
    ),
  );
});

test("rejects open geometry, mixed shells and invalid simplification budgets", async () => {
  const mesh = tetra();
  await assert.rejects(
    simplifyPhysicalShell(mesh.slice(1), 0.1),
    /Physical simplification input is invalid:.*incident faces/,
  );
  const moved = mesh.map((triangle): MaskTriangle => {
    const p = triangle.map(([x, y, z]): Vec3 => [x + 10, y, z]);
    return [p[0]!, p[1]!, p[2]!];
  });
  await assert.rejects(simplifyPhysicalShell([...mesh, ...moved], 0.1), /one closed shell/);
  await assert.rejects(simplifyPhysicalShell(mesh, -1), /error budget/);
  await assert.rejects(simplifyPhysicalShell(mesh, NaN), /error budget/);
});

test("rejects invalid simplifier output with stage and precision diagnostics", async (t) => {
  t.mock.method(MeshoptSimplifier, "simplify", (indices: Uint32Array) => [
    new Uint32Array([...indices, ...indices.slice(0, 3)]),
    0,
  ]);
  await assert.rejects(simplifyPhysicalShell(tetra(), 0.1), (error: unknown) => {
    assert.ok(error instanceof Error);
    assert.match(error.message, /Simplified physical shell is invalid \(5 triangles/);
    assert.match(error.message, /approximate error 0, 0 coincident float32 positions/);
    assert.ok(error.cause instanceof Error);
    assert.match(error.cause.message, /3 incident faces/);
    return true;
  });
});
