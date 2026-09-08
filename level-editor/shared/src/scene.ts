// 3D scene reconstruction schema + the map camera model.
//
// The original maps are pre-rendered from 3D scenes with a fixed oblique
// orthographic camera. The game's world coordinates are already in projected
// units: a world point (x, y, z) lands on map pixel (x, y - z). Fitting the
// sight-obstacle footprints (which are rectangles in the true ground plane)
// gives the ground foreshortening `sin(elevation)`; the game itself defines
// it as aspect ratio 0.573576436 = cos 55° = sin 35°, i.e. the
// camera looks down 55° from the vertical, and every map fits that constant.
//
// Scene frame (used for all 3D assets and scene documents):
//   right-handed, Z up, units = map pixels (at 1:1 map resolution)
//   X = map x (east), Y = away from the camera (north, up the map image)
//   ground point at map pixel (px, py) -> (px, -py / sin θ, 0)
//   game world (x, y, z)                -> (x, -y / sin θ, z / cos θ)
//   projection back to map pixels       -> (X, -Y sin θ - Z cos θ)
//
// GLB export converts to glTF's Y-up frame with (X, Y, Z) -> (X, Z, -Y).

export type Vec3 = [number, number, number];
export type Quat = [number, number, number, number]; // x, y, z, w

export interface MapCamera {
  kind: "oblique-orthographic";
  /** camera elevation above the ground plane in degrees */
  elevation_deg: number;
}

/**
 * Axis convention of the camera frame that SAM 3D Objects poses are expressed
 * in. Determined empirically per reconstruction by silhouette matching.
 *   opengl:    x right, y up,   z backward (toward the viewer)
 *   opencv:    x right, y down, z forward
 *   pytorch3d: x left,  y up,   z forward
 */
export type CameraConvention = "opengl" | "opencv" | "pytorch3d";

/** local -> camera pose of one reconstructed object, as returned by the model */
export interface ModelPose {
  rotation: Quat;
  translation: Vec3;
  scale: Vec3;
  camera_pose?: number[][];
}

/** placement of a library model in a scene (scene frame, see above) */
export interface ScenePlacement {
  /** library asset id */
  asset: string;
  position: Vec3;
  /** rotation applied to the model's local frame */
  rotation: Quat;
  /** uniform scale from model-local units to scene units */
  scale: number;
}

/** 3D reconstruction data attached to a library asset (`<id>/model.glb`) */
export interface AssetModel {
  /** GLB file in the asset dir, model-local frame (unposed) */
  glb: string;
  /** gaussian splat PLY in the asset dir, if kept */
  splat?: string;
  /** true when the GLB carries a baked texture instead of vertex colors */
  textured: boolean;
  /** SAM 3D only: the pose the model returned; other backends return none */
  pose_l2c?: ModelPose;
  camera_convention?: CameraConvention;
  /** model-local axis-aligned bounds */
  bounds_local: { min: Vec3; max: Vec3 };
  /**
   * placement of this model in its source map's scene frame, fitted so the
   * projected model matches the extraction mask (ground contact at Z = 0)
   */
  placement: ScenePlacement;
  /** silhouette IoU between the placed model and the extraction mask */
  fit_iou: number;
  /** colour agreement (0..1) of the unlit map-camera render with the crop */
  fit_appearance?: number;
  extraction: {
    /** endpoint id, e.g. "fal-ai/sam-3/3d-objects" */
    tool: string;
    seed?: number;
    request_id?: string;
    /** map crop sent to the model, in map pixels [x, y, w, h] */
    crop: [number, number, number, number];
    /** request wall-clock seconds and approximate price, for comparisons */
    seconds?: number;
    price_usd?: number;
  };
}

export interface SceneGround {
  /** texture file relative to the scene document (a downscaled map image) */
  texture: string;
  /** map pixel rect covered by the texture [x, y, w, h] */
  rect: [number, number, number, number];
}

export interface SceneDoc {
  version: 1;
  /** source map name as in the datadir */
  map: string;
  /** map size in pixels */
  size: [number, number];
  camera: MapCamera;
  ground?: SceneGround;
  placements: ScenePlacement[];
  notes?: string;
}

// ── camera math ──────────────────────────────────────────────────────

export function cameraFromFit(sinElevation: number): MapCamera {
  return {
    kind: "oblique-orthographic",
    elevation_deg: (Math.asin(sinElevation) * 180) / Math.PI,
  };
}

function sincos(cam: MapCamera): [number, number] {
  const t = (cam.elevation_deg * Math.PI) / 180;
  return [Math.sin(t), Math.cos(t)];
}

/** map pixel on the ground plane -> scene point */
export function groundToScene(cam: MapCamera, px: number, py: number): Vec3 {
  const [s] = sincos(cam);
  return [px, -py / s, 0];
}

/** game world (x, y, z) -> scene point */
export function gameToScene(cam: MapCamera, x: number, y: number, z: number): Vec3 {
  const [s, c] = sincos(cam);
  return [x, -y / s, z / c];
}

/** scene point -> map pixel */
export function sceneToMap(cam: MapCamera, p: Vec3): [number, number] {
  const [s, c] = sincos(cam);
  return [p[0], -p[1] * s - p[2] * c];
}

/**
 * Row-major 3x3 matrix whose columns are the camera axes expressed in the
 * scene frame, i.e. `scene = M * cam` for a camera-frame vector.
 */
export function cameraToSceneMatrix(cam: MapCamera, convention: CameraConvention): number[] {
  const [s, c] = sincos(cam);
  // OpenGL camera: x right, y up, z backward
  const right: Vec3 = [1, 0, 0];
  const up: Vec3 = [0, s, c];
  const back: Vec3 = [0, -c, s];
  let ax: Vec3, ay: Vec3, az: Vec3;
  switch (convention) {
    case "opengl":
      [ax, ay, az] = [right, up, back];
      break;
    case "opencv":
      [ax, ay, az] = [right, neg(up), neg(back)];
      break;
    case "pytorch3d":
      [ax, ay, az] = [neg(right), up, neg(back)];
      break;
  }
  return [ax[0], ay[0], az[0], ax[1], ay[1], az[1], ax[2], ay[2], az[2]];
}

function neg(v: Vec3): Vec3 {
  return [-v[0], -v[1], -v[2]];
}

export const CAMERA_CONVENTIONS: CameraConvention[] = ["opengl", "opencv", "pytorch3d"];
