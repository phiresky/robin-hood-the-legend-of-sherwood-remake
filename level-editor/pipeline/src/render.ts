// Minimal software rasterizer (orthographic, z-buffer, Lambert shading,
// nearest texture sampling) for review sheets and scene litmus renders.
import { mat3, vec3 } from "gl-matrix";
import { cameraToSceneMatrix, type MapCamera, type Vec3 } from "@rle/shared";
import type { MeshData } from "./mesh.ts";

export interface RenderInstance {
  mesh: MeshData;
  /** scene-frame vertex positions (same layout as mesh.positions) */
  positions: Float32Array;
}

export interface RenderView {
  /** row-major 3x3 scene->view rotation; view x right, y up, z toward the viewer */
  rot: number[];
  /** scene point that lands at the image center */
  center: Vec3;
  width: number;
  height: number;
  pixelsPerUnit: number;
  /** light direction in view space (toward the light) */
  light?: Vec3;
  /**
   * draw texture / vertex colours as they are. SAM 3D bakes the map's own
   * lighting into the texture, so shading it again darkens everything;
   * scene renders use this, geometry review views keep the Lambert term
   */
  unlit?: boolean;
}

/** the map's own camera: 1 scene unit = 1 map pixel, image top-left = map rect origin */
export function mapView(
  cam: MapCamera,
  rect: [number, number, number, number],
  scale = 1,
): RenderView {
  const m = cameraToSceneMatrix(cam, "opengl");
  // scene = M * cam  =>  cam = M^T * scene
  const rot = [m[0]!, m[3]!, m[6]!, m[1]!, m[4]!, m[7]!, m[2]!, m[5]!, m[8]!];
  const [rx, ry, rw, rh] = rect;
  const t = (cam.elevation_deg * Math.PI) / 180;
  // map center pixel on the ground plane
  const cx = rx + rw / 2;
  const cy = ry + rh / 2;
  return {
    rot,
    center: [cx, -cy / Math.sin(t), 0],
    width: Math.round(rw * scale),
    height: Math.round(rh * scale),
    pixelsPerUnit: scale,
    light: [-0.4, 0.8, 0.6],
  };
}

/** camera orbiting `center` from yaw (deg, 0 = looking north/+Y) and pitch above the ground */
export function orbitView(
  center: Vec3,
  yawDeg: number,
  pitchDeg: number,
  width: number,
  height: number,
  pixelsPerUnit: number,
): RenderView {
  const yaw = (yawDeg * Math.PI) / 180;
  const pitch = (pitchDeg * Math.PI) / 180;
  const f = vec3.fromValues(
    Math.cos(pitch) * Math.sin(yaw),
    Math.cos(pitch) * Math.cos(yaw),
    -Math.sin(pitch),
  );
  const up = vec3.fromValues(0, 0, 1);
  const right = vec3.normalize(vec3.create(), vec3.cross(vec3.create(), f, up));
  const trueUp = vec3.cross(vec3.create(), right, f);
  return {
    rot: [right[0], right[1], right[2], trueUp[0], trueUp[1], trueUp[2], -f[0], -f[1], -f[2]],
    center,
    width,
    height,
    pixelsPerUnit,
    light: [-0.4, 0.8, 0.6],
  };
}

/** bright green: texture pixels with alpha 0 are holes left for inpainting */
const HOLE: [number, number, number] = [0, 255, 0];

function sampleTexture(tex: { width: number; height: number; data: Buffer }, u: number, v: number) {
  const x = Math.min(tex.width - 1, Math.max(0, Math.floor((u - Math.floor(u)) * tex.width)));
  const y = Math.min(tex.height - 1, Math.max(0, Math.floor((v - Math.floor(v)) * tex.height)));
  const i = (y * tex.width + x) * 4;
  if (tex.data[i + 3] === 0) return HOLE;
  return [tex.data[i]!, tex.data[i + 1]!, tex.data[i + 2]!];
}

