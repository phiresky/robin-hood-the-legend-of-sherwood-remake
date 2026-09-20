import * as THREE from "three";

/** Display preferences do not change the source atlas or the saved map. */
export class TextureDisplay {
  smooth = true;
  readonly synthesized = { value: true };
  private readonly configured = new WeakSet<THREE.Material>();

  material(material: THREE.Material) {
    if (material.userData.source_ownership_fill !== "synthesized" || this.configured.has(material)) return;
    this.configured.add(material);
    const previous = material.onBeforeCompile;
    material.onBeforeCompile = (shader, renderer) => {
      previous.call(material, shader, renderer);
      shader.uniforms.showSynthesized = this.synthesized;
      shader.fragmentShader = "uniform bool showSynthesized;\n" + shader.fragmentShader;
      shader.fragmentShader = shader.fragmentShader.replace(
        "#include <map_fragment>",
        THREE.ShaderChunk.map_fragment.replace(
          "diffuseColor *= sampledDiffuseColor;",
          // Alpha encodes source ownership, not surface transparency. The shade
          // is linear, matching the neutral source-only projection material.
          "if (!showSynthesized) sampledDiffuseColor.rgb = mix(vec3(0.24), sampledDiffuseColor.rgb, sampledDiffuseColor.a);\n" +
          "sampledDiffuseColor.a = 1.0;\ndiffuseColor *= sampledDiffuseColor;",
        ),
      );
    };
    material.customProgramCacheKey = () => "source-ownership-display-v1";
    material.needsUpdate = true;
  }

  apply(root: THREE.Object3D, maxAnisotropy = 1) {
    const textures = new Set<THREE.Texture>();
    root.traverse((object) => {
      if (!(object instanceof THREE.Mesh)) return;
      for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
        this.material(material);
        for (const value of Object.values(material)) {
          if (value instanceof THREE.Texture) textures.add(value);
        }
      }
    });
    for (const texture of textures) {
      const mag = this.smooth ? THREE.LinearFilter : THREE.NearestFilter;
      const min = this.smooth ? THREE.LinearMipmapLinearFilter : THREE.NearestFilter;
      const anisotropy = this.smooth ? maxAnisotropy : 1;
      if (texture.magFilter === mag && texture.minFilter === min && texture.anisotropy === anisotropy) continue;
      texture.magFilter = mag;
      texture.minFilter = min;
      texture.generateMipmaps = this.smooth;
      texture.anisotropy = anisotropy;
      texture.needsUpdate = true;
    }
  }
}
