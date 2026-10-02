import * as THREE from "three";

/** Frozen mesh bounds keep camera fitting independent of mesh density. */
export class FramingBounds implements Iterable<THREE.Vector3> {
  private readonly instances: { bounds: THREE.Box3; matrix: THREE.Matrix4 }[] = [];
  readonly length: number;

  constructor(roots: Iterable<THREE.Object3D> = []) {
    const bounds = new Map<THREE.BufferAttribute | THREE.InterleavedBufferAttribute, THREE.Box3>();
    for (const root of roots) {
      root.updateWorldMatrix(true, true);
      root.traverseVisible((node) => {
        if (!(node instanceof THREE.Mesh)) return;
        const attribute = node.geometry.getAttribute("position");
        if (!attribute?.count) return;
        let box = bounds.get(attribute);
        if (!box) {
          box = new THREE.Box3().setFromBufferAttribute(attribute);
          bounds.set(attribute, box);
        }
        this.instances.push({ bounds: box, matrix: node.matrixWorld.clone() });
      });
    }
    this.length = this.instances.length * 8;
  }

  /** The yielded vector is reused; consumers retaining a corner must copy it. */
  *[Symbol.iterator](): IterableIterator<THREE.Vector3> {
    const point = new THREE.Vector3();
    for (const { bounds, matrix } of this.instances)
      for (const x of [bounds.min.x, bounds.max.x])
        for (const y of [bounds.min.y, bounds.max.y])
          for (const z of [bounds.min.z, bounds.max.z])
            yield point.set(x, y, z).applyMatrix4(matrix);
  }
}