/** render to RGBA (alpha 0 where nothing was drawn) */
export function render(instances: RenderInstance[], view: RenderView): Buffer {
  const { width, height } = view;
  const out = Buffer.alloc(width * height * 4);
  const depth = new Float32Array(width * height).fill(-Infinity);
  const R = mat3.fromValues(
    view.rot[0]!,
    view.rot[3]!,
    view.rot[6]!,
    view.rot[1]!,
    view.rot[4]!,
    view.rot[7]!,
    view.rot[2]!,
    view.rot[5]!,
    view.rot[8]!,
  );
  const light = vec3.normalize(vec3.create(), vec3.fromValues(...(view.light ?? [0, 0, 1])));
  const center = vec3.fromValues(...view.center);
  const v = vec3.create();

  for (const inst of instances) {
    const { mesh, positions } = inst;
    const n = positions.length / 3;
    // view-space positions -> screen x, y, depth
    const sx = new Float32Array(n);
    const sy = new Float32Array(n);
    const sz = new Float32Array(n);
    for (let i = 0; i < n; i++) {
      vec3.set(v, positions[i * 3]!, positions[i * 3 + 1]!, positions[i * 3 + 2]!);
      vec3.sub(v, v, center);
      vec3.transformMat3(v, v, R);
      sx[i] = width / 2 + v[0] * view.pixelsPerUnit;
      sy[i] = height / 2 - v[1] * view.pixelsPerUnit;
      sz[i] = v[2];
    }
    const e1 = vec3.create();
    const e2 = vec3.create();
    const nrm = vec3.create();
    for (let t = 0; t < mesh.indices.length; t += 3) {
      const ia = mesh.indices[t]!,
        ib = mesh.indices[t + 1]!,
        ic = mesh.indices[t + 2]!;
      const ax = sx[ia]!,
        ay = sy[ia]!,
        bx = sx[ib]!,
        by = sy[ib]!,
        cx = sx[ic]!,
        cy = sy[ic]!;
      const minX = Math.max(0, Math.floor(Math.min(ax, bx, cx)));
      const maxX = Math.min(width - 1, Math.ceil(Math.max(ax, bx, cx)));
      const minY = Math.max(0, Math.floor(Math.min(ay, by, cy)));
      const maxY = Math.min(height - 1, Math.ceil(Math.max(ay, by, cy)));
      if (minX > maxX || minY > maxY) continue;
      const area = (bx - ax) * (cy - ay) - (cx - ax) * (by - ay);
      if (Math.abs(area) < 1e-9) continue;

      // flat shading from the view-space normal (two-sided)
      vec3.set(e1, bx - ax, -(by - ay), sz[ib]! - sz[ia]!);
      vec3.set(e2, cx - ax, -(cy - ay), sz[ic]! - sz[ia]!);
      vec3.cross(nrm, e1, e2);
      vec3.normalize(nrm, nrm);
      const lambert = view.unlit ? 1 : 0.35 + 0.65 * Math.abs(vec3.dot(nrm, light));

      const texIdx = mesh.triTexture[t / 3]!;
      const tex = texIdx >= 0 ? mesh.textures[texIdx]! : null;
      for (let y = minY; y <= maxY; y++) {
        const py = y + 0.5;
        for (let x = minX; x <= maxX; x++) {
          const px = x + 0.5;
          let w0 = (bx - px) * (cy - py) - (cx - px) * (by - py);
          let w1 = (cx - px) * (ay - py) - (ax - px) * (cy - py);
          let w2 = (ax - px) * (by - py) - (bx - px) * (ay - py);
          if (area < 0) {
            w0 = -w0;
            w1 = -w1;
            w2 = -w2;
          }
          if (w0 < 0 || w1 < 0 || w2 < 0) continue;
          const inv = 1 / Math.abs(area);
          const l0 = w0 * inv,
            l1 = w1 * inv,
            l2 = w2 * inv;
          const z = l0 * sz[ia]! + l1 * sz[ib]! + l2 * sz[ic]!;
          const di = y * width + x;
          if (z <= depth[di]!) continue;
          depth[di] = z;
          let r: number, g: number, b: number;
          if (tex && mesh.uvs) {
            const u = l0 * mesh.uvs[ia * 2]! + l1 * mesh.uvs[ib * 2]! + l2 * mesh.uvs[ic * 2]!;
            const vv =
              l0 * mesh.uvs[ia * 2 + 1]! + l1 * mesh.uvs[ib * 2 + 1]! + l2 * mesh.uvs[ic * 2 + 1]!;
            [r, g, b] = sampleTexture(tex, u, vv) as [number, number, number];
          } else if (mesh.colors) {
            const c = mesh.colors;
            r = 255 * (l0 * c[ia * 3]! + l1 * c[ib * 3]! + l2 * c[ic * 3]!);
            g = 255 * (l0 * c[ia * 3 + 1]! + l1 * c[ib * 3 + 1]! + l2 * c[ic * 3 + 1]!);
            b = 255 * (l0 * c[ia * 3 + 2]! + l1 * c[ib * 3 + 2]! + l2 * c[ic * 3 + 2]!);
          } else {
            r = g = b = 200;
          }
          const o = di * 4;
          out[o] = Math.min(255, r * lambert);
          out[o + 1] = Math.min(255, g * lambert);
          out[o + 2] = Math.min(255, b * lambert);
          out[o + 3] = 255;
        }
      }
    }
  }
  return out;
}
