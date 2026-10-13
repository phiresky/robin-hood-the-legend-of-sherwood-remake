import * as THREE from "three";

const additional = new WeakMap<THREE.Object3D, readonly THREE.Material[]>();
export function retainObjectMaterials(root: THREE.Object3D, materials: readonly THREE.Material[]) {
  additional.set(root, materials);
}

/** Call only on asset owners, never on editable clones sharing their resources. */
export function disposeObjectResources(roots: Iterable<THREE.Object3D>) {
  const geometries = new Set<THREE.BufferGeometry>();
  const materials = new Set<THREE.Material>();
  const textures = new Set<THREE.Texture>();
  const bitmaps = new Set<ImageBitmap>();
  for (const root of roots)
    root.traverse((node) => {
      for (const material of additional.get(node) ?? []) materials.add(material);
      const mesh = node as THREE.Mesh;
      if (mesh.geometry) geometries.add(mesh.geometry);
      if (mesh.material)
        for (const material of Array.isArray(mesh.material) ? mesh.material : [mesh.material])
          materials.add(material);
    });
  for (const material of materials)
    for (const value of Object.values(material))
      if (value instanceof THREE.Texture) textures.add(value);
  for (const texture of textures) {
    if (typeof ImageBitmap !== "undefined" && texture.source.data instanceof ImageBitmap)
      bitmaps.add(texture.source.data);
    texture.dispose();
  }
  for (const bitmap of bitmaps) bitmap.close();
  for (const material of materials) material.dispose();
  for (const geometry of geometries) geometry.dispose();
}
