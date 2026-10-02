import * as THREE from "three";
import { gameToScene, terrainTriangles, type Level3D, type TerrainTriangle } from "@rle/shared";
import { terrainMaterialMixTexture } from "./terrain-material-mix.ts";
import { terrainMaterialTexture } from "./terrain-texture.ts";

/** Shared evaluated triangles keep editor appearance and exported receiving geometry aligned. */
export class TerrainLayer {
  readonly root = new THREE.Group();
  private terrain: Level3D["terrain"];
  private splines: Level3D["splines"];
  private camera: Level3D["camera"] | undefined;
  private customMaterials: Level3D["customMaterials"];
  private resources: THREE.Material[] = [];
  private textures: THREE.Texture[] = [];
  private textureCache = new Map<string, THREE.DataTexture>();
  private mixtureCache = new Map<string, ReturnType<typeof terrainMaterialMixTexture>>();
  private materialCache = new Map<string, THREE.MeshLambertMaterial>();
  private triangles: TerrainTriangle[] | undefined;
  private sectionCells: import("@rle/shared").TerrainCell[] | undefined;
  private sections = new Map<string, number>();
  private materialIds = new Map<string, number>();
  private batches = new Map<string, { triangles: TerrainTriangle[]; mesh: THREE.Mesh }>();
  private triangleMaterials = new WeakMap<
    TerrainTriangle,
    { key: string; mixes: Record<string, number>[]; section?: number; batchKey?: string }
  >();
  private image: THREE.Texture | undefined;
  sync(document: Level3D) {
    if (
      this.terrain === document.terrain &&
      this.splines === document.splines &&
      this.camera === document.camera &&
      this.customMaterials === document.customMaterials
    )
      return false;
    const triangles = terrainTriangles(document);
    if (
      triangles === this.triangles &&
      this.camera === document.camera &&
      this.customMaterials === document.customMaterials &&
      this.terrain?.texture === document.terrain?.texture
    ) {
      this.terrain = document.terrain;
      this.splines = document.splines;
      return false;
    }
    // Height-only edits reuse appearance resources. Bound caches during long painting sessions.
    if (
      this.customMaterials !== document.customMaterials ||
      this.terrain?.texture !== document.terrain?.texture ||
      this.materialCache.size > 512 ||
      this.mixtureCache.size > 1024
    )
      this.clear();
    const previousBatches = this.batches;
    this.batches = new Map();
    const textures = this.textureCache;
    const texture = (id: string) => {
      let t = textures.get(id);
      if (!t) {
        t = terrainMaterialTexture(id, document.customMaterials);
        textures.set(id, t);
        this.textures.push(t);
      }
      return t;
    };
    const mixtures = this.mixtureCache;
    const mixture = (mix: Record<string, number>) => {
      const key = JSON.stringify(Object.entries(mix).sort(([a], [b]) => a.localeCompare(b)));
      let result = mixtures.get(key);
      if (!result) {
        result = terrainMaterialMixTexture(mix, texture);
        mixtures.set(key, result);
        if (result.owned) this.textures.push(result.texture);
      }
      return result;
    };
    if (document.terrain?.texture && !this.image) {
      this.image = new THREE.TextureLoader().load(document.terrain.texture);
      this.image.colorSpace = THREE.SRGBColorSpace;
      this.textures.push(this.image);
    }
    const image = this.image;
    const batches = new Map<
      string,
      { materialKey: string; mixes: Record<string, number>[]; triangles: TerrainTriangle[] }
    >();
    // Small sections limit geometry uploads without creating a draw call per cell.
    if (this.sectionCells !== document.terrain?.cells) {
      this.sectionCells = document.terrain?.cells;
      this.sections = new Map(
        this.sectionCells?.map((cell, index) => [cell.id, Math.floor(index / 32)]),
      );
    }
    for (const t of triangles) {
      let entry = this.triangleMaterials.get(t);
      if (!entry) {
        const mixes =
          t.materialMixes ??
          (t.materials ?? [t.cell.material, t.cell.material, t.cell.material]).map((id) => ({
            [id]: 1,
          }));
        entry = {
          mixes,
          key: JSON.stringify(
            mixes.map((m) => Object.entries(m).sort(([a], [b]) => a.localeCompare(b))),
          ),
        };
        this.triangleMaterials.set(t, entry);
      }
      const section = this.sections.get(t.cell.id);
      if (entry.batchKey === undefined || entry.section !== section) {
        let materialId = this.materialIds.get(entry.key);
        if (materialId === undefined) {
          materialId = this.materialIds.size;
          this.materialIds.set(entry.key, materialId);
        }
        entry.section = section;
        entry.batchKey = `${section}/${materialId}`;
      }
      const key = entry.batchKey;
      let batch = batches.get(key);
      if (!batch) {
        batch = { materialKey: entry.key, mixes: entry.mixes, triangles: [] };
        batches.set(key, batch);
      }
      batch.triangles.push(t);
    }
    for (const [key, batch] of batches) {
      const previous = previousBatches.get(key);
      if (
        previous &&
        this.camera === document.camera &&
        previous.triangles.length === batch.triangles.length &&
        batch.triangles.every((t, i) => t === previous.triangles[i])
      ) {
        this.batches.set(key, previous);
        previousBatches.delete(key);
        continue;
      }
      const count = batch.triangles.length;
      const b = {
        positions: new Float32Array(count * 9),
        uvs: new Float32Array(count * 6),
        surfaceUvs: new Float32Array(count * 6),
        weights: new Float32Array(count * 9),
        cells: new Array<string>(count),
      };
      for (let triangle = 0; triangle < count; triangle++) {
        const t = batch.triangles[triangle]!;
        b.cells[triangle] = t.cell.id;
        for (let vertex = 0; vertex < 3; vertex++) {
          const point = t.points[vertex]!,
            positionOffset = triangle * 9 + vertex * 3,
            uvOffset = triangle * 6 + vertex * 2;
          b.positions.set(gameToScene(document.camera, ...point), positionOffset);
          for (let k = 0; k < 3; k++)
            b.weights[positionOffset + k] =
              t.materialWeights?.[vertex]?.[k] ?? (vertex === k ? 1 : 0);
          b.surfaceUvs[uvOffset] = point[0] / 1024;
          b.surfaceUvs[uvOffset + 1] = point[1] / 1024;
          const uv =
            image && (t.uv?.[vertex] ?? document.terrain!.vertices[t.indices[vertex]!]!.uv);
          b.uvs[uvOffset] = uv ? uv[0] : point[0] / 1024;
          b.uvs[uvOffset + 1] = uv ? uv[1] : point[1] / 1024;
        }
      }
      let material = this.materialCache.get(batch.materialKey);
      if (!material) {
        const mixed = batch.mixes.map(mixture),
          maps = mixed.map((m) => m.texture);
        material = new THREE.MeshLambertMaterial({
          map: maps[0],
          side: THREE.DoubleSide,
        });
        {
          material.userData.terrainMaterialBlend = true;
          material.onBeforeCompile = (shader) => {
            shader.uniforms.terrainSource = {
              value: new THREE.Vector3(
                ...(mixed.map((m) => m.sourceWeight) as [number, number, number]),
              ),
            };
            shader.uniforms.terrainImage = { value: image ?? maps[0] };
            shader.uniforms.terrainMapA = { value: maps[0] };
            shader.uniforms.terrainMapB = { value: maps[1] };
            shader.uniforms.terrainMapC = { value: maps[2] };
            shader.vertexShader =
              "attribute vec3 terrainWeights; attribute vec2 terrainSurfaceUv; varying vec2 vTerrainSurfaceUv; varying vec3 vTerrainWeights;\n" +
              shader.vertexShader.replace(
                "#include <begin_vertex>",
                "#include <begin_vertex>\nvTerrainWeights=terrainWeights;vTerrainSurfaceUv=terrainSurfaceUv;",
              );
            shader.fragmentShader =
              "varying vec2 vTerrainSurfaceUv; uniform vec3 terrainSource; varying vec3 vTerrainWeights; uniform sampler2D terrainMapA; uniform sampler2D terrainMapB; uniform sampler2D terrainMapC; uniform sampler2D terrainImage;\n" +
              shader.fragmentShader.replace(
                "#include <map_fragment>",
                "diffuseColor *= vec4(mix(texture2D(terrainMapA,vTerrainSurfaceUv).rgb,texture2D(terrainImage,vMapUv).rgb,terrainSource.x)*vTerrainWeights.x + mix(texture2D(terrainMapB,vTerrainSurfaceUv).rgb,texture2D(terrainImage,vMapUv).rgb,terrainSource.y)*vTerrainWeights.y + mix(texture2D(terrainMapC,vTerrainSurfaceUv).rgb,texture2D(terrainImage,vMapUv).rgb,terrainSource.z)*vTerrainWeights.z, 1.0);",
              );
            // Apply shadow strength once to the complete lit color, including ambient light.
            // Match black overlay blending in output color space, while retaining normal bias.
            shader.fragmentShader = shader.fragmentShader
              .replace(
                "#include <shadowmap_pars_fragment>",
                "#include <shadowmap_pars_fragment>\n#include <shadowmask_pars_fragment>",
              )
              .replace(
                "#include <lights_fragment_begin>",
                THREE.ShaderChunk.lights_fragment_begin.replaceAll("&& receiveShadow", "&& false"),
              )
              .replace(
                "#include <colorspace_fragment>",
                "#include <colorspace_fragment>\ngl_FragColor.rgb *= getShadowMask();",
              );
          };
          material.customProgramCacheKey = () => "terrain-blend-v2";
        }
        this.resources.push(material);
        this.materialCache.set(batch.materialKey, material);
      }
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.BufferAttribute(b.positions, 3));
      geometry.setAttribute("uv", new THREE.BufferAttribute(b.uvs, 2));
      geometry.setAttribute("terrainSurfaceUv", new THREE.BufferAttribute(b.surfaceUvs, 2));
      geometry.setAttribute("terrainWeights", new THREE.BufferAttribute(b.weights, 3));
      // Map-to-scene conversion reverses Y. Keep terrain faces pointing upward so
      // the shadow normal bias samples above the surface rather than beneath it.
      const indices: number[] = [];
      for (let i = 0; i < b.positions.length; i += 9) {
        const x = b.positions[i]!,
          y = b.positions[i + 1]!;
        const crossZ =
          (b.positions[i + 3]! - x) * (b.positions[i + 7]! - y) -
          (b.positions[i + 4]! - y) * (b.positions[i + 6]! - x);
        const vertex = i / 3;
        indices.push(vertex, vertex + (crossZ < 0 ? 2 : 1), vertex + (crossZ < 0 ? 1 : 2));
      }
      geometry.setIndex(indices);
      geometry.computeVertexNormals();
      const mesh = new THREE.Mesh(geometry, material);
      mesh.userData.terrainSurface = true;
      mesh.userData.terrainCells = b.cells;
      mesh.castShadow = true;
      mesh.receiveShadow = true;
      this.root.add(mesh);
      this.batches.set(key, { triangles: batch.triangles, mesh });
    }
    for (const { mesh } of previousBatches.values()) {
      mesh.removeFromParent();
      mesh.geometry.dispose();
    }
    this.triangles = triangles;
    this.terrain = document.terrain;
    this.camera = document.camera;
    this.splines = document.splines;
    this.customMaterials = document.customMaterials;
    return true;
  }
  clear() {
    this.root.traverse((node) => {
      if (node instanceof THREE.Mesh) node.geometry.dispose();
    });
    this.root.clear();
    for (const m of this.resources) m.dispose();
    for (const t of this.textures) t.dispose();
    this.resources = [];
    this.textures = [];
    this.textureCache.clear();
    this.mixtureCache.clear();
    this.materialCache.clear();
    this.batches.clear();
    this.triangleMaterials = new WeakMap();
    this.image = undefined;
    this.terrain = undefined;
    this.triangles = undefined;
    this.sectionCells = undefined;
    this.sections.clear();
    this.materialIds.clear();
    this.camera = undefined;
    this.splines = undefined;
    this.customMaterials = undefined;
  }
}
