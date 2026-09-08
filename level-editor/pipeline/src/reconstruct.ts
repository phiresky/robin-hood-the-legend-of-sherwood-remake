// 3D reconstruction of library assets with SAM 3D Objects.
//
//   # augment an existing 2D asset (uses its mask.png + source bbox)
//   tsx src/reconstruct.ts --asset york-tower-house
//   # ad-hoc: 2D-extract with SAM 3 first, then reconstruct
//   tsx src/reconstruct.ts --map York --bbox 1450,840,200,300 --prompt "stone tower house" \
//       --name "York tower house" [--id ...] [--tags building,tower]
//   # batch from a detection file (see detect.ts)
//   tsx src/reconstruct.ts --detections work/york-scene/detections.json [--only id,id] \
//       [--limit N] [--skip-existing] [--parallel 4]
//
// Options: --backend sam3d|trellis2|tripo|hunyuan (alternatives fill
// asset.alt_models[backend] from an RGBA cutout; see backends.ts),
// --cutout mask|context|soft (what the alternatives are shown; non-mask
// results are keyed backend-mode),
// --no-patches (don't composite roof-closer patches), --seed N,
// --vertex-colors (no baked texture), --splats (copy the gaussian splat PLY),
// --pad px (context around the mask sent to the model), --diag (tabulate
// every pose interpretation).
//
// Flow per asset: crop the map around the mask with context, send crop +
// mask to SAM 3D, load the returned GLB, and fit it into the map's scene
// frame (see fitPlacement). Writes library/<id>/model.glb +
// asset.json.model and a review sheet in work/<id>/.
import fs from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import sharp from "sharp";
import { quat, vec3 } from "gl-matrix";
import {
  CAMERA_CONVENTIONS,
  cameraToSceneMatrix,
  type AssetDescriptor,
  type AssetModel,
  type CameraConvention,
  type MapCamera,
  type ModelPose,
  type ProtoLevel,
  type Quat,
  type ScenePlacement,
} from "@rle/shared";
import { libraryDir, workDir } from "./env";
import { loadProtoLevel, mapImageSource, writeMaskedAsset } from "./asset-writer";
import { EXTRACT_DEFAULTS, runExtraction, slugify } from "./extract-core";
import type { Bbox } from "./clip";
import { fitMapCamera } from "./map-camera";
import { reconstruct3d } from "./sam3d";
import { ALT_BACKENDS, backendInfo, reconstructWith, type Backend } from "./backends";
import {
  appearanceScore,
  bbox2d,
  bounds,
  composeTransforms,
  gridIou,
  loadGlb,
  maskToGrid,
  mat3ToQuat,
  projectToMap,
  rasterizeAppearance,
  rasterizeSilhouette,
  transformPositions,
  uniformScaleOf,
  type Grid,
  type MeshData,
} from "./mesh";
import { mapView, orbitView, render } from "./render";

/**
 * What the alternative backends are shown (SAM 3D always gets the full crop
 * plus the mask):
 *   mask    — building pixels on transparent, 8% margin (default)
 *   context — the opaque map crop uncropped by 10% on each side, no alpha
 *   soft    — building opaque, surroundings fading out over ~20% of the size
 */
export type CutoutMode = "mask" | "context" | "soft";

export interface ReconstructOptions {
  /** which image-to-3D model to use; sam3d fills asset.model, others asset.alt_models[key] */
  backend: Backend;
  cutout: CutoutMode;
  applyPatches: boolean;
  seed: number;
  textured: boolean;
  keepSplats: boolean;
  /** context padding around the mask bbox in map pixels; default 35% of the larger side, min 48 */
  pad?: number;
  /** tabulate every pose interpretation (see fitPlacement) */
  diag?: boolean;
}

export const RECONSTRUCT_DEFAULTS: ReconstructOptions = {
  backend: "sam3d",
  cutout: "mask",
  applyPatches: true,
  seed: 42,
  textured: true,
  keepSplats: false,
};

export interface FitCandidate extends PoseInterpretation {
  placement: ScenePlacement;
  iou: number;
  /** colour agreement of the rendered model with the crop (0..1), if a reference was given */
  appearance: number | null;
  /** yaw offset applied on top of the model's pose after snapping, degrees */
  yaw_deg: number;
  /** ratio of x-scale to y-scale the mask bbox would have needed; 1 = isotropic */
  anisotropy: number;
  /** tilt of the model's local up from scene Z before gravity snapping, degrees */
  tilt_deg: number;
  positions: Float32Array;
}

/**
 * Component order of the pose quaternion. The endpoint documents [x, y, z, w]
 * but the model code is pytorch3d-based, which stores [w, x, y, z]; both are
 * tried and the silhouette decides.
 */
