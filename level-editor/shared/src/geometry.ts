import type { MapCamera, Vec3 } from "./scene";

/** Signed map-plane area; positive for counter-clockwise winding. */
export function signedPolygonArea(
  points: readonly (readonly [number, number])[],
): number {
  let twice = 0;
  for (let i = 0; i < points.length; i++) {
    const a = points[i]!;
    const b = points[(i + 1) % points.length]!;
    twice += a[0] * b[1] - b[0] * a[1];
  }
  return twice / 2;
}
/** Column-major affine matrix, in the same Z-up scene frame as gameTransformMatrix. */
export function applyAffineMatrix(m: readonly number[], p: Vec3): Vec3 {
  if (m.length !== 16) throw new Error("affine matrix must contain 16 numbers");
  return [
    m[0]! * p[0] + m[4]! * p[1] + m[8]! * p[2] + m[12]!,
    m[1]! * p[0] + m[5]! * p[1] + m[9]! * p[2] + m[13]!,
    m[2]! * p[0] + m[6]! * p[1] + m[10]! * p[2] + m[14]!,
  ];
}
export function sceneToGame(cam: MapCamera, p: Vec3): Vec3 {
  const angle = (cam.elevation_deg * Math.PI) / 180;
  return [p[0], -p[1] * Math.sin(angle), p[2] * Math.cos(angle)];
}
export function sceneToGltf(p: Vec3): Vec3 {
  return [p[0], p[2], -p[1]];
}
export function gltfToScene(p: Vec3): Vec3 {
  return [p[0], -p[2], p[1]];
}
