import * as THREE from "three";
import {
  validateReviewedNodePoses,
  type ReviewedNodePose,
} from "../../shared/src/reviewed-node-poses.ts";
export { validateReviewedNodePoses } from "../../shared/src/reviewed-node-poses.ts";

/** Applies absolute reviewed deltas without accumulating transforms or changing owned resources. */
export class ReviewedNodePoses {
  private readonly frames: ReviewedNodePose[][];
  private readonly bindings: { object: THREE.Object3D; matrix: THREE.Matrix4; auto: boolean }[];
  private readonly delta = new THREE.Matrix4();

  constructor(root: THREE.Object3D, frames: ReviewedNodePose[][]) {
    validateReviewedNodePoses(frames);
    this.frames = structuredClone(frames);
    const named = new Map<string, THREE.Object3D[]>();
    root.traverse((object) => {
      const rows = named.get(object.name) ?? [];
      rows.push(object);
      named.set(object.name, rows);
    });
    const objects = frames[0]!.map(({ node }) => {
      const rows = named.get(node);
      if (rows?.length !== 1) throw new Error(`Reviewed node missing or ambiguous: ${node}`);
      if (!(rows[0] as THREE.Mesh).isMesh) throw new Error(`Reviewed node is not a mesh: ${node}`);
      return rows[0]!;
    });
    const selected = new Set(objects);
    for (const object of objects)
      for (let parent = object.parent; parent; parent = parent.parent)
        if (selected.has(parent)) throw new Error("Reviewed nodes must not be nested");
    this.bindings = objects.map((object) => {
      if (object.matrixAutoUpdate) object.updateMatrix();
      return { object, matrix: object.matrix.clone(), auto: object.matrixAutoUpdate };
    });
  }

  apply(frame: number): void {
    if (!Number.isSafeInteger(frame) || frame < 0 || frame >= this.frames.length)
      throw new Error("Invalid reviewed node frame");
    this.bindings.forEach(({ object, matrix }, index) => {
      object.matrixAutoUpdate = false;
      object.matrix.copy(this.delta.fromArray(this.frames[frame]![index]!.matrix).multiply(matrix));
      object.matrixWorldNeedsUpdate = true;
    });
  }

  reset(): void {
    for (const { object, matrix, auto } of this.bindings) {
      object.matrix.copy(matrix);
      object.matrixAutoUpdate = auto;
      object.matrixWorldNeedsUpdate = true;
    }
  }
}