export type QuaternionOrder = "xyzw" | "wxyz";
const QUATERNION_ORDERS: QuaternionOrder[] = ["xyzw", "wxyz"];

/**
 * Axis convention of the local frame the pose refers to, relative to the
 * GLB's Y-up frame. The reference code re-maps gaussians with
 * (x, y, z) -> (-x, z, y) before rendering, so the model's own frame may be
 * Z-up; "zup" applies that involution to the GLB before posing.
 */
export type LocalSwap = "none" | "zup";
const LOCAL_SWAPS: LocalSwap[] = ["none", "zup"];
const ZUP_SWAP: Quat = mat3ToQuat([-1, 0, 0, 0, 0, 1, 0, 1, 0].map((v) => v) as number[]);
/** glTF Y-up -> scene Z-up: (x, y, z) -> (x, -z, y), i.e. +90° about X */
const YUP_TO_ZUP: Quat = [Math.SQRT1_2, 0, 0, Math.SQRT1_2];

/** what the fit needs to know about a reconstruction: SAM 3D's pose, or none */
export interface FitInput {
  pose: ModelPose | null;
}

const levelCache = new Map<string, Promise<ProtoLevel>>();
function levelFor(map: string): Promise<ProtoLevel> {
  let p = levelCache.get(map);
  if (!p) {
    p = loadProtoLevel(map);
    levelCache.set(map, p);
  }
  return p;
}
const cameraCache = new Map<string, MapCamera>();
async function cameraFor(map: string): Promise<MapCamera> {
  let cam = cameraCache.get(map);
  if (!cam) {
    const fit = fitMapCamera(await levelFor(map));
    console.log(
      `${map}: camera elevation ${fit.elevation_deg.toFixed(2)}° (mean |cos| ${fit.fit_cos.toFixed(4)} over ${fit.quads} footprints)`,
    );
    cam = { kind: fit.kind, elevation_deg: fit.elevation_deg };
    cameraCache.set(map, cam);
  }
  return cam;
}

/** one way of reading the pose the endpoint returns */
export interface PoseInterpretation {
  quaternionOrder: QuaternionOrder;
  convention: CameraConvention;
  invertRotation: boolean;
  localSwap: LocalSwap;
}

/**
 * The interpretation that leaves buildings upright: pytorch3d quaternions
 * are [w, x, y, z], its camera frame is x-left/y-up/z-forward, the model's
 * local frame is Z-up relative to the exported GLB (the reference code's
 * (x, y, z) -> (-x, z, y) remap), and the returned quaternion rotates
 * camera->local (pytorch3d applies rotations to row vectors, which is the
 * transpose of the column-vector convention used here). Verified over 42
 * York buildings (pose-diag.ts): median pre-snap tilt 9°, 38/42 under 20°;
 * every other reading has a mean tilt ≥ 62°.
 */
export const CANONICAL_INTERPRETATION: PoseInterpretation = {
  quaternionOrder: "wxyz",
  convention: "pytorch3d",
  invertRotation: true,
  localSwap: "zup",
};

/** the map crop over the mask bbox, downsampled to the fit grid */
export interface ReferenceGrid {
  rgb: Uint8Array;
  width: number;
  height: number;
}

/** fit grid resolution: map pixels per cell for a mask bbox */
export function gridStep(aw: number, ah: number): number {
  return Math.max(1, Math.round(Math.max(aw, ah) / 160));
}

function allInterpretations(): PoseInterpretation[] {
  const out: PoseInterpretation[] = [];
  for (const quaternionOrder of QUATERNION_ORDERS)
    for (const convention of CAMERA_CONVENTIONS)
      for (const invertRotation of [false, true])
        for (const localSwap of LOCAL_SWAPS)
          out.push({ quaternionOrder, convention, invertRotation, localSwap });
  return out;
}

export function describe(i: PoseInterpretation): string {
  return `${i.quaternionOrder}/${i.convention}${i.invertRotation ? "⁻¹" : ""}${i.localSwap === "zup" ? "/zup" : ""}`;
}

/**
 * Fit the model into the map's scene frame under one pose interpretation:
 * the model is gravity-snapped (buildings stand upright, so any residual
 * tilt of the local up axis is removed and reported), scaled so the
 * projected bbox area matches the mask bbox, placed with its lowest point
 * on the ground plane and its projected bbox aligned to the mask bbox, and
 * scored by silhouette IoU against the mask.
 */
