// GLB loading, pose/camera transforms, map projection, and silhouette
// rasterization used to fit SAM 3D reconstructions into the map's scene frame.
import fs from "node:fs/promises";
import { NodeIO, type Document } from "@gltf-transform/core";
import { ALL_EXTENSIONS } from "@gltf-transform/extensions";
import { mat3, mat4, quat, vec3 } from "gl-matrix";
import sharp from "sharp";
import {
  cameraToSceneMatrix,
  sceneToMap,
  type CameraConvention,
  type MapCamera,
  type ModelPose,
  type Quat,
  type Vec3,
} from "@rle/shared";

export interface TextureData {
  width: number;
  height: number;
  /** RGBA */
  data: Buffer;
}

export interface MeshData {
  /** xyz per vertex, node transforms applied (GLB file frame, Y up) */
  positions: Float32Array;
  indices: Uint32Array;
  /** rgb 0..1 per vertex, if present */
  colors: Float32Array | null;
  uvs: Float32Array | null;
  /** per-triangle index into `textures`, -1 for untextured */
  triTexture: Int32Array;
  textures: TextureData[];
}

let io: NodeIO | null = null;
async function readDocument(file: string): Promise<Document> {
  if (!io) io = new NodeIO().registerExtensions(ALL_EXTENSIONS);
  return io.read(file);
}

export async function loadGlb(file: string): Promise<MeshData> {
  const doc = await readDocument(file);
  const root = doc.getRoot();
  const scene = root.getDefaultScene() ?? root.listScenes()[0];
  if (!scene) throw new Error(`${file}: no scene`);

  const positions: number[] = [];
  const indices: number[] = [];
  const colors: number[] = [];
  const uvs: number[] = [];
  const triTexture: number[] = [];
  const textures: TextureData[] = [];
  const textureIndex = new Map<object, number>();
  let anyColor = false;
  let anyUv = false;

  const nodes: { world: mat4; node: ReturnType<typeof root.listNodes>[number] }[] = [];
  scene.traverse((node) => {
    nodes.push({ world: node.getWorldMatrix() as unknown as mat4, node });
  });

  for (const { world, node } of nodes) {
    const mesh = node.getMesh();
    if (!mesh) continue;
    for (const prim of mesh.listPrimitives()) {
      if (prim.getMode() !== 4) {
        throw new Error(`${file}: unsupported primitive mode ${prim.getMode()} (want TRIANGLES)`);
      }
      const pos = prim.getAttribute("POSITION");
      if (!pos) throw new Error(`${file}: primitive without POSITION`);
      const base = positions.length / 3;
      const n = pos.getCount();
      const tmp: number[] = [0, 0, 0];
      const v = vec3.create();
      for (let i = 0; i < n; i++) {
        pos.getElement(i, tmp);
        vec3.set(v, tmp[0]!, tmp[1]!, tmp[2]!);
        vec3.transformMat4(v, v, world);
        positions.push(v[0], v[1], v[2]);
      }
      const col = prim.getAttribute("COLOR_0");
      if (col) {
        anyColor = true;
        const c: number[] = [0, 0, 0, 0];
        for (let i = 0; i < n; i++) {
          col.getElement(i, c);
          colors.push(c[0]!, c[1]!, c[2]!);
        }
      } else {
        for (let i = 0; i < n; i++) colors.push(1, 1, 1);
      }
      const uv = prim.getAttribute("TEXCOORD_0");
      if (uv) {
        anyUv = true;
        const t: number[] = [0, 0];
        for (let i = 0; i < n; i++) {
          uv.getElement(i, t);
          uvs.push(t[0]!, t[1]!);
        }
      } else {
        for (let i = 0; i < n; i++) uvs.push(0, 0);
      }

      let texIdx = -1;
      const tex = prim.getMaterial()?.getBaseColorTexture();
      if (tex) {
        const cached = textureIndex.get(tex);
        if (cached !== undefined) texIdx = cached;
        else {
          const img = tex.getImage();
          if (img) {
            const { data, info } = await sharp(Buffer.from(img))
              .ensureAlpha()
              .raw()
              .toBuffer({ resolveWithObject: true });
            texIdx = textures.length;
            textures.push({ width: info.width, height: info.height, data });
            textureIndex.set(tex, texIdx);
          }
        }
      }

      const idx = prim.getIndices();
      const triCount = idx ? idx.getCount() / 3 : n / 3;
      if (idx) {
        const arr = idx.getArray()!;
        for (let i = 0; i < arr.length; i++) indices.push(base + arr[i]!);
      } else {
        for (let i = 0; i < n; i++) indices.push(base + i);
      }
      for (let i = 0; i < triCount; i++) triTexture.push(texIdx);
    }
  }
  if (positions.length === 0) throw new Error(`${file}: no triangle geometry`);
  return {
    positions: Float32Array.from(positions),
    indices: Uint32Array.from(indices),
    colors: anyColor ? Float32Array.from(colors) : null,
    uvs: anyUv ? Float32Array.from(uvs) : null,
    triTexture: Int32Array.from(triTexture),
    textures,
  };
}

