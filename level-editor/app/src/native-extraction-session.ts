import * as THREE from "three";
import { createNativeExtractionGeometrySnapshot } from "./native-extraction-snapshot.ts";
import { prepareNativeExtractionContext, type NativeExtractionContext } from "./native-extraction-context.ts";
import { nativeEffectCamera, renderPhysicalEffect, validateNativeEffectMaterial } from "./native-physical-effect-preview.ts";
import { hasQualifiedSourceDisplayShader, TextureDisplay } from "./texture-display.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
type Binding = MissionStateContract["targets"][number];

/** Explicit pinned-snapshot contract, not a replacement for strict prepare().
 * Later source-array writes cannot alter this render snapshot. Supported geometry,
 * material, and texture edits must invalidate and rebuild it before displaying edits.
 * Only transforms, visibility, and morph weights are read from the live hierarchy.
 */
export async function createNativeExtractionSession(
  context: NativeExtractionContext, object: THREE.Object3D, binding: Binding,
) {
  const pinnedBinding = JSON.stringify(binding);
  const prepared = prepareNativeExtractionContext(context, object, binding);
  const geometries = new Map<THREE.BufferGeometry, THREE.BufferGeometry>();
  const snapshotGeometry = createNativeExtractionGeometrySnapshot();
  const materials = new Map<THREE.Material, THREE.Material>();
  const textures = new Map<THREE.Texture, THREE.Texture>();
  const sources = new Map<THREE.Texture["source"], THREE.Texture["source"]>();
  const bitmaps: ImageBitmap[] = [];
  let retired: string | undefined;
  const dispose = () => {
    for (const geometry of geometries.values()) geometry.dispose();
    for (const material of materials.values()) material.dispose();
    for (const texture of textures.values()) texture.dispose();
    for (const bitmap of bitmaps) bitmap.close();
    prepared.dispose();
  };
  const textureCopy = async (texture: THREE.Texture) => {
    const known = textures.get(texture); if (known) return known;
    if (texture.mipmaps.length) throw new Error("Extraction session requires decoded base textures");
    let source = sources.get(texture.source);
    if (!source) {
      const image = texture.source.data;
      let copy: unknown;
      if (image && typeof image === "object" && "data" in image && ArrayBuffer.isView(image.data)) {
        if (image.data instanceof DataView) throw new Error("Extraction session requires typed texture pixels");
        copy = { ...image, data: (image.data as THREE.TypedArray).slice() };
      } else if (image && typeof createImageBitmap === "function") {
        const bitmap = await createImageBitmap(image as ImageBitmapSource, { premultiplyAlpha: "none", colorSpaceConversion: "none" });
        bitmaps.push(bitmap); copy = bitmap;
      } else throw new Error("Extraction session cannot isolate texture pixels");
      source = new THREE.Source(copy); sources.set(texture.source, source);
    }
    const copy = texture.clone(); copy.source = source; copy.needsUpdate = true;
    textures.set(texture, copy); return copy;
  };
  const materialCopy = async (material: THREE.Material, authority: THREE.Material) => {
    const known = materials.get(material); if (known) return known;
    if (!hasQualifiedSourceDisplayShader(authority))
      throw new Error("Extraction session requires a qualified source shader");
    const copy = authority.clone();
    // Own the clone before asynchronous texture isolation can fail.
    materials.set(material, copy);
    copy.colorWrite = material.colorWrite;
    for (const [key, value] of Object.entries(authority))
      if (value instanceof THREE.Texture) (copy as any)[key] = await textureCopy(value);
    const display = new TextureDisplay(); display.material(copy);
    try { validateNativeEffectMaterial(copy); } catch (error) {
      throw new Error(`Extraction session material ${material.name}: ${error}`, { cause: error });
    }
    return copy;
  };
  const original: THREE.Object3D[] = [], owners: THREE.Object3D[] = [], receivers: THREE.Object3D[] = [];
  object.traverse(node => original.push(node));
  prepared.owner.traverse(node => owners.push(node));
  prepared.depth.traverse(node => receivers.push(node));
  const children = original.map(node => [...node.children]);
  const meshInputs = original.map(node => node instanceof THREE.Mesh
    ? { geometry: node.geometry, material: node.material, weights: node.morphTargetInfluences?.length } : undefined);
  try {
    // Copy every vertex buffer synchronously before any image-decoding await.
    for (const node of [...owners, ...receivers]) if (node instanceof THREE.Mesh) {
      let geometry = geometries.get(node.geometry);
      if (!geometry) {
        geometry = snapshotGeometry(node.geometry);
        geometry.computeBoundingBox(); geometry.computeBoundingSphere();
        geometries.set(node.geometry, geometry);
      }
      node.geometry = geometry;
    }
    for (const nodes of [owners, receivers]) for (const [index, node] of nodes.entries()) if (node instanceof THREE.Mesh) {
      const authority = (original[index] as THREE.Mesh).material;
      if (Array.isArray(node.material)) {
        const copies: THREE.Material[] = [];
        for (const [i, material] of node.material.entries())
          copies.push(await materialCopy(material, Array.isArray(authority) ? authority[i]! : authority));
        node.material = copies;
      } else node.material = await materialCopy(node.material, Array.isArray(authority) ? authority[0]! : authority);
    }
    // Reject edits racing asynchronous texture isolation before making the session usable.
    prepareNativeExtractionContext(context, object, binding).dispose();
  } catch (error) { dispose(); throw error; }
  const ownerScene = new THREE.Scene(), depthScene = new THREE.Scene();
  ownerScene.add(prepared.owner); depthScene.add(prepared.depth);
  const synchronize = () => {
    if (retired) throw new Error(`Extraction session retired: ${retired}`);
    if (JSON.stringify(binding) !== pinnedBinding) throw new Error("Extraction session binding changed");
    object.updateWorldMatrix(true, true);
    original.forEach((source, index) => {
      if (source.children.length !== children[index]!.length || source.children.some((child, i) => child !== children[index]![i]))
        throw new Error("Extraction session hierarchy changed; invalidate and rebuild");
      const mesh = meshInputs[index];
      if (mesh && source instanceof THREE.Mesh && (source.geometry !== mesh.geometry || source.material !== mesh.material || source.morphTargetInfluences?.length !== mesh.weights))
        throw new Error("Extraction session resources changed; invalidate and rebuild");
      for (const target of [owners[index]!, receivers[index]!]) {
        target.matrixAutoUpdate = false;
        target.matrix.copy(index === 0 ? source.matrixWorld : source.matrix);
        target.visible = source.visible;
        if (source instanceof THREE.Mesh && target instanceof THREE.Mesh && source.morphTargetInfluences) {
          if (source.morphTargetInfluences.some(value => !Number.isFinite(value))) throw new Error("Extraction session invalid morph pose");
          target.morphTargetInfluences = [...source.morphTargetInfluences];
        }
      }
    });
    ownerScene.updateMatrixWorld(true); depthScene.updateMatrixWorld(true);
  };
  const invalidate = (reason: string) => {
    if (retired) return;
    retired = reason || "explicit invalidation";
    dispose();
  };
  return Object.freeze({
    synchronize,
    render(renderer: THREE.WebGLRenderer, elevation: number, frame?: { x: number; y: number; width: number; height: number }) {
      synchronize();
      const view = nativeEffectCamera(new THREE.Box3().setFromObject(prepared.owner), elevation, frame);
      return { ...renderPhysicalEffect(renderer, ownerScene, view.camera, view.width, view.height, false, depthScene), x: view.x, y: view.y };
    },
    invalidate,
    dispose: () => invalidate("disposed"),
  });
}