function fitUnder(
  interp: PoseInterpretation,
  assetId: string,
  mesh: MeshData,
  obj: FitInput,
  cam: MapCamera,
  bbox: Bbox,
  maskGrid: Grid,
  step: number,
  reference: ReferenceGrid | null,
  refineYaw: boolean,
): FitCandidate | null {
  const [ax, ay, aw, ah] = bbox;
  const t = (cam.elevation_deg * Math.PI) / 180;
  const sinT = Math.sin(t);
  const cosT = Math.cos(t);
  const localUp = vec3.fromValues(0, 1, 0);
  const sceneUp = vec3.fromValues(0, 0, 1);

  let toScene: { rotation: Quat; scale: number; translation: [number, number, number] };
  if (obj.pose) {
    const s = uniformScaleOf(obj.pose);
    const r = obj.pose.rotation;
    const ordered: Quat = interp.quaternionOrder === "xyzw" ? r : [r[1], r[2], r[3], r[0]];
    const camToScene = mat3ToQuat(cameraToSceneMatrix(cam, interp.convention));
    const q = quat.normalize(quat.create(), quat.fromValues(...ordered));
    if (interp.invertRotation) quat.conjugate(q, q);
    const rotation: Quat = [q[0], q[1], q[2], q[3]];
    // glb local -> model local -> camera -> scene (unscaled)
    const pose = composeTransforms(
      {
        rotation: interp.localSwap === "zup" ? ZUP_SWAP : [0, 0, 0, 1],
        scale: 1,
        translation: [0, 0, 0],
      },
      { rotation, scale: s, translation: obj.pose.translation },
    );
    toScene = composeTransforms(pose, { rotation: camToScene, scale: 1, translation: [0, 0, 0] });
  } else {
    // no pose: the GLB is canonical Y-up and upright; only the yaw is unknown
    toScene = { rotation: YUP_TO_ZUP, scale: 1, translation: [0, 0, 0] };
  }
  // gravity snap
  const up = vec3.transformQuat(vec3.create(), localUp, quat.fromValues(...toScene.rotation));
  const tilt = Math.acos(Math.max(-1, Math.min(1, vec3.dot(up, sceneUp))));
  const snap = quat.rotationTo(quat.create(), up, sceneUp);
  const snapped = composeTransforms(toScene, {
    rotation: [snap[0], snap[1], snap[2], snap[3]],
    scale: 1,
    translation: [0, 0, 0],
  });

  /** scale + translate a (snapped, yawed) transform onto the mask bbox and score it */
  const place = (yawDeg: number): FitCandidate | null => {
    const yawQ = quat.setAxisAngle(quat.create(), sceneUp, (yawDeg * Math.PI) / 180);
    const yawed = composeTransforms(snapped, {
      rotation: [yawQ[0], yawQ[1], yawQ[2], yawQ[3]],
      scale: 1,
      translation: [0, 0, 0],
    });
    const p0 = transformPositions(mesh.positions, yawed.rotation, yawed.scale, yawed.translation);
    const proj = projectToMap(cam, p0);
    const [x0, y0, x1, y1] = bbox2d(proj);
    const pw = x1 - x0;
    const ph = y1 - y0;
    if (pw <= 0 || ph <= 0) return null;
    const k = Math.sqrt((aw * ah) / (pw * ph));
    const anisotropy = aw / pw / (ah / ph);
    const minZ = bounds(p0).min[2];
    const X0 = ax - k * x0;
    const Z0 = -k * minZ;
    const Y0 = -(ay + ah - k * y1 + Z0 * cosT) / sinT;
    const placed = composeTransforms(yawed, {
      rotation: [0, 0, 0, 1],
      scale: k,
      translation: [X0, Y0, Z0],
    });
    const positions = transformPositions(
      mesh.positions,
      placed.rotation,
      placed.scale,
      placed.translation,
    );
    const sil = rasterizeSilhouette(projectToMap(cam, positions), mesh.indices, bbox, step);
    const iou = gridIou(sil, maskGrid);
    const appearance = reference
      ? appearanceScore(rasterizeAppearance(positions, cam, mesh, bbox, step), reference, maskGrid)
      : null;
    return {
      ...interp,
      tilt_deg: (tilt * 180) / Math.PI,
      yaw_deg: yawDeg,
      placement: {
        asset: assetId,
        position: placed.translation,
        rotation: placed.rotation,
        scale: placed.scale,
      },
      iou,
      appearance,
      anisotropy,
      positions,
    };
  };

  if (!refineYaw) return place(0);
  // yaw search: the pose's yaw is unreliable on oblique artwork; the baked
  // texture reproduces the crop only at the right yaw, so score silhouette
  // + colour agreement over a full turn, then refine around the best
  const score = (c: FitCandidate) => c.iou + (c.appearance ?? 0);
  let best: FitCandidate | null = null;
  for (let yaw = 0; yaw < 360; yaw += 10) {
    const c = place(yaw);
    if (c && (!best || score(c) > score(best))) best = c;
  }
  if (!best) return null;
  for (const dy of [-6, -4, -2, 2, 4, 6]) {
    const c = place(best.yaw_deg + dy);
    if (c && score(c) > score(best)) best = c;
  }
  return best;
}