export function bounds(p: Float32Array): { min: Vec3; max: Vec3 } {
  const min: Vec3 = [Infinity, Infinity, Infinity];
  const max: Vec3 = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < p.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      const v = p[i + k]!;
      if (v < min[k]!) min[k] = v;
      if (v > max[k]!) max[k] = v;
    }
  }
  return { min, max };
}

/** rigid + uniform-scale transform: out = R * (s * p) + t */
export function transformPositions(
  p: Float32Array,
  rotation: Quat,
  scale: number,
  translation: Vec3,
): Float32Array {
  const r = mat3.fromQuat(mat3.create(), quat.fromValues(...rotation));
  const out = new Float32Array(p.length);
  const v = vec3.create();
  for (let i = 0; i < p.length; i += 3) {
    vec3.set(v, p[i]! * scale, p[i + 1]! * scale, p[i + 2]! * scale);
    vec3.transformMat3(v, v, r);
    out[i] = v[0] + translation[0];
    out[i + 1] = v[1] + translation[1];
    out[i + 2] = v[2] + translation[2];
  }
  return out;
}

/** compose two (rotation, uniform scale, translation) transforms: outer ∘ inner */
export function composeTransforms(
  inner: { rotation: Quat; scale: number; translation: Vec3 },
  outer: { rotation: Quat; scale: number; translation: Vec3 },
): { rotation: Quat; scale: number; translation: Vec3 } {
  const qi = quat.fromValues(...inner.rotation);
  const qo = quat.fromValues(...outer.rotation);
  const q = quat.normalize(quat.create(), quat.multiply(quat.create(), qo, qi));
  const t = vec3.fromValues(...inner.translation);
  vec3.scale(t, t, outer.scale);
  vec3.transformQuat(t, t, qo);
  vec3.add(t, t, vec3.fromValues(...outer.translation));
  return {
    rotation: [q[0], q[1], q[2], q[3]],
    scale: inner.scale * outer.scale,
    translation: [t[0], t[1], t[2]],
  };
}

/** row-major 3x3 -> quaternion (matrix must be a proper rotation) */
export function mat3ToQuat(m: number[]): Quat {
  // gl-matrix is column-major; transpose on the way in
  const cm = mat3.fromValues(m[0]!, m[3]!, m[6]!, m[1]!, m[4]!, m[7]!, m[2]!, m[5]!, m[8]!);
  const q = quat.normalize(quat.create(), quat.fromMat3(quat.create(), cm));
  return [q[0], q[1], q[2], q[3]];
}

export function uniformScaleOf(pose: ModelPose): number {
  const [sx, sy, sz] = pose.scale;
  if (Math.abs(sx - sy) > 1e-4 * sx || Math.abs(sx - sz) > 1e-4 * sx) {
    throw new Error(`SAM 3D pose scale is not uniform: ${pose.scale.join(", ")}`);
  }
  return sx;
}

/** camera-frame rotation of a SAM 3D pose expressed in the scene frame */
export function poseToSceneRotation(
  cam: MapCamera,
  convention: CameraConvention,
  rotation: Quat,
): Quat {
  const m = cameraToSceneMatrix(cam, convention);
  return composeTransforms(
    { rotation, scale: 1, translation: [0, 0, 0] },
    { rotation: mat3ToQuat(m), scale: 1, translation: [0, 0, 0] },
  ).rotation;
}

/** project scene-frame positions to map pixels, xy interleaved */
export function projectToMap(cam: MapCamera, p: Float32Array): Float32Array {
  const out = new Float32Array((p.length / 3) * 2);
  for (let i = 0, j = 0; i < p.length; i += 3, j += 2) {
    const [x, y] = sceneToMap(cam, [p[i]!, p[i + 1]!, p[i + 2]!]);
    out[j] = x;
    out[j + 1] = y;
  }
  return out;
}

export interface Grid {
  data: Uint8Array;
  width: number;
  height: number;
}

/**
 * Rasterize projected triangles into a coverage grid over map rect
 * [x, y, w, h] at `step` map pixels per cell.
 */
