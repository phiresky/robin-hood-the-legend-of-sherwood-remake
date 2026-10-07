import * as THREE from "three";
export type Triple = readonly [number, number, number];
type ClipPoint = [number, number, number];
export interface SpriteBounds {
  left: number;
  top: number;
  width: number;
  height: number;
}
export interface PhysicalReceiverTriangle {
  id: string;
  points: readonly Triple[];
}
const cross = (a: ClipPoint, b: ClipPoint, p: ClipPoint) =>
  (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
const area = (poly: ClipPoint[]) =>
  poly.reduce((sum, a, i) => {
    const b = poly[(i + 1) % poly.length]!;
    return sum + a[0] * b[1] - b[0] * a[1];
  }, 0) / 2;
function clip(poly: ClipPoint[], a: ClipPoint, b: ClipPoint, sign = 1) {
  const out: ClipPoint[] = [];
  for (let i = 0; i < poly.length; i++) {
    const p = poly[i]!,
      q = poly[(i + 1) % poly.length]!;
    const dp = sign * cross(a, b, p),
      dq = sign * cross(a, b, q);
    if (dp >= 0) out.push(p);
    if (dp >= 0 !== dq >= 0) {
      const t = dp / (dp - dq);
      out.push(p.map((v, k) => v + t * (q[k]! - v)) as ClipPoint);
    }
  }
  return out;
}
function intersection(subject: ClipPoint[], boundary: ClipPoint[]) {
  let result = subject;
  const sign = Math.sign(area(boundary));
  for (let i = 0; i < boundary.length && result.length; i++)
    result = clip(result, boundary[i]!, boundary[(i + 1) % boundary.length]!, sign);
  const same = (a: ClipPoint, b: ClipPoint) =>
    a.every((value, i) => Math.abs(value - b[i]!) <= 1e-10);
  result = result.filter((point, i) => i === 0 || !same(point, result[i - 1]!));
  if (result.length > 1 && same(result[0]!, result[result.length - 1]!)) result.pop();
  return result;
}

/** Receiver vertices are [worldX, worldY-height, height], along native camera rays. */
export function projectShadowReceivers({
  anchor,
  bounds,
  coverage = bounds,
  elevation,
  triangles,
}: {
  anchor: Triple;
  bounds: SpriteBounds;
  coverage?: SpriteBounds;
  elevation: number;
  triangles: readonly (readonly Triple[])[];
}) {
  if (
    !Array.isArray(anchor) ||
    anchor.length !== 3 ||
    !anchor.every(Number.isFinite) ||
    !Number.isFinite(elevation) ||
    elevation <= 0 ||
    elevation >= Math.PI / 2
  )
    throw Error("Invalid projection anchor or elevation");
  const { left, top, width, height } = bounds ?? {};
  if (![left, top, width, height].every(Number.isFinite) || width <= 0 || height <= 0)
    throw Error("Invalid frame bounds");
  if (
    !coverage ||
    ![coverage.left, coverage.top, coverage.width, coverage.height].every(Number.isFinite) ||
    coverage.width <= 0 ||
    coverage.height <= 0 ||
    coverage.left < left ||
    coverage.top > top ||
    coverage.left + coverage.width > left + width ||
    coverage.top - coverage.height < top - height
  )
    throw Error("Shadow coverage escapes its source frame");
  if (!Array.isArray(triangles) || !triangles.length) throw Error("Missing support triangles");
  const sin = Math.sin(elevation),
    cos = Math.cos(elevation),
    baseZ = anchor[1] * cos;
  const mapY = anchor[2] * sin - baseZ;
  const x0 = anchor[0] + coverage.left,
    x1 = x0 + coverage.width,
    y0 = mapY - coverage.top,
    y1 = y0 + coverage.height;
  const footprint: ClipPoint[] = [
    [x0, y0, 0],
    [x1, y0, 0],
    [x1, y1, 0],
    [x0, y1, 0],
  ];
  const expectedArea = coverage.width * coverage.height,
    tolerance = Math.max(1, expectedArea) * 1e-8;
  if (![x0, x1, y0, y1, expectedArea].every(Number.isFinite))
    throw Error("Projection extent overflow");
  const pieces = [];
  for (const [receiver, triangle] of triangles.entries()) {
    if (
      !Array.isArray(triangle) ||
      triangle.length !== 3 ||
      triangle.some((p) => !Array.isArray(p) || p.length !== 3 || !p.every(Number.isFinite))
    )
      throw Error(`Invalid receiver triangle ${receiver}`);
    if (Math.abs(area(triangle)) <= Number.EPSILON)
      throw Error(`Vertical or degenerate receiver triangle ${receiver}`);
    const polygon = intersection(
      triangle.map((p) => [...p] as ClipPoint),
      footprint,
    );
    const covered = Math.abs(area(polygon));
    if (covered > tolerance) pieces.push({ receiver, polygon, area: covered });
  }
  for (let i = 0; i < pieces.length; i++)
    for (let j = 0; j < i; j++) {
      if (Math.abs(area(intersection(pieces[i]!.polygon, pieces[j]!.polygon))) > tolerance)
        throw Error(`Overlapping support receivers ${pieces[j]!.receiver}/${pieces[i]!.receiver}`);
    }
  const coveredArea = pieces.reduce((sum, p) => sum + p.area, 0);
  if (Math.abs(coveredArea - expectedArea) > tolerance)
    throw Error(`Incomplete support coverage: ${coveredArea}/${expectedArea}`);
  const positions = [],
    uvs = [],
    indices = [],
    ranges = [];
  for (const piece of pieces) {
    const firstVertex = positions.length / 3,
      firstIndex = indices.length;
    for (const [x, y, z] of piece.polygon) {
      const localX = x - anchor[0],
        up = mapY - y,
        dz = z - baseZ;
      positions.push(localX, dz / cos + 0.15, (dz - up) / sin);
      uvs.push((localX - left) / width, (up - (top - height)) / height);
    }
    for (let i = 1; i + 1 < piece.polygon.length; i++)
      indices.push(firstVertex, firstVertex + i, firstVertex + i + 1);
    // Each receiver owns separate vertices. A discontinuous height edge is never
    // welded and no generated face connects the high and low receiver surfaces.
    ranges.push({
      receiver: piece.receiver,
      firstVertex,
      vertexCount: piece.polygon.length,
      firstIndex,
      indexCount: indices.length - firstIndex,
    });
  }
  return { positions, uvs, indices, ranges, coveredArea, expectedArea };
}

/** Bound only explicitly decoded shadow alpha, in the shared source-frame coordinates. */
export function shadowCoverageBounds(pixels: Uint8Array | Uint8ClampedArray, bounds: SpriteBounds) {
  const { left, top, width, height } = bounds ?? {};
  if (
    ![left, top, width, height].every(Number.isSafeInteger) ||
    width <= 0 ||
    height <= 0 ||
    !(pixels instanceof Uint8Array || pixels instanceof Uint8ClampedArray) ||
    pixels.length !== width * height * 4
  )
    throw Error("Invalid decoded shadow frame");
  let x0 = width,
    y0 = height,
    x1 = -1,
    y1 = -1;
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++)
      if (pixels[(y * width + x) * 4 + 3]) {
        x0 = Math.min(x0, x);
        y0 = Math.min(y0, y);
        x1 = Math.max(x1, x);
        y1 = Math.max(y1, y);
      }
  return x1 < 0
    ? null
    : { left: left + x0, top: top - y0, width: x1 - x0 + 1, height: y1 - y0 + 1 };
}

