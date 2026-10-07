import { MeshoptSimplifier } from "meshoptimizer";
import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { Vec3 } from "../../shared/src/scene.ts";
import { closedMeshComponents } from "./closed-mesh-components.ts";

/** Offline physical-mesh candidate. The reported simplifier error is an
 * approximate appearance metric, not a certified maximum contact displacement.
 * Preserve shells independently; never prune a small rail or fill between posts. */
export async function simplifyPhysicalShell(
  mesh: readonly MaskTriangle[],
  maximumError: number,
  onCollinearFace?: (face: number) => void,
) {
  if (!Number.isFinite(maximumError) || maximumError < 0)
    throw new Error("Physical simplification needs a finite nonnegative error budget");
  let components: MaskTriangle[][];
  try {
    components = closedMeshComponents(mesh, 1e-5, onCollinearFace);
  } catch (error) {
    throw new Error(`Physical simplification input is invalid: ${String(error)}`, { cause: error });
  }
  if (components.length !== 1) throw new Error("Physical simplification requires one closed shell");
  const vertices: Vec3[] = [];
  const lookup = new Map<Vec3, number>();
  const indices = new Uint32Array(
    components[0]!.flatMap((triangle) =>
      triangle.map((point) => {
        const existing = lookup.get(point);
        if (existing !== undefined) return existing;
        const id = vertices.length;
        vertices.push(point);
        lookup.set(point, id);
        return id;
      }),
    ),
  );
  await MeshoptSimplifier.ready;
  const [simplified, error] = MeshoptSimplifier.simplify(
    indices,
    new Float32Array(vertices.flat()),
    3,
    12,
    maximumError,
    ["ErrorAbsolute", "LockBorder"],
  );
  if (!Number.isFinite(error) || error > maximumError + 1e-6)
    throw new Error("Physical simplifier exceeded its error budget");
  const triangles: MaskTriangle[] = [];
  for (let i = 0; i < simplified.length; i += 3)
    triangles.push([
      vertices[simplified[i]!]!,
      vertices[simplified[i + 1]!]!,
      vertices[simplified[i + 2]!]!,
    ]);
  // The simplifier chooses indices only. Retain the original double-precision
  // positions, then validate that this exact output is still one closed shell.
  try {
    if (closedMeshComponents(triangles).length !== 1)
      throw new Error("Physical simplification split the shell");
  } catch (cause) {
    const distinctPositions = new Set(vertices.map((p) => p.map(Math.fround).join(",")));
    throw new Error(
      `Simplified physical shell is invalid (${triangles.length} triangles, approximate error ${error}, ${vertices.length - distinctPositions.size} coincident float32 positions): ${String(cause)}`,
      { cause },
    );
  }
  return { triangles, error, sourceTriangles: mesh.length };
}
