import * as THREE from "three";
import {
  documentProvenance,
  expandStoredMap,
  parseStoredMap,
  parseLevel3D,
  snapFloatingParts,
} from "@rle/shared";
import { readJson, subdir } from "./fs.ts";
import { loadProtoLevel, type DatadirIndex } from "./datadir.ts";
import {
  listLossyModels,
  prepareProjectionAsset,
  readPinnedAssetDescriptors,
} from "./projection-library.ts";
import { MapTextureCompressor } from "./texture-compression.ts";
import { SceneAssetLoader } from "./scene-assets.ts";
import { disposeObjectResources } from "./resources.ts";

/** Load a complete JSON manifest. Publication happens only after every pinned asset validates. */
export async function prepareMapCandidate(
  name: string,
  library: FileSystemDirectoryHandle,
  idx: DatadirIndex | null,
  onProgress?: (completed: number, total: number, phase: string) => void,
  documentMap = name,
  importedDocument?: unknown,
  signal?: AbortSignal,
) {
  const asset = new THREE.Group();
  const loader = new SceneAssetLoader(library, await listLossyModels(library));
  let compressor: MapTextureCompressor | undefined;
  try {
    signal?.throwIfAborted();
    const directory = await subdir(library, ["scenes"]);
    if (!directory) throw new Error("scenes/ missing");
    const saved = importedDocument ?? (await readJson(directory, `${name}.rhlos-map.json`));
    const expanded = expandStoredMap(saved);
    const descriptors = await readPinnedAssetDescriptors(
      library,
      (expanded.assetSources as import("@rle/shared").ExternalAssetSource[] | undefined) ?? [],
      (expanded.sceneAssets as import("@rle/shared").SceneAssetSource[] | undefined) ?? [],
    );
    const document = parseStoredMap(saved, descriptors);
    if (document.map.toLowerCase() !== documentMap.toLowerCase())
      throw new Error(`level3d.map: expected source ${documentMap}, got ${document.map}`);
    asset.userData = structuredClone(document.sceneMetadata ?? {});
    const level = idx && document.sourceMap ? await loadProtoLevel(idx, document.sourceMap) : null;
    const sources = new Map<string, THREE.Object3D>();
    let ground: THREE.Object3D | null = null;
    const total = document.sceneAssets.length + (document.assetSources?.length ?? 0);
    let completed = 0;
    let encoding = 0;
    const report = () =>
      onProgress?.(
        completed,
        total,
        encoding ? `Loading assets — encoding textures (${encoding} active)` : "Loading assets",
      );
    compressor = new MapTextureCompressor((active) => {
      encoding = active;
      report();
    }, signal);
    report();
    const addSource = (key: string, node: THREE.Object3D) => {
      if (sources.has(key)) throw new Error(`Duplicate scene source node ${key}`);
      sources.set(key, node);
    };
    // Decode in manifest order; shared material variants retain common resources.
    for (const reference of document.sceneAssets) {
      signal?.throwIfAborted();
      const loaded = await loader.load(reference);
      asset.add(loaded);
      await compressor.compress(loaded);
      const root = loaded.children.find((node) => node.name === "map") ?? loaded;
      if (reference.role === "ground") {
        if (root.children.length !== 1 || root.children[0]!.name !== "ground")
          throw new Error(`Ground asset must contain exactly one ground node: ${reference.id}`);
        ground = root.children[0]!;
      } else {
        for (const group of root.children)
          for (const node of group.children) addSource(node.name, node);
      }
      completed++;
      report();
    }
    const references = document.assetSources ?? [];
    let next = 0,
      failure: unknown;
    let failed = false;
    const prepared = new Array<Awaited<ReturnType<typeof prepareProjectionAsset>>>(
      references.length,
    );
    const worker = async () => {
      while (!failed && next < references.length) {
        const index = next++;
        try {
          signal?.throwIfAborted();
          const reference = references[index]!;
          const lossy_model = loader.lossyFor(reference.model);
          const result = await prepareProjectionAsset(
            library,
            { ...reference, lossy_model },
            document.map,
            reference,
            loader,
            descriptors.get(reference.id),
          );
          asset.add(result.asset);
          await compressor!.compress(result.asset);
          prepared[index] = result;
          completed++;
          report();
        } catch (error) {
          failed = true;
          failure = error;
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(8, references.length) }, worker));
    if (failed) throw failure;
    for (const result of prepared) {
      asset.add(result.asset);
      for (const [key, node] of result.sources) addSource(key, node);
    }
    for (const part of document.objects) {
      if (part.node.startsWith("asset:")) continue;
      const node = sources.get(part.node);
      if (!node) throw new Error(`Missing scene source node ${part.node}`);
      if (
        node.userData.source_obstacle !== undefined &&
        node.userData.source_obstacle !== part.source.obstacle
      )
        throw new Error(`Source obstacle mismatch: ${part.node}`);
      if (part.source.components && !node.userData.obstacle_local_game)
        throw new Error(`Missing component footprint: ${part.node}`);
      if (
        part.kind === "mission" &&
        node.userData.mission_patch_profile !== part.source.mission_profile
      )
        throw new Error(`Mission source profile mismatch: ${part.node}`);
      if ((part.kind === "scenery") !== (node.userData.scenery === true))
        throw new Error(`Scenery source mismatch: ${part.node}`);
    }
    parseLevel3D(document, {
      map: documentMap,
      level: level ?? undefined,
      nodes: new Set(sources.keys()),
      sourceSha256: (await documentProvenance(level)).source_sha256,
    });
    const suspects = new Map<number, { delta: number; support: number }>();
    if (level) {
      const terraces = new Set(
        document.objects
          .filter((part) => part.kind === "terrace")
          .flatMap((part) => (part.source.obstacle === undefined ? [] : [part.source.obstacle])),
      );
      for (const item of snapFloatingParts(level.sight_obstacles, terraces, { includeOpaque: true })
        .snapped)
        suspects.set(item.index, { delta: item.delta, support: item.support });
    }
    onProgress?.(total, total, "Finalizing map");
    return { name, document, directory, level, sources, ground, suspects, asset, saved: true };
  } catch (error) {
    disposeObjectResources([asset]);
    throw error;
  } finally {
    compressor?.dispose();
    loader.dispose();
  }
}