/** fit under every interpretation (diagnostics; see pose-diag.ts) */
export function fitAll(
  assetId: string,
  mesh: MeshData,
  obj: FitInput,
  cam: MapCamera,
  bbox: Bbox,
  mask: Uint8Array,
): FitCandidate[] {
  const [, , aw, ah] = bbox;
  const step = gridStep(aw, ah);
  const maskGrid = maskToGrid({ data: mask, width: aw, height: ah }, step);
  return allInterpretations()
    .map((i) => fitUnder(i, assetId, mesh, obj, cam, bbox, maskGrid, step, null, false))
    .filter((c): c is FitCandidate => c !== null);
}

/**
 * Fit the model into the map's scene frame using the canonical pose
 * interpretation. With `diag`, every interpretation is fitted and tabulated
 * (pre-snap tilt and silhouette IoU) so a new map/model version can be
 * checked for a different convention.
 */
export function fitPlacement(
  assetId: string,
  mesh: MeshData,
  obj: FitInput,
  cam: MapCamera,
  bbox: Bbox,
  mask: Uint8Array,
  reference: ReferenceGrid | null,
  diag = false,
): FitCandidate {
  const [, , aw, ah] = bbox;
  const step = gridStep(aw, ah);
  const maskGrid = maskToGrid({ data: mask, width: aw, height: ah }, step);

  if (diag) {
    const rows = allInterpretations()
      .map((i) => fitUnder(i, assetId, mesh, obj, cam, bbox, maskGrid, step, null, false))
      .filter((c): c is FitCandidate => c !== null)
      .sort((a, b) => a.tilt_deg - b.tilt_deg);
    console.log(
      `${assetId}: pose interpretations by pre-snap tilt\n  ` +
        rows
          .map((c) => `${describe(c)}: tilt ${c.tilt_deg.toFixed(0)}° IoU ${c.iou.toFixed(3)} aniso ${c.anisotropy.toFixed(2)}`)
          .join("\n  "),
    );
  }
  const fit = fitUnder(
    CANONICAL_INTERPRETATION,
    assetId,
    mesh,
    obj,
    cam,
    bbox,
    maskGrid,
    step,
    reference,
    true,
  );
  if (!fit) throw new Error(`${assetId}: canonical interpretation produced no geometry`);
  if (fit.tilt_deg > 45) {
    console.warn(
      `${assetId}: WARNING model was ${fit.tilt_deg.toFixed(0)}° from upright before gravity snapping`,
    );
  }
  return fit;
}

async function reviewSheet(
  assetId: string,
  cropPng: Buffer,
  cropRect: Bbox,
  cam: MapCamera,
  mesh: MeshData,
  fit: FitCandidate,
  suffix = "",
) {
  const [, , cw, ch] = cropRect;
  const dir = path.join(workDir, assetId);
  await fs.mkdir(dir, { recursive: true });

  // 1: crop with the fitted model rendered from the map camera on top
  // (unlit: the texture already carries the map's lighting)
  const over = render([{ mesh, positions: fit.positions }], {
    ...mapView(cam, cropRect),
    unlit: true,
  });
  const overlay = await sharp(cropPng)
    .composite([{ input: over, raw: { width: cw, height: ch, channels: 4 }, blend: "over" }])
    .png()
    .toBuffer();

  // 2..5: orbit views around the placed model
  const b = bounds(fit.positions);
  const center: [number, number, number] = [
    (b.min[0] + b.max[0]) / 2,
    (b.min[1] + b.max[1]) / 2,
    (b.min[2] + b.max[2]) / 2,
  ];
  const extent = Math.max(b.max[0] - b.min[0], b.max[1] - b.min[1], b.max[2] - b.min[2]);
  const size = Math.max(cw, ch);
  const ppu = (size * 0.8) / extent;
  const views = [
    [35, 30],
    [125, 30],
    [215, 30],
    [305, 30],
  ].map(([yaw, pitch]) =>
    sharp(render([{ mesh, positions: fit.positions }], orbitView(center, yaw!, pitch!, size, size, ppu)), {
      raw: { width: size, height: size, channels: 4 },
    })
      .flatten({ background: "#303030" })
      .png()
      .toBuffer(),
  );
  const orbit = await Promise.all(views);

  const tiles = [
    { input: cropPng, left: 0, top: 0 },
    { input: overlay, left: cw, top: 0 },
    ...orbit.map((buf, i) => ({ input: buf, left: 2 * cw + i * size, top: 0 })),
  ];
  const sheet = await sharp({
    create: {
      width: 2 * cw + 4 * size,
      height: Math.max(ch, size),
      channels: 4,
      background: "#202020",
    },
  })
    .composite(tiles)
    .png()
    .toBuffer();
  const file = path.join(dir, `fit${suffix}.png`);
  await fs.writeFile(file, sheet);
  return file;
}

