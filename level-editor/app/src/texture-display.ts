import * as THREE from "three";

/** Display preferences do not change the source atlas or the saved map. */
export class TextureDisplay {
  smooth = true;
  readonly synthesized = { value: true };
  private readonly configured = new WeakSet<THREE.Material>();
  private readonly samplingConfigured = new WeakSet<THREE.Material>();
  private readonly nearestRequired = new WeakSet<THREE.Texture>();

  material(material: THREE.Material) {
    if (this.configured.has(material)) return;
    const foliage = material.userData.foliage_physical_opacity === true;
    if (foliage) {
      if (
        material.userData.opacity_semantics !== "physical-coverage" ||
        material.userData.source_ownership_semantics !== "separate-mask" ||
        material.userData.source_ownership_channel !== "vertex-color-r"
      )
        throw new Error(
          "Foliage requires explicit physical opacity and separate vertex ownership metadata",
        );
      const colored = material as THREE.MeshBasicMaterial;
      if (
        !colored.vertexColors ||
        !colored.map ||
        material.alphaTest !== 0.5 ||
        material.transparent ||
        material.side !==
          (material.userData.foliage_card_sides === "paired-one-sided"
            ? THREE.FrontSide
            : THREE.DoubleSide)
      )
        throw new Error(
          "Foliage requires COLOR_0 ownership, a base color map, MASK cutoff 0.5 and its declared card-sidedness",
        );
    } else if (material.userData.source_ownership_fill !== "synthesized") return;
    this.configured.add(material);
    const previous = material.onBeforeCompile.bind(material);
    material.onBeforeCompile = (shader, renderer) => {
      previous(shader, renderer);
      shader.uniforms.showSynthesized = this.synthesized;
      shader.fragmentShader = "uniform bool showSynthesized;\n" + shader.fragmentShader;
      if (foliage) {
        // COLOR_0 carries provenance only. Its alpha must not multiply coverage,
        // and its RGB must not tint the physical base-color texture.
        shader.fragmentShader = shader.fragmentShader.replace(
          "#include <color_fragment>",
          `float sourceOwnership = ${material.userData.source_ownership_backface === "inferred" ? "(gl_FrontFacing ? clamp(vColor.r, 0.0, 1.0) : 0.0)" : "clamp(vColor.r, 0.0, 1.0)"};
           if (!showSynthesized) diffuseColor.rgb = mix(vec3(0.24), diffuseColor.rgb, sourceOwnership);
           ${material.userData.foliage_backface_fill === "neutral" ? "if (!gl_FrontFacing) diffuseColor.rgb = vec3(0.24);" : ""}`,
        );
        return;
      }
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
    const previousKey = material.customProgramCacheKey();
    const contractKey = foliage
      ? `foliage-ownership-color-r-v1:${material.userData.source_ownership_backface}:${material.userData.foliage_backface_fill}`
      : "source-ownership-display-v1";
    material.customProgramCacheKey = () => `${previousKey}:${contractKey}`;
    material.needsUpdate = true;
  }

  apply(root: THREE.Object3D, maxAnisotropy = 1) {
    // Authored room floors sit just above retained floor shells. Their narrow
    // separation can lose the depth test in the editor's full-map projection.
    const roomFloor = (object: THREE.Object3D) =>
      /^patch-\d+-room-floor$/.test(object.userData.projection_component ?? "");
    const unrelated = new Set<THREE.Material>();
    root.traverse((object) => {
      if (!(object instanceof THREE.Mesh) || roomFloor(object)) return;
      for (const material of Array.isArray(object.material) ? object.material : [object.material])
        unrelated.add(material);
    });
    const textures = new Set<THREE.Texture>();
    root.traverse((object) => {
      if (!(object instanceof THREE.Mesh)) return;
      if (roomFloor(object)) {
        const offset = (material: THREE.Material) => {
          const target = unrelated.has(material) ? material.clone() : material;
          target.polygonOffset = true;
          target.polygonOffsetFactor = -2;
          target.polygonOffsetUnits = -2;
          return target;
        };
        object.material = Array.isArray(object.material)
          ? object.material.map(offset)
          : offset(object.material);
      }
      for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
        if (
          material.userData.foliage_physical_opacity === true &&
          !object.geometry.getAttribute("color")
        )
          throw new Error(
            `Foliage mesh ${object.name} is missing its separate ownership COLOR_0 attribute`,
          );
        this.material(material);
        if (
          material.userData.export_complementary_binary_ownership === true &&
          !this.samplingConfigured.has(material)
        ) {
          if (material.alphaTest !== 0.5 || material.transparent)
            throw new Error("Complementary ownership requires binary MASK cutoff 0.5");
          // Alpha is discrete ownership data; smoothing it mixes the two
          // coincident branches. RGB keeps the normal display preference.
          const masked = material as THREE.MeshStandardMaterial;
          if (!(masked.map instanceof THREE.Texture))
            throw new Error("Complementary ownership requires a binary mask texture");
          // Isolate the mask so shared RGB textures keep their own sampling.
          masked.map = masked.map.clone();
          this.nearestRequired.add(masked.map);
          this.samplingConfigured.add(material);
        }
        for (const value of Object.values(material)) {
          if (value instanceof THREE.Texture) textures.add(value);
        }
      }
    });
    for (const texture of textures) {
      const smooth = this.smooth && !this.nearestRequired.has(texture);
      const mag = smooth ? THREE.LinearFilter : THREE.NearestFilter;
      const min = smooth ? THREE.LinearMipmapLinearFilter : THREE.NearestFilter;
      const anisotropy = smooth ? maxAnisotropy : 1;
      if (
        texture.magFilter === mag &&
        texture.minFilter === min &&
        texture.anisotropy === anisotropy &&
        texture.generateMipmaps === smooth
      )
        continue;
      texture.magFilter = mag;
      texture.minFilter = min;
      texture.generateMipmaps = smooth;
      texture.anisotropy = anisotropy;
      texture.needsUpdate = true;
    }
  }
}