export function rasterizeSilhouette(
  pts: Float32Array,
  indices: Uint32Array,
  rect: [number, number, number, number],
  step: number,
): Grid {
  const [rx, ry, rw, rh] = rect;
  const width = Math.ceil(rw / step);
  const height = Math.ceil(rh / step);
  const data = new Uint8Array(width * height);
  for (let t = 0; t < indices.length; t += 3) {
    const a = indices[t]! * 2,
      b = indices[t + 1]! * 2,
      c = indices[t + 2]! * 2;
    const ax = (pts[a]! - rx) / step,
      ay = (pts[a + 1]! - ry) / step;
    const bx = (pts[b]! - rx) / step,
      by = (pts[b + 1]! - ry) / step;
    const cx = (pts[c]! - rx) / step,
      cy = (pts[c + 1]! - ry) / step;
    const minX = Math.max(0, Math.floor(Math.min(ax, bx, cx)));
    const maxX = Math.min(width - 1, Math.ceil(Math.max(ax, bx, cx)));
    const minY = Math.max(0, Math.floor(Math.min(ay, by, cy)));
    const maxY = Math.min(height - 1, Math.ceil(Math.max(ay, by, cy)));
    if (minX > maxX || minY > maxY) continue;
    const area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    if (Math.abs(area) < 1e-12) continue;
    for (let y = minY; y <= maxY; y++) {
      const py = y + 0.5;
      for (let x = minX; x <= maxX; x++) {
        const px = x + 0.5;
        const w0 = (bx - px) * (cy - py) - (cx - px) * (by - py);
        const w1 = (cx - px) * (ay - py) - (ax - px) * (cy - py);
        const w2 = (ax - px) * (by - py) - (bx - px) * (ay - py);
        if (
          (w0 >= 0 && w1 >= 0 && w2 >= 0 && area > 0) ||
          (w0 <= 0 && w1 <= 0 && w2 <= 0 && area < 0)
        ) {
          data[y * width + x] = 1;
        }
      }
    }
  }
  return { data, width, height };
}

/** downsample a full-res mask (map-rect aligned) onto the same grid as rasterizeSilhouette */
export function maskToGrid(
  mask: { data: Uint8Array; width: number; height: number },
  step: number,
): Grid {
  const width = Math.ceil(mask.width / step);
  const height = Math.ceil(mask.height / step);
  const data = new Uint8Array(width * height);
  for (let y = 0; y < mask.height; y++) {
    const gy = Math.floor(y / step);
    for (let x = 0; x < mask.width; x++) {
      if (mask.data[y * mask.width + x]) data[gy * width + Math.floor(x / step)] = 1;
    }
  }
  return { data, width, height };
}

export function gridIou(a: Grid, b: Grid): number {
  if (a.width !== b.width || a.height !== b.height) {
    throw new Error(`grid size mismatch ${a.width}x${a.height} vs ${b.width}x${b.height}`);
  }
  let inter = 0,
    union = 0;
  for (let i = 0; i < a.data.length; i++) {
    const x = a.data[i]!,
      y = b.data[i]!;
    if (x && y) inter++;
    if (x || y) union++;
  }
  return union === 0 ? 0 : inter / union;
}

export function bbox2d(pts: Float32Array): [number, number, number, number] {
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (let i = 0; i < pts.length; i += 2) {
    const x = pts[i]!,
      y = pts[i + 1]!;
    if (x < x0) x0 = x;
    if (x > x1) x1 = x;
    if (y < y0) y0 = y;
    if (y > y1) y1 = y;
  }
  return [x0, y0, x1, y1];
}

export async function fileExists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

export interface ColorGrid {
  /** rgb per cell */
  rgb: Uint8Array;
  /** 1 where a triangle was drawn */
  covered: Uint8Array;
  width: number;
  height: number;
}

/**
 * Unlit, depth-tested raster of the placed model as seen by the map camera,
 * over map rect [x, y, w, h] at `step` map pixels per cell. Textured
 * triangles sample the texture (nearest), others use vertex colors.
 */