/** attach a SAM 3D reconstruction to an existing library asset */
export async function reconstructAsset(
  assetId: string,
  opts: ReconstructOptions,
): Promise<{ model: AssetModel; sheet: string }> {
  const dir = path.join(libraryDir, assetId);
  const desc: AssetDescriptor = JSON.parse(await fs.readFile(path.join(dir, "asset.json"), "utf8"));
  const map = desc.source.map;
  const [ax, ay, aw, ah] = desc.source.bbox;
  const { data: maskRaw, info } = await sharp(path.join(dir, desc.images.mask))
    .extractChannel(0)
    .raw()
    .toBuffer({ resolveWithObject: true });
  if (info.width !== aw || info.height !== ah) {
    throw new Error(`${assetId}: mask ${info.width}x${info.height} != bbox ${aw}x${ah}`);
  }
  const mask = new Uint8Array(maskRaw);

  const level = await levelFor(map);
  const cam = await cameraFor(map);
  const src = await mapImageSource(map, "Day", opts.applyPatches, level);
  if (!src) throw new Error(`no Day map for ${map}`);
  const meta = await sharp(src).metadata();
  const mapW = meta.width!;
  const mapH = meta.height!;

  const pad = opts.pad ?? Math.max(48, Math.round(0.35 * Math.max(aw, ah)));
  const cx = Math.max(0, ax - pad);
  const cy = Math.max(0, ay - pad);
  const cw = Math.min(mapW - cx, aw + 2 * pad + Math.min(0, ax - pad));
  const ch = Math.min(mapH - cy, ah + 2 * pad + Math.min(0, ay - pad));
  const cropRect: Bbox = [cx, cy, cw, ch];
  const cropPng = await sharp(src).extract({ left: cx, top: cy, width: cw, height: ch }).png().toBuffer();

  // mask as an RGB PNG at crop size
  const maskRgb = Buffer.alloc(cw * ch * 3);
  for (let y = 0; y < ah; y++) {
    for (let x = 0; x < aw; x++) {
      if (!mask[y * aw + x]) continue;
      const o = ((y + ay - cy) * cw + (x + ax - cx)) * 3;
      maskRgb[o] = maskRgb[o + 1] = maskRgb[o + 2] = 255;
    }
  }
  const maskPng = await sharp(maskRgb, { raw: { width: cw, height: ch, channels: 3 } })
    .png()
    .toBuffer();

  let mesh: MeshData;
  let fitInput: FitInput;
  let glbSource: string;
  let splatSource: string | null = null;
  let extraction: AssetModel["extraction"];
  if (opts.backend === "sam3d") {
    console.log(`${assetId}: SAM 3D on ${cw}x${ch} crop of ${map} @ ${cx},${cy}`);
    const res = await reconstruct3d({
      imagePng: cropPng,
      maskPngs: [maskPng],
      seed: opts.seed,
      textured: opts.textured,
    });
    const obj = res.objects[0]!;
    mesh = await loadGlb(obj.glb);
    console.log(
      `${assetId}: mesh ${mesh.positions.length / 3} verts, ${mesh.indices.length / 3} tris, ${mesh.textures.length} texture(s), pose scale ${obj.pose.scale[0]?.toFixed(3)} t=${obj.pose.translation.map((v) => v.toFixed(2)).join(",")}`,
    );
    fitInput = { pose: obj.pose };
    glbSource = obj.glb;
    extraction = {
      tool: "fal-ai/sam-3/3d-objects",
      seed: opts.seed,
      request_id: res.requestId,
      crop: cropRect,
      price_usd: 0.02,
    };
    splatSource = opts.keepSplats ? obj.splat : null;
  } else {
    // RGBA cutout: the masked object on a transparent background with a
    // little margin is what these models expect; the other modes test
    // whether context helps them (see CutoutMode)
    const feather = opts.cutout === "soft" ? Math.max(16, Math.round(0.2 * Math.max(aw, ah))) : 0;
    const margin =
      opts.cutout === "context"
        ? Math.max(16, Math.round(0.1 * Math.max(aw, ah)))
        : Math.max(8, Math.round(0.08 * Math.max(aw, ah))) + feather;
    const ox = Math.max(0, ax - margin);
    const oy = Math.max(0, ay - margin);
    const ow = Math.min(mapW - ox, aw + 2 * margin);
    const oh = Math.min(mapH - oy, ah + 2 * margin);
    const rgb = await sharp(src)
      .extract({ left: ox, top: oy, width: ow, height: oh })
      .removeAlpha()
      .raw()
      .toBuffer();
    // distance (in px) from the mask for the soft falloff: two-pass chamfer
    let dist: Float32Array | null = null;
    if (feather > 0) {
      dist = new Float32Array(ow * oh).fill(1e9);
      for (let y = 0; y < oh; y++) {
        for (let x = 0; x < ow; x++) {
          const mx = x + ox - ax;
          const my = y + oy - ay;
          if (mx >= 0 && my >= 0 && mx < aw && my < ah && mask[my * aw + mx]) dist[y * ow + x] = 0;
        }
      }
      const relax = (i: number, j: number, d: number) => {
        if (dist![j]! + d < dist![i]!) dist![i] = dist![j]! + d;
      };
      for (let y = 0; y < oh; y++)
        for (let x = 0; x < ow; x++) {
          const i = y * ow + x;
          if (x > 0) relax(i, i - 1, 1);
          if (y > 0) relax(i, i - ow, 1);
          if (x > 0 && y > 0) relax(i, i - ow - 1, Math.SQRT2);
          if (x < ow - 1 && y > 0) relax(i, i - ow + 1, Math.SQRT2);
        }
      for (let y = oh - 1; y >= 0; y--)
        for (let x = ow - 1; x >= 0; x--) {
          const i = y * ow + x;
          if (x < ow - 1) relax(i, i + 1, 1);
          if (y < oh - 1) relax(i, i + ow, 1);
          if (x < ow - 1 && y < oh - 1) relax(i, i + ow + 1, Math.SQRT2);
          if (x > 0 && y < oh - 1) relax(i, i + ow - 1, Math.SQRT2);
        }
    }
    const rgba = Buffer.alloc(ow * oh * 4);
    for (let y = 0; y < oh; y++) {
      for (let x = 0; x < ow; x++) {
        const mx = x + ox - ax;
        const my = y + oy - ay;
        const inside = mx >= 0 && my >= 0 && mx < aw && my < ah && mask[my * aw + mx];
        const o = (y * ow + x) * 4;
        let alpha: number;
        if (opts.cutout === "context") alpha = 255;
        else if (dist) alpha = Math.round(255 * Math.max(0, 1 - dist[y * ow + x]! / feather));
        else alpha = inside ? 255 : 0;
        rgba[o] = rgb[(y * ow + x) * 3]!;
        rgba[o + 1] = rgb[(y * ow + x) * 3 + 1]!;
        rgba[o + 2] = rgb[(y * ow + x) * 3 + 2]!;
        rgba[o + 3] = alpha;
      }
    }
    const cutoutPng = await sharp(rgba, { raw: { width: ow, height: oh, channels: 4 } })
      .png()
      .toBuffer();
    console.log(`${assetId}: ${opts.backend} on ${ow}x${oh} ${opts.cutout} cutout of ${map} @ ${ox},${oy}`);
    const res = await reconstructWith(opts.backend, cutoutPng, opts.seed);
    mesh = await loadGlb(res.glb);
    console.log(
      `${assetId}: ${opts.backend} mesh ${mesh.positions.length / 3} verts, ${mesh.indices.length / 3} tris, ${mesh.textures.length} texture(s), ${res.seconds.toFixed(0)} s`,
    );
    fitInput = { pose: null };
    glbSource = res.glb;
    extraction = {
      tool: res.endpoint,
      seed: opts.seed,
      request_id: res.requestId,
      crop: [ox, oy, ow, oh],
      seconds: res.seconds,
      price_usd: backendInfo(opts.backend).price,
    };
  }

  // reference: the crop over the mask bbox at the fit grid resolution
  const step = gridStep(aw, ah);
  const refGrid = await sharp(cropPng)
    .extract({ left: ax - cx, top: ay - cy, width: aw, height: ah })
    .resize(Math.ceil(aw / step), Math.ceil(ah / step), { kernel: "lanczos3", fit: "fill" })
    .removeAlpha()
    .raw()
    .toBuffer({ resolveWithObject: true });
  const reference: ReferenceGrid = {
    rgb: new Uint8Array(refGrid.data),
    width: refGrid.info.width,
    height: refGrid.info.height,
  };
  const altKey = `${opts.backend}${opts.cutout === "mask" ? "" : `-${opts.cutout}`}`;
  const suffix = opts.backend === "sam3d" ? "" : `-${altKey}`;
  const fit = fitPlacement(assetId, mesh, fitInput, cam, desc.source.bbox, mask, reference, opts.diag);
  if (fit.iou < 0.5) {
    console.warn(`${assetId}: WARNING low silhouette IoU ${fit.iou.toFixed(3)} — check work/${assetId}/fit${suffix}.png`);
  }

  const glbName = `model${suffix}.glb`;
  await fs.copyFile(glbSource, path.join(dir, glbName));
  let splat: string | undefined;
  if (splatSource) {
    await fs.copyFile(splatSource, path.join(dir, "splat.ply"));
    splat = "splat.ply";
  }
  const model: AssetModel = {
    glb: glbName,
    splat,
    textured: opts.backend === "sam3d" ? opts.textured : true,
    pose_l2c: fitInput.pose ?? undefined,
    camera_convention: fitInput.pose ? fit.convention : undefined,
    bounds_local: bounds(mesh.positions),
    placement: fit.placement,
    fit_iou: fit.iou,
    fit_appearance: fit.appearance ?? undefined,
    extraction,
  };
  if (opts.backend === "sam3d") desc.model = model;
  else desc.alt_models = { ...(desc.alt_models ?? {}), [altKey]: model };
  if (!desc.tags.includes("3d")) desc.tags.push("3d");
  await fs.writeFile(path.join(dir, "asset.json"), JSON.stringify(desc, null, 2));

  const sheet = await reviewSheet(assetId, cropPng, cropRect, cam, mesh, fit, suffix);
  console.log(
    `${assetId}: placed at ${fit.placement.position.map((v) => v.toFixed(1)).join(",")} scale ${fit.placement.scale.toFixed(2)} (${describe(fit)}, IoU ${fit.iou.toFixed(3)}, appearance ${fit.appearance?.toFixed(3) ?? "-"}, yaw ${fit.yaw_deg}°, aniso ${fit.anisotropy.toFixed(2)}, tilt ${fit.tilt_deg.toFixed(0)}°); review ${sheet}`,
  );
  return { model, sheet };
}

