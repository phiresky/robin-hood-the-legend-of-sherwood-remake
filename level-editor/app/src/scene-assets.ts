import { NativeExtractionLoader, type NativeExtractionPin } from "./native-extraction-loader.ts";
import type { MissionStateContract } from "../../shared/src/mission-state.ts";
import { retainPhaseMaterials, type LoadedPhaseMaterial } from "./state-material-frames.ts";
import { disposeObjectResources, retainObjectMaterials } from "./resources.ts";
import * as THREE from "three";
import { createGltfLoader } from "./gltf-loader.ts";
import {
  safeLibraryPath,
  selectGlbScene,
  selectGltfScene,
  resolveGltfResources,
  type SceneAssetSource,
} from "@rle/shared";
import { subdir } from "./fs.ts";
import { readLossyModel, lossyApplies } from "./lossy-models.ts";

import { captureStateAppearance, type StateAppearanceTemplate } from "./state-appearance-player.ts";

/** Retain selected-scene clips before display names replace loader binding names. */
export function retainSceneAnimations(
  scene: THREE.Object3D,
  clips: readonly THREE.AnimationClip[] = [],
  loadedScenes: readonly THREE.Object3D[] = [scene],
) {
  const selected = new Set<THREE.Object3D>();
  scene.traverse((node) => selected.add(node));
  const all = new Set<THREE.Object3D>(selected);
  for (const root of loadedScenes) root.traverse((node) => all.add(node));
  scene.animations = clips.flatMap((clip) => {
    const tracks = clip.tracks.flatMap((original) => {
      const parsed = THREE.PropertyBinding.parseTrackName(original.name);
      const name = parsed.nodeName;
      const matches =
        !name || name === "."
          ? [scene]
          : [...all].filter((node) => node.name === name || node.uuid === name);
      if (matches.length !== 1)
        throw new Error(`Missing or ambiguous loaded animation target: ${original.name}`);
      const target = matches[0]!;
      // A shared GLB may contain clips for another selected scene.
      if (!selected.has(target)) return [];
      const prefix = !name || name === "." ? "" : name;
      if (!original.name.startsWith(prefix + "."))
        throw new Error(`Unsupported loaded animation binding: ${original.name}`);
      const track = original.clone();
      track.name = target.uuid + original.name.slice(prefix.length);
      return [track];
    });
    return tracks.length
      ? [new THREE.AnimationClip(clip.name, clip.duration, tracks, clip.blendMode)]
      : [];
  });
}

/** Capture an extracted group using retained UUID bindings; static assets have no player. */
export function captureLoadedStateAppearance(
  scene: THREE.Object3D,
  selectedRoot: THREE.Object3D = scene,
): StateAppearanceTemplate | undefined {
  return scene.animations.length
    ? captureStateAppearance(selectedRoot, scene.animations)
    : undefined;
}

async function read(root: FileSystemDirectoryHandle, path: string) {
  if (!safeLibraryPath(path)) throw new Error(`Unsafe scene asset path: ${path}`);
  const parts = path.split("/");
  const name = parts.pop()!;
  const directory = await subdir(root, parts);
  if (!directory) throw new Error(`Missing scene asset directory: ${path}`);
  return (await directory.getFileHandle(name)).getFile();
}
async function checked(file: File, expected: string) {
  const bytes = await file.arrayBuffer();
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  if (hash !== expected) throw new Error(`Scene asset changed: ${file.name}`);
  return bytes;
}

