import * as THREE from "three";
import type { PhysicalMaterialFrame } from "../../shared/src/state-delivery.ts";
export type LoadedPhaseMaterial = {
  material: THREE.Material;
  name: string;
  texture_index: number;
  tex_coord: number;
  alpha_mode: string;
};
const retained = new WeakMap<THREE.Object3D, ReadonlyMap<number, LoadedPhaseMaterial>>();
export function retainPhaseMaterials(
  root: THREE.Object3D,
  materials: ReadonlyMap<number, LoadedPhaseMaterial>,
) {
  retained.set(root, materials);
}
/** Discrete source appearance accompanies the existing geometry player's clock. */
export class StateMaterialFrames {
  private frames: {
    end: number;
    bindings: { mesh: THREE.Mesh; slot: number; material: THREE.Material }[];
  }[] = [];
  private originals = new Map<THREE.Mesh, THREE.Material | THREE.Material[]>();
  private owned = new Set<THREE.Material>();
  private disposed = false;
  constructor(
    source: THREE.Object3D,
    content: THREE.Object3D,
    frames: readonly PhysicalMaterialFrame[],
  ) {
    const materials = retained.get(source);
    if (!materials) throw new Error("Pinned phase materials were not loaded");
    let end = 0;
    try {
      this.frames = frames.map((frame) => ({
        end: (end += frame.duration_ticks),
        bindings: frame.materials.map((binding) => {
          const matches: THREE.Mesh[] = [];
          content.traverse((node) => {
            if (node.name === binding.node && (node as THREE.Mesh).isMesh)
              matches.push(node as THREE.Mesh);
          });
          if (matches.length !== 1) throw new Error("Missing or ambiguous material frame mesh");
          const mesh = matches[0]!;
          const slots = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
          if (!slots[binding.slot]) throw new Error("Material frame slot does not exist");
          const loaded = materials.get(binding.material_index);
          if (
            !loaded ||
            loaded.name !== binding.material_name ||
            loaded.texture_index !== binding.texture_index ||
            loaded.tex_coord !== binding.tex_coord ||
            loaded.alpha_mode !== binding.alpha_mode
          )
            throw new Error("Material frame identity differs from pinned model");
          const map = (loaded.material as THREE.MeshStandardMaterial).map;
          const uv = binding.tex_coord === 0 ? "uv" : `uv${binding.tex_coord}`;
          if (!map || map.channel !== binding.tex_coord || !mesh.geometry.getAttribute(uv))
            throw new Error("Material frame texture or UV binding differs from model");
          if (!this.originals.has(mesh)) this.originals.set(mesh, mesh.material);
          const material = loaded.material.clone();
          // Match geometry-dependent loader variants for the receiving primitive.
          material.vertexColors = !!mesh.geometry.getAttribute("color");
          if ("flatShading" in material)
            material.flatShading = !mesh.geometry.getAttribute("normal");
          this.owned.add(material);
          return { mesh, slot: binding.slot, material };
        }),
      }));
      this.sample(0);
    } catch (error) {
      this.dispose();
      throw error;
    }
  }
  sample(tick: number) {
    if (this.disposed) throw new Error("Material frames are disposed");
    if (!Number.isSafeInteger(tick) || tick < 0) throw new Error("Invalid material frame tick");
    const frame = this.frames.find((f) => tick < f.end) ?? this.frames.at(-1);
    if (!frame) throw new Error("Material frames are empty");
    for (const { mesh, slot, material } of frame.bindings) {
      if (Array.isArray(mesh.material)) {
        const slots = mesh.material.slice();
        slots[slot] = material;
        mesh.material = slots;
      } else mesh.material = material;
    }
  }
  dispose() {
    if (this.disposed) return;
    for (const [mesh, material] of this.originals) mesh.material = material;
    for (const material of this.owned) material.dispose();
    this.owned.clear();
    this.disposed = true;
  }
}
