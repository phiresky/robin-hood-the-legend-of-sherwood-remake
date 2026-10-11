import * as THREE from "three";

/** Frozen local transforms for rigid pieces below an asset's existing coordinate wrapper. */
export interface ReviewedNodePose {
  node: string;
  matrix: number[];
}

/** Validate the entire pose table before changing any loaded object. */
export function validateReviewedNodePoses(frames: ReviewedNodePose[][]): void {
  if (!Array.isArray(frames) || !frames.length) throw new Error("Missing reviewed node poses");
  let names: string[] | undefined;
  for (const frame of frames) {
    if (!Array.isArray(frame) || !frame.length) throw new Error("Empty reviewed node pose");
    const current = frame.map((row) => row?.node);
    if (
      current.some((name) => typeof name !== "string" || !name) ||
      new Set(current).size !== current.length ||
      (names && (current.length !== names.length || current.some((name, i) => name !== names![i])))
    )
      throw new Error("Inconsistent reviewed node identities");
    names = current;
    for (const { matrix } of frame) {
      if (!Array.isArray(matrix) || matrix.length !== 16 || !matrix.every(Number.isFinite))
        throw new Error("Invalid reviewed node matrix");
      if ([3, 7, 11].some((i) => Math.abs(matrix[i]!) > 1e-6) || Math.abs(matrix[15]! - 1) > 1e-6)
        throw new Error("Reviewed node matrix must be affine");
      const basis = [0, 4, 8].map((i) => new THREE.Vector3(...matrix.slice(i, i + 3)));
      if (
        basis.some((v) => Math.abs(v.lengthSq() - 1) > 1e-4) ||
        Math.abs(basis[0]!.dot(basis[1]!)) > 1e-4 ||
        Math.abs(basis[0]!.dot(basis[2]!)) > 1e-4 ||
        Math.abs(basis[1]!.dot(basis[2]!)) > 1e-4 ||
        Math.abs(basis[0]!.clone().cross(basis[1]!).dot(basis[2]!) - 1) > 1e-4
      )
        throw new Error("Reviewed node pose must be rigid without reflection");
    }
  }
}