/** Convert explicit evaluated game-world terrain triangles to camera-ray support. */
export function projectedTerrainReceivers(
  evaluated: readonly PhysicalReceiverTriangle[],
  footprint: readonly [number, number, number, number],
) {
  if (
    !Array.isArray(evaluated) ||
    !Array.isArray(footprint) ||
    footprint.length !== 4 ||
    !footprint.every(Number.isFinite) ||
    footprint[0] >= footprint[2] ||
    footprint[1] >= footprint[3]
  )
    throw Error("Invalid evaluated terrain or shadow footprint");
  const ids = new Set(),
    selected = [];
  for (const row of evaluated as readonly PhysicalReceiverTriangle[]) {
    if (
      typeof row.id !== "string" ||
      ids.has(row.id) ||
      !Array.isArray(row.points) ||
      row.points.length !== 3 ||
      row.points.some(
        (p: Triple) => !Array.isArray(p) || p.length !== 3 || !p.every(Number.isFinite),
      )
    )
      throw Error("Invalid or duplicate evaluated receiver");
    ids.add(row.id);
    const points = row.points.map(([x, y, z]: Triple): Triple => [x, y - z, z]);
    const xs = points.map((p) => p[0]),
      ys = points.map((p) => p[1]);
    if (
      Math.max(...xs) <= footprint[0] ||
      Math.min(...xs) >= footprint[2] ||
      Math.max(...ys) <= footprint[1] ||
      Math.min(...ys) >= footprint[3]
    )
      continue;
    selected.push({ id: row.id, points });
  }
  // Selection preserves all intersecting evaluated surfaces. The projector must
  // reject overlapping receivers rather than choosing an invented top surface.
  return selected;
}