// ── detections ───────────────────────────────────────────────────────

/** one entry of a detect.ts output file */
export interface Detection {
  id: string;
  name: string;
  tags?: string[];
  prompt: string;
  score: number | null;
  /** tight map-pixel bbox [x, y, w, h] */
  bbox: Bbox;
  /** 8-bit mask PNG relative to the detections file, bbox-sized */
  mask: string;
  /** for merged detections (merge-detections.ts): ids of the members */
  members?: string[];
}

export interface DetectionsFile {
  map: string;
  apply_patches: boolean;
  detections: Detection[];
}

async function reconstructDetections(
  file: string,
  only: Set<string> | null,
  limit: number,
  skipExisting: boolean,
  parallel: number,
  opts: ReconstructOptions,
) {
  const det: DetectionsFile = JSON.parse(await fs.readFile(file, "utf8"));
  const level = await levelFor(det.map);
  await cameraFor(det.map);
  const results: { id: string; iou?: number; error?: string; skipped?: string }[] = [];

  const queue: Detection[] = [];
  for (const d of det.detections) {
    if (only && !only.has(d.id)) continue;
    if (queue.length >= limit) break;
    if (skipExisting) {
      try {
        const existing: AssetDescriptor = JSON.parse(
          await fs.readFile(path.join(libraryDir, d.id, "asset.json"), "utf8"),
        );
        if (existing.model) {
          results.push({ id: d.id, skipped: "already reconstructed" });
          continue;
        }
      } catch {
        // no asset yet
      }
    }
    queue.push(d);
  }
  console.log(`${queue.length} detections to reconstruct, ${parallel} in parallel`);

  let next = 0;
  let done = 0;
  const worker = async () => {
    while (next < queue.length) {
      const d = queue[next++]!;
      try {
        const { data } = await sharp(path.resolve(path.dirname(file), d.mask))
          .extractChannel(0)
          .raw()
          .toBuffer({ resolveWithObject: true });
        const desc = await writeMaskedAsset({
          map: det.map,
          level,
          applyPatches: det.apply_patches,
          mask: new Uint8Array(data),
          bbox: d.bbox,
          id: d.id,
          name: d.name,
          tags: d.tags ?? ["building"],
          scaleClass: "unique",
          extraction: {
            tool: "fal-ai/sam-3/image-rle",
            prompt: d.prompt,
            score: d.score ?? undefined,
          },
        });
        if (d.members) {
          desc.merged_from = d.members;
          await fs.writeFile(
            path.join(libraryDir, d.id, "asset.json"),
            JSON.stringify(desc, null, 2),
          );
        }
        const { model } = await reconstructAsset(d.id, { ...opts, applyPatches: det.apply_patches });
        results.push({ id: d.id, iou: model.fit_iou });
      } catch (e) {
        console.error(`FAILED ${d.id}: ${e}`);
        results.push({ id: d.id, error: String(e) });
      }
      done++;
      console.log(`[${done}/${queue.length}] ${d.id} done`);
    }
  };
  await Promise.all(Array.from({ length: Math.max(1, parallel) }, worker));

  console.log("\n=== reconstruction summary ===");
  results.sort((a, b) => a.id.localeCompare(b.id));
  for (const r of results) {
    console.log(
      `${r.id}: ${r.error ? `ERROR ${r.error}` : r.skipped ? r.skipped : `IoU ${r.iou!.toFixed(3)}`}`,
    );
  }
  const ious = results.filter((r) => r.iou !== undefined).map((r) => r.iou!);
  if (ious.length) {
    ious.sort((a, b) => a - b);
    console.log(
      `${ious.length} reconstructed, ${results.filter((r) => r.error).length} failed; IoU median ${ious[Math.floor(ious.length / 2)]!.toFixed(3)}, min ${ious[0]!.toFixed(3)}, ${ious.filter((v) => v < 0.5).length} below 0.5`,
    );
  }
}