export function rasterizeAppearance(
  positions: Float32Array,
  cam: MapCamera,
  mesh: MeshData,
  rect: [number, number, number, number],
  step: number,
): ColorGrid {
  const [rx, ry, rw, rh] = rect;
  const width = Math.ceil(rw / step);
  const height = Math.ceil(rh / step);
  const rgb = new Uint8Array(width * height * 3);
  const covered = new Uint8Array(width * height);
  const depth = new Float32Array(width * height).fill(Infinity);
  const pts = projectToMap(cam, positions);
  // camera forward in the scene frame: (0, cos t, -sin t); larger = farther
  const t = (cam.elevation_deg * Math.PI) / 180;
  const n = positions.length / 3;
  const d = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    d[i] = positions[i * 3 + 1]! * Math.cos(t) - positions[i * 3 + 2]! * Math.sin(t);
  }
  const uvs = mesh.uvs;
  const cols = mesh.colors;
  for (let tri = 0; tri < mesh.indices.length; tri += 3) {
    const ia = mesh.indices[tri]!,
      ib = mesh.indices[tri + 1]!,
      ic = mesh.indices[tri + 2]!;
    const ax = (pts[ia * 2]! - rx) / step,
      ay = (pts[ia * 2 + 1]! - ry) / step;
    const bx = (pts[ib * 2]! - rx) / step,
      by = (pts[ib * 2 + 1]! - ry) / step;
    const cx = (pts[ic * 2]! - rx) / step,
      cy = (pts[ic * 2 + 1]! - ry) / step;
    const minX = Math.max(0, Math.floor(Math.min(ax, bx, cx)));
    const maxX = Math.min(width - 1, Math.ceil(Math.max(ax, bx, cx)));
    const minY = Math.max(0, Math.floor(Math.min(ay, by, cy)));
    const maxY = Math.min(height - 1, Math.ceil(Math.max(ay, by, cy)));
    if (minX > maxX || minY > maxY) continue;
    const area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
    if (Math.abs(area) < 1e-12) continue;
    const inv = 1 / area;
    const texIdx = mesh.triTexture[tri / 3]!;
    const tex = texIdx >= 0 ? mesh.textures[texIdx]! : null;
    for (let y = minY; y <= maxY; y++) {
      const py = y + 0.5;
      for (let x = minX; x <= maxX; x++) {
        const px = x + 0.5;
        const l0 = ((bx - px) * (cy - py) - (cx - px) * (by - py)) * inv;
        const l1 = ((cx - px) * (ay - py) - (ax - px) * (cy - py)) * inv;
        const l2 = 1 - l0 - l1;
        if (l0 < 0 || l1 < 0 || l2 < 0) continue;
        const z = l0 * d[ia]! + l1 * d[ib]! + l2 * d[ic]!;
        const cell = y * width + x;
        if (z >= depth[cell]!) continue;
        depth[cell] = z;
        covered[cell] = 1;
        let r: number, g: number, b: number;
        if (tex && uvs) {
          const u = l0 * uvs[ia * 2]! + l1 * uvs[ib * 2]! + l2 * uvs[ic * 2]!;
          const v = l0 * uvs[ia * 2 + 1]! + l1 * uvs[ib * 2 + 1]! + l2 * uvs[ic * 2 + 1]!;
          const tx = Math.min(tex.width - 1, Math.max(0, Math.floor((u - Math.floor(u)) * tex.width)));
          const ty = Math.min(tex.height - 1, Math.max(0, Math.floor((v - Math.floor(v)) * tex.height)));
          const o = (ty * tex.width + tx) * 4;
          r = tex.data[o]!;
          g = tex.data[o + 1]!;
          b = tex.data[o + 2]!;
        } else if (cols) {
          r = 255 * (l0 * cols[ia * 3]! + l1 * cols[ib * 3]! + l2 * cols[ic * 3]!);
          g = 255 * (l0 * cols[ia * 3 + 1]! + l1 * cols[ib * 3 + 1]! + l2 * cols[ic * 3 + 1]!);
          b = 255 * (l0 * cols[ia * 3 + 2]! + l1 * cols[ib * 3 + 2]! + l2 * cols[ic * 3 + 2]!);
        } else {
          r = g = b = 128;
        }
        rgb[cell * 3] = r;
        rgb[cell * 3 + 1] = g;
        rgb[cell * 3 + 2] = b;
      }
    }
  }
  return { rgb, covered, width, height };
}

/**
 * Mean colour agreement (1 = identical) between a rendered model and the
 * reference crop over cells inside the mask that the model covers.
 */
export function appearanceScore(
  rendered: ColorGrid,
  reference: { rgb: Uint8Array; width: number; height: number },
  mask: Grid,
): number {
  if (rendered.width !== reference.width || rendered.width !== mask.width) {
    throw new Error("appearance grids differ in size");
  }
  let sum = 0;
  let n = 0;
  for (let i = 0; i < mask.data.length; i++) {
    if (!mask.data[i] || !rendered.covered[i]) continue;
    const dr = Math.abs(rendered.rgb[i * 3]! - reference.rgb[i * 3]!);
    const dg = Math.abs(rendered.rgb[i * 3 + 1]! - reference.rgb[i * 3 + 1]!);
    const db = Math.abs(rendered.rgb[i * 3 + 2]! - reference.rgb[i * 3 + 2]!);
    sum += 1 - (dr + dg + db) / 765;
    n++;
  }
  return n === 0 ? 0 : sum / n;
}