/** Evaluate explicitly selected physical receiver meshes in the current scene.
 * The caller owns logical receiver membership; visual/contact overlays are not inferred. */
export function evaluatePhysicalReceiver(mesh: THREE.Mesh, owner: string, elevation: number) {
  if (
    !mesh?.isMesh ||
    (mesh as THREE.SkinnedMesh).isSkinnedMesh ||
    (mesh as THREE.InstancedMesh).isInstancedMesh ||
    !owner ||
    typeof owner !== "string" ||
    !Number.isFinite(elevation) ||
    elevation <= 0 ||
    elevation >= Math.PI / 2
  )
    throw Error("An explicit static physical receiver is required");
  const positions = mesh.geometry?.getAttribute("position"),
    index = mesh.geometry?.index;
  if (!positions || positions.itemSize !== 3 || mesh.geometry.morphAttributes.position?.length)
    throw Error("Receiver needs evaluated static triangle positions");
  const count = index?.count ?? positions.count;
  if (count % 3) throw Error("Receiver is not a triangle mesh");
  mesh.updateWorldMatrix(true, false);
  if (
    !mesh.matrixWorld.elements.every(Number.isFinite) ||
    Math.abs(mesh.matrixWorld.determinant()) < 1e-12
  )
    throw Error("Invalid receiver world transform");
  const sin = Math.sin(elevation),
    cos = Math.cos(elevation),
    triangles = [];
  for (let i = 0; i < count; i += 3) {
    const points = [0, 1, 2].map((k) => {
      const vertex = index ? index.getX(i + k) : i + k;
      if (!Number.isSafeInteger(vertex) || vertex < 0 || vertex >= positions.count)
        throw Error("Invalid receiver index");
      const p = new THREE.Vector3()
        .fromBufferAttribute(positions, vertex)
        .applyMatrix4(mesh.matrixWorld);
      if (!p.toArray().every(Number.isFinite)) throw Error("Invalid receiver vertex");
      return p;
    });
    const normal = new THREE.Vector3()
      .subVectors(points[1]!, points[0]!)
      .cross(new THREE.Vector3().subVectors(points[2]!, points[0]!));
    const length = normal.length();
    // A relative tolerance excludes nominally vertical sides despite float32
    // export transforms, while preserving any meaningful upward-facing slope.
    if (length === 0 || normal.y / length <= 1e-5) continue;
    triangles.push({
      id: `${owner}:triangle-${i / 3}`,
      points: points.map((p): Triple => [p.x, p.z * sin, p.y * cos]),
    });
  }
  if (!triangles.length) throw Error(`Receiver ${owner} has no upward physical triangles`);
  return triangles;
}
