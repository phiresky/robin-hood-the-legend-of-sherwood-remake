import type { MaskTriangle } from "../../shared/src/compile-mask-geometry.ts";
import type { Vec3 } from "../../shared/src/scene.ts";

/** Prepare separate closed shells for physical volume authoring. Texture seams
 * may duplicate vertices, but open surfaces must not silently become solids.
 * Connectivity follows edges, so pieces touching at a point stay separate. */
export function closedMeshComponents(
  triangles: readonly MaskTriangle[],
  tolerance = 1e-5,
  onCollinearFace?: (face: number) => void,
): MaskTriangle[][] {
  if (!Number.isFinite(tolerance) || tolerance <= 0 || triangles.length === 0)
    throw new Error("Closed mesh requires triangles and a positive weld tolerance");
  const vertices: Vec3[] = [];
  const buckets = new Map<string, number[]>();
  const vertex = (point: Vec3): number => {
    if (point.length !== 3 || !point.every(Number.isFinite))
      throw new Error("Nonfinite physical mesh vertex");
    const [x, y, z] = point.map((v) => Math.floor(v / tolerance));
    for (let dx = -1; dx <= 1; dx++)
      for (let dy = -1; dy <= 1; dy++)
        for (let dz = -1; dz <= 1; dz++)
          for (const id of buckets.get(`${x! + dx},${y! + dy},${z! + dz}`) ?? []) {
            const p = vertices[id]!;
            if (Math.hypot(p[0] - point[0], p[1] - point[1], p[2] - point[2]) <= tolerance)
              return id;
          }
    const id = vertices.length;
    vertices.push([...point]);
    const key = `${x},${y},${z}`;
    const bucket = buckets.get(key) ?? [];
    bucket.push(id);
    buckets.set(key, bucket);
    return id;
  };
  const edges = new Map<string, { face: number; forward: boolean }[]>();
  const faces = triangles.map((triangle, face) => {
    const ids = triangle.map(vertex);
    if (new Set(ids).size !== 3) throw new Error(`Collapsed physical mesh triangle ${face}`);
    const [a, b, c] = ids.map((id) => vertices[id]!);
    const u = b!.map((v, i) => v - a![i]!);
    const v = c!.map((value, i) => value - a![i]!);
    if (
      Math.hypot(
        u[1]! * v[2]! - u[2]! * v[1]!,
        u[2]! * v[0]! - u[0]! * v[2]!,
        u[0]! * v[1]! - u[1]! * v[0]!,
      ) <=
      tolerance * tolerance
    ) {
      if (!onCollinearFace) throw new Error(`Degenerate physical mesh triangle ${face}`);
      // A collinear face can join a subdivided edge to an unsplit neighbour.
      // Retain its edges for the closed-shell check, but require callers to
      // explicitly record this exception. Repeated vertices still reject.
      onCollinearFace(face);
    }
    for (let i = 0; i < 3; i++) {
      const from = ids[i]!;
      const to = ids[(i + 1) % 3]!;
      const key = `${Math.min(from, to)},${Math.max(from, to)}`;
      const uses = edges.get(key) ?? [];
      uses.push({ face, forward: from < to });
      edges.set(key, uses);
    }
    return ids;
  });
  const neighbours = faces.map(() => new Set<number>());
  for (const [edge, uses] of edges) {
    if (uses.length !== 2)
      throw new Error(`Physical mesh edge ${edge} has ${uses.length} incident faces; expected 2`);
    const [a, b] = uses;
    if (a!.forward === b!.forward)
      throw new Error(`Inconsistent physical mesh winding at edge ${edge}`);
    neighbours[a!.face]!.add(b!.face);
    neighbours[b!.face]!.add(a!.face);
  }
  const visited = new Set<number>();
  const components: MaskTriangle[][] = [];
  for (let start = 0; start < faces.length; start++) {
    if (visited.has(start)) continue;
    visited.add(start);
    const pending = [start];
    const component: MaskTriangle[] = [];
    while (pending.length) {
      const face = pending.pop()!;
      const ids = faces[face]!;
      component.push([vertices[ids[0]!]!, vertices[ids[1]!]!, vertices[ids[2]!]!]);
      for (const next of neighbours[face]!) {
        if (visited.has(next)) continue;
        visited.add(next);
        pending.push(next);
      }
    }
    components.push(component);
  }
  // This checks topology, not self-intersection or containment. Nested shells
  // and intersecting faces require geometric validation before volume export.
  return components;
}