// ── CLI ──────────────────────────────────────────────────────────────

async function main() {
  const argv = process.argv.slice(2);
  const get = (flag: string): string | undefined => {
    const i = argv.indexOf(`--${flag}`);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  const has = (flag: string) => argv.includes(`--${flag}`);
  const backend = (get("backend") ?? "sam3d") as Backend;
  if (backend !== "sam3d" && !ALT_BACKENDS.includes(backend as (typeof ALT_BACKENDS)[number])) {
    throw new Error(`unknown --backend ${backend} (sam3d, ${ALT_BACKENDS.join(", ")})`);
  }
  const cutout = (get("cutout") ?? "mask") as CutoutMode;
  if (!["mask", "context", "soft"].includes(cutout)) throw new Error(`unknown --cutout ${cutout}`);
  const opts: ReconstructOptions = {
    backend,
    cutout,
    applyPatches: !has("no-patches"),
    seed: Number(get("seed") ?? RECONSTRUCT_DEFAULTS.seed),
    textured: !has("vertex-colors"),
    keepSplats: has("splats"),
    pad: get("pad") !== undefined ? Number(get("pad")) : undefined,
    diag: has("diag"),
  };

  const detections = get("detections");
  if (detections) {
    const only = get("only") ? new Set(get("only")!.split(",")) : null;
    await reconstructDetections(
      detections,
      only,
      Number(get("limit") ?? Infinity),
      has("skip-existing"),
      Number(get("parallel") ?? 4),
      opts,
    );
    return;
  }

  let assetId = get("asset");
  if (!assetId) {
    const map = get("map");
    const bboxStr = get("bbox");
    const prompt = get("prompt");
    const name = get("name");
    if (!map || !bboxStr || !prompt || !name) {
      throw new Error("usage: --asset <id> | --detections <file> | --map --bbox --prompt --name");
    }
    const bbox = bboxStr.split(",").map(Number);
    if (bbox.length !== 4 || bbox.some(Number.isNaN)) throw new Error("--bbox wants x,y,w,h");
    assetId = get("id") ?? slugify(name);
    const summary = await runExtraction({
      map,
      bbox: bbox as Bbox,
      prompt,
      applyPatches: opts.applyPatches,
      fillHoles: true,
      name,
      id: assetId,
      tags: get("tags")?.split(",") ?? ["building"],
      pad: EXTRACT_DEFAULTS.pad,
      maxMasks: EXTRACT_DEFAULTS.maxMasks,
      pick: "best",
      scaleClass: "unique",
      minScore: EXTRACT_DEFAULTS.minScore,
      minArea: EXTRACT_DEFAULTS.minArea,
      dedupeIou: 1.01, // never dedupe an explicitly requested asset
    });
    if (summary.written.length === 0) {
      throw new Error(`2D extraction wrote nothing: ${summary.skipped.map((s) => s.reason).join("; ")}`);
    }
  }
  await reconstructAsset(assetId, opts);
}

// run the CLI only when executed directly (other scripts import fitPlacement)
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
