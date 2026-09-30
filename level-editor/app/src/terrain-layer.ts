import * as THREE from "three";
import { gameToScene, terrainTriangles, type Level3D } from "@rle/shared";
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
  sync(document: Level3D) {
    if (
      this.terrain === document.terrain &&
      this.splines === document.splines &&
      this.camera === document.camera &&
      this.customMaterials === document.customMaterials
    )
      return false;
    this.clear();
    const textures = new Map<string, THREE.DataTexture>();
    const texture = (id: string) => {
      let t = textures.get(id);
      if (!t) {
        t = terrainMaterialTexture(id, document.customMaterials);
        textures.set(id, t);
        this.textures.push(t);
      }
      return t;
    };
    const mixtures = new Map<string, ReturnType<typeof terrainMaterialMixTexture>>();
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
    let image: THREE.Texture | undefined;
    if (document.terrain?.texture) {
      image = new THREE.TextureLoader().load(document.terrain.texture);
      image.colorSpace = THREE.SRGBColorSpace;
      this.textures.push(image);
    }
    const batches = new Map<
      string,
      {
        mixes: Record<string, number>[];
        positions: number[];
        uvs: number[];
        surfaceUvs: number[];
        weights: number[];
        cells: string[];
      }
    >();
    for (const t of terrainTriangles(document)) {
      const mixes =
          t.materialMixes ??
          (t.materials ?? [t.cell.material, t.cell.material, t.cell.material]).map((id) => ({
            [id]: 1,
          })),
        key = JSON.stringify(
          mixes.map((m) => Object.entries(m).sort(([a], [b]) => a.localeCompare(b))),
        );
      let b = batches.get(key);
      if (!b) {
        b = { mixes, positions: [], uvs: [], surfaceUvs: [], weights: [], cells: [] };
        batches.set(key, b);
      }
      b.positions.push(...t.points.flatMap((p) => gameToScene(document.camera, ...p)));
      b.weights.push(...(t.materialWeights?.flat() ?? [1, 0, 0, 0, 1, 0, 0, 0, 1]));
      b.cells.push(t.cell.id);
      b.surfaceUvs.push(...t.points.flatMap((p) => [p[0] / 1024, p[1] / 1024]));
      b.uvs.push(
        ...t.indices.flatMap((index, i) =>
          image && (t.uv?.[i] ?? document.terrain!.vertices[index]?.uv)
            ? (t.uv?.[i] ?? document.terrain!.vertices[index]!.uv!)
            : [t.points[i]![0] / 1024, t.points[i]![1] / 1024],
        ),
      );
    }
    for (const b of batches.values()) {
      const mixed = b.mixes.map(mixture),
        maps = mixed.map((m) => m.texture);
      const material = new THREE.MeshBasicMaterial({
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
        };
        material.customProgramCacheKey = () => "terrain-blend-v1";
      }
      this.resources.push(material);
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.Float32BufferAttribute(b.positions, 3));
      geometry.setAttribute("uv", new THREE.Float32BufferAttribute(b.uvs, 2));
      geometry.setAttribute("terrainSurfaceUv", new THREE.Float32BufferAttribute(b.surfaceUvs, 2));
      geometry.setAttribute("terrainWeights", new THREE.Float32BufferAttribute(b.weights, 3));
      geometry.computeVertexNormals();
      const mesh = new THREE.Mesh(geometry, material);
      mesh.userData.terrainSurface = true;
      mesh.userData.terrainCells = b.cells;
      mesh.userData.noSunShadow = true;
      this.root.add(mesh);
    }
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
    this.terrain = undefined;
    this.camera = undefined;
    this.splines = undefined;
    this.customMaterials = undefined;
  }
}