/** One load owns shared payload URLs until every asset has finished decoding. */
export class SceneAssetLoader {
  private resources = new Map<string, Promise<string>>();
  private urls: string[] = [];
  private textures = new Map<string, Promise<THREE.Texture>>();
  private materials = new Map<string, Promise<THREE.Material>>();
  private finalMaterials = new Map<string, THREE.Material>();
  private root: FileSystemDirectoryHandle;
  private extraction: NativeExtractionLoader;
  private extractionPins = new WeakMap<THREE.Object3D, NativeExtractionPin>();
  private lossyModels: ReadonlyMap<string, string>;
  /** `lossyModels` maps pinned model paths to derived lossy models (see lossy-models.ts). */
  constructor(
    root: FileSystemDirectoryHandle,
    lossyModels: ReadonlyMap<string, string> = new Map(),
  ) {
    this.root = root;
    this.extraction = new NativeExtractionLoader(
      async (path) => new Uint8Array(await (await read(root, path)).arrayBuffer()),
    );
    this.lossyModels = lossyModels;
  }
  lossyFor(model: string): string | undefined {
    return this.lossyModels.get(model);
  }
  /** `verifiedLossy`: lossy bytes already loaded by the caller. */
  async load(
    reference: SceneAssetSource,
    verifiedLossy?: ArrayBuffer,
    materialIndices: readonly number[] = [],
    extractionPin?: NativeExtractionPin,
  ): Promise<THREE.Group> {
    if (
      extractionPin &&
      (verifiedLossy ||
        this.lossyModels.has(reference.model) ||
        reference.model_scene ||
        reference.resources.length ||
        !reference.model.endsWith(".glb"))
    )
      throw new Error("Native extraction requires the complete pinned embedded GLB");
    const applies = lossyApplies(reference.model);
    if (verifiedLossy && !applies)
      throw new Error(`Lossy model cannot replace a glTF JSON model: ${reference.model}`);
    const lossy = applies ? this.lossyModels.get(reference.model) : undefined;
    const lossyBytes =
      verifiedLossy ??
      (lossy ? await readLossyModel((path) => read(this.root, path), lossy) : null);
    if (materialIndices.length && lossyBytes)
      throw new Error("Phase materials require the pinned model");
    let bytes =
      lossyBytes ?? (await checked(await read(this.root, reference.model), reference.model_sha256));
    // Lossy models embed everything, so the published model's shared resources are not fetched.
    const resources = lossyBytes ? [] : reference.resources;
    if (reference.model.endsWith(".gltf"))
      bytes = new TextEncoder().encode(
        JSON.stringify(
          resolveGltfResources(
            reference.model,
            selectGltfScene(JSON.parse(new TextDecoder().decode(bytes)), reference.model_scene),
          ),
        ),
      ).buffer;
    else
      bytes = selectGlbScene(
        bytes,
        reference.model_scene,
        resources.length ? reference.model : undefined,
      );
    const urls = new Map<string, string>();
    await Promise.all(
      resources.map(async (resource) => {
        const key = `${resource.path}:${resource.sha256}`;
        let pending = this.resources.get(key);
        if (!pending) {
          pending = (async () => {
            const file = await read(this.root, resource.path);
            const url = URL.createObjectURL(
              new Blob([await checked(file, resource.sha256)], { type: file.type }),
            );
            this.urls.push(url);
            return url;
          })();
          this.resources.set(key, pending);
        }
        urls.set(resource.path, await pending);
      }),
    );
    const manager = new THREE.LoadingManager();
    manager.setURLModifier((url) => {
      const mapped = urls.get(url);
      if (mapped) return mapped;
      // Embedded GLB image URLs are allocated by GLTFLoader itself.
      if (url.startsWith("blob:")) return url;
      throw new Error(`Scene asset requested an unpinned resource: ${url}`);
    });
    const loader = createGltfLoader(manager);
    loader.register((parser) => {
      // GLTFLoader creates geometry-specific material variants in a per-parser
      // cache. Share those variants too, so split meshes retain depth sorting.
      const assign = parser.assignFinalMaterial.bind(parser);
      parser.assignFinalMaterial = (mesh) => {
        const value = mesh;
        const material = value.material as THREE.Material;
        const attributes = value.geometry.attributes;
        const key = [
          value.type,
          material.uuid,
          !!attributes.tangent,
          !!attributes.color,
          !!attributes.normal,
        ].join(":");
        const shared = this.finalMaterials.get(key);
        if (shared) value.material = shared;
        else {
          assign(mesh);
          this.finalMaterials.set(key, value.material as THREE.Material);
        }
      };
      const textureKey = (index: number) => {
        const texture = parser.json.textures[index];
        return JSON.stringify({
          scope: parser.json.images[texture.source]?.uri ? undefined : reference.model_sha256,
          ...texture,
          source: parser.json.images[texture.source],
          sampler: parser.json.samplers?.[texture.sampler],
        });
      };
      return {
        name: "RLE_shared_asset_resources",
        loadTexture: (index: number) => {
          const key = textureKey(index);
          let result = this.textures.get(key);
          if (!result) {
            result = parser.loadTexture(index);
            this.textures.set(key, result);
          }
          return result;
        },
        loadMaterial: (index: number) => {
          const value = structuredClone(parser.json.materials[index]);
          for (const parent of [value, value.pbrMetallicRoughness ?? {}])
            for (const [key, texture] of Object.entries(parent)) {
              if (
                key.endsWith("Texture") &&
                texture &&
                typeof texture === "object" &&
                "index" in texture
              )
                texture.index = textureKey(texture.index as number);
            }
          const key = JSON.stringify(value);
          let result = this.materials.get(key);
          if (!result) {
            result = parser.loadMaterial(index);
            this.materials.set(key, result);
          }
          return result;
        },
      };
    });
    const result = await loader.parseAsync(bytes, "");
    if (materialIndices.length) {
      const materials = new Map<number, LoadedPhaseMaterial>();
      try {
        for (const index of new Set(materialIndices)) {
          const definition = result.parser.json.materials?.[index];
          const texture = definition?.pbrMetallicRoughness?.baseColorTexture;
          if (!definition || !texture) throw new Error("Missing pinned phase material texture");
          const material = (await result.parser.getDependency("material", index)) as THREE.Material;
          materials.set(index, {
            material,
            name: definition.name,
            texture_index: texture.index,
            tex_coord: texture.extensions?.KHR_texture_transform?.texCoord ?? texture.texCoord ?? 0,
            alpha_mode: definition.alphaMode ?? "OPAQUE",
          });
        }
      } catch (error) {
        retainObjectMaterials(
          result.scene,
          [...materials.values()].map((m) => m.material),
        );
        disposeObjectResources([result.scene]);
        throw error;
      }
      retainPhaseMaterials(result.scene, materials);
      retainObjectMaterials(
        result.scene,
        [...materials.values()].map((m) => m.material),
      );
    }
    if (extractionPin) {
      this.extraction.retain({
        object: result.scene,
        modelBytes: new Uint8Array(bytes),
        parserJson: result.parser.json,
        associations: new Map(
          [...result.parser.associations].flatMap(([node, association]) =>
            node instanceof THREE.Object3D ? [[node, association] as const] : [],
          ),
        ),
      });
      this.extractionPins.set(result.scene, { ...extractionPin });
    }
    retainSceneAnimations(result.scene, result.animations, result.scenes);
    result.scene.traverse((node) => {
      const index = result.parser?.associations.get(node)?.nodes;
      const name = index === undefined ? undefined : result.parser.json.nodes[index]?.name;
      if (typeof name === "string") node.name = name;
    });
    return result.scene;
  }
  /** Bind only after the complete placed content exists, before registering its effect. */
  bindNativeExtraction(
    source: THREE.Object3D,
    placed: THREE.Object3D,
    binding: MissionStateContract["targets"][number],
    isCurrent: () => boolean,
    displayRoot: THREE.Object3D = placed,
  ) {
    const pin = this.extractionPins.get(source);
    const requested = binding.extraction_context;
    if (!requested && !pin) return Promise.resolve(undefined);
    if (!requested || !pin || requested.path !== pin.path || requested.sha256 !== pin.sha256)
      throw new Error("Native extraction requested context was not retained");
    return this.extraction.bind(source, placed, binding, pin, isCurrent, displayRoot);
  }
  dispose() {
    this.extraction.dispose();
    this.extractionPins = new WeakMap();
    for (const url of this.urls) URL.revokeObjectURL(url);
    this.urls = [];
    this.resources.clear();
    this.textures.clear();
    this.materials.clear();
    this.finalMaterials.clear();
  }
}
