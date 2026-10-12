import * as THREE from "three";

/** Upload unchanged candidate textures before they enter the live render loop. */
export async function prepareGpuTextures(
  roots: Iterable<THREE.Object3D>,
  renderer: Pick<THREE.WebGLRenderer, "initTexture">,
  checkCurrent: () => void,
  progress: (completed: number, total: number) => void,
  yieldTask: () => Promise<void> = () => new Promise((resolve) => setTimeout(resolve, 0)),
): Promise<void> {
  const textures = new Set<THREE.Texture>();
  for (const root of roots)
    root.traverse((node) => {
      const material = (node as THREE.Mesh).material;
      for (const item of material ? (Array.isArray(material) ? material : [material]) : []) {
        for (const value of Object.values(item))
          if (value instanceof THREE.Texture) textures.add(value);
        if (item instanceof THREE.ShaderMaterial)
          for (const uniform of Object.values(item.uniforms)) {
            const values = Array.isArray(uniform.value) ? uniform.value : [uniform.value];
            for (const value of values) if (value instanceof THREE.Texture) textures.add(value);
          }
      }
    });
  checkCurrent();
  progress(0, textures.size);
  // A task boundary lets loading UI, cancellation and browser automation run.
  await yieldTask();
  checkCurrent();
  let completed = 0;
  for (const texture of textures) {
    renderer.initTexture(texture);
    checkCurrent();
    progress(++completed, textures.size);
    await yieldTask();
    checkCurrent();
  }
}
